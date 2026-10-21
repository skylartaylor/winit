use std::ops::Deref;

use sctk::globals::GlobalData;
use sctk::reexports::client::{Connection, Proxy, QueueHandle};

use sctk::reexports::client::globals::{BindError, GlobalList};
use sctk::reexports::client::protocol::wl_surface::WlSurface;
use sctk::reexports::client::{delegate_dispatch, Dispatch};
use sctk::reexports::protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::ZwpTextInputManagerV3;
use sctk::reexports::protocols::wp::text_input::zv3::client::zwp_text_input_v3::{
    ContentHint, ContentPurpose, Event as TextInputEvent, ZwpTextInputV3,
};

use crate::event::{Ime, WindowEvent};
use crate::platform::wayland::ImeDeleteSurrounding;
use crate::platform_impl::wayland;
use crate::platform_impl::wayland::state::WinitState;
use crate::window::ImePurpose;

pub struct TextInputState {
    text_input_manager: ZwpTextInputManagerV3,
}

impl TextInputState {
    pub fn new(
        globals: &GlobalList,
        queue_handle: &QueueHandle<WinitState>,
    ) -> Result<Self, BindError> {
        let text_input_manager = globals.bind(queue_handle, 1..=1, GlobalData)?;
        Ok(Self { text_input_manager })
    }
}

impl Deref for TextInputState {
    type Target = ZwpTextInputManagerV3;

    fn deref(&self) -> &Self::Target {
        &self.text_input_manager
    }
}

impl Dispatch<ZwpTextInputManagerV3, GlobalData, WinitState> for TextInputState {
    fn event(
        _state: &mut WinitState,
        _proxy: &ZwpTextInputManagerV3,
        _event: <ZwpTextInputManagerV3 as Proxy>::Event,
        _data: &GlobalData,
        _conn: &Connection,
        _qhandle: &QueueHandle<WinitState>,
    ) {
    }
}

impl Dispatch<ZwpTextInputV3, TextInputData, WinitState> for TextInputState {
    fn event(
        state: &mut WinitState,
        text_input: &ZwpTextInputV3,
        event: <ZwpTextInputV3 as Proxy>::Event,
        data: &TextInputData,
        _conn: &Connection,
        _qhandle: &QueueHandle<WinitState>,
    ) {
        let windows = state.windows.get_mut();
        let mut text_input_data = data.inner.lock().unwrap();
        match event {
            TextInputEvent::Enter { surface } => {
                let window_id = wayland::make_wid(&surface);
                text_input_data.surface = Some(surface);

                let mut window = match windows.get(&window_id) {
                    Some(window) => window.lock().unwrap(),
                    None => return,
                };

                if window.ime_allowed() {
                    text_input.enable();
                    window.apply_ime_state(text_input);
                    text_input.commit();
                    state.events_sink.push_window_event(WindowEvent::Ime(Ime::Enabled), window_id);
                }

                window.text_input_entered(text_input);
            },
            TextInputEvent::Leave { surface } => {
                text_input_data.surface = None;

                // Always issue a disable.
                text_input.disable();
                text_input.commit();

                let window_id = wayland::make_wid(&surface);

                // XXX this check is essential, because `leave` could have a
                // reference to nil surface...
                let mut window = match windows.get(&window_id) {
                    Some(window) => window.lock().unwrap(),
                    None => return,
                };

                window.text_input_left(text_input);

                state.events_sink.push_window_event(WindowEvent::Ime(Ime::Disabled), window_id);
            },
            TextInputEvent::PreeditString { text, cursor_begin, cursor_end } => {
                let text = text.unwrap_or_default();
                let cursor_begin = usize::try_from(cursor_begin)
                    .ok()
                    .and_then(|idx| text.is_char_boundary(idx).then_some(idx));
                let cursor_end = usize::try_from(cursor_end)
                    .ok()
                    .and_then(|idx| text.is_char_boundary(idx).then_some(idx));

                text_input_data.pending_preedit = Some(Preedit { text, cursor_begin, cursor_end })
            },
            TextInputEvent::CommitString { text } => {
                text_input_data.pending_preedit = None;
                text_input_data.pending_commit = text;
            },
            TextInputEvent::Done { .. } => {
                let window_id = match text_input_data.surface.as_ref() {
                    Some(surface) => wayland::make_wid(surface),
                    None => return,
                };

                // The deletion goes out with a commit, so the application applies it after
                // removing the preedit and before inserting the text, as text-input-v3 orders
                // them.
                let delete = text_input_data.pending_delete.take();
                if delete.is_some() && text_input_data.pending_commit.is_none() {
                    text_input_data.pending_commit = Some(String::new());
                }

                // Clear preedit, unless all we'll be doing next is sending a new preedit.
                if text_input_data.pending_commit.is_some()
                    || text_input_data.pending_preedit.is_none()
                {
                    text_input_data.preedit_shown = false;
                    state.events_sink.push_window_event(
                        WindowEvent::Ime(Ime::Preedit(String::new(), None)),
                        window_id,
                    );
                }

                // Send `Commit`.
                if let Some(text) = text_input_data.pending_commit.take() {
                    state.ime_deletions.push_back(delete);
                    state
                        .events_sink
                        .push_window_event(WindowEvent::Ime(Ime::Commit(text)), window_id);
                }

                // Send preedit.
                if let Some(preedit) = text_input_data.pending_preedit.take() {
                    text_input_data.preedit_shown = true;
                    let cursor_range =
                        preedit.cursor_begin.map(|b| (b, preedit.cursor_end.unwrap_or(b)));

                    state.events_sink.push_window_event(
                        WindowEvent::Ime(Ime::Preedit(preedit.text, cursor_range)),
                        window_id,
                    );
                }
            },
            TextInputEvent::DeleteSurroundingText { before_length, after_length } => {
                text_input_data.pending_delete = Some(ImeDeleteSurrounding {
                    before_bytes: before_length as usize,
                    after_bytes: after_length as usize,
                });
            },
            _ => {},
        }
    }
}

pub trait ZwpTextInputV3Ext {
    fn set_content_type_by_purpose(&self, purpose: ImePurpose);

    /// Drops the preedit not yet sent and returns whether the window was showing one.
    fn take_preedit(&self) -> bool;
}

impl ZwpTextInputV3Ext for ZwpTextInputV3 {
    fn take_preedit(&self) -> bool {
        let Some(data) = self.data::<TextInputData>() else { return false };
        let mut data = data.inner.lock().unwrap();
        data.pending_preedit = None;
        std::mem::take(&mut data.preedit_shown)
    }

    fn set_content_type_by_purpose(&self, purpose: ImePurpose) {
        let (hint, purpose) = match purpose {
            ImePurpose::Normal => (ContentHint::None, ContentPurpose::Normal),
            ImePurpose::Password => (ContentHint::SensitiveData, ContentPurpose::Password),
            ImePurpose::Terminal => (ContentHint::None, ContentPurpose::Terminal),
        };
        self.set_content_type(hint, purpose);
    }
}

/// The Data associated with the text input.
#[derive(Default)]
pub struct TextInputData {
    inner: std::sync::Mutex<TextInputDataInner>,
}

#[derive(Default)]
pub struct TextInputDataInner {
    /// The `WlSurface` we're performing input to.
    surface: Option<WlSurface>,

    /// The commit to submit on `done`.
    pending_commit: Option<String>,

    /// The preedit to submit on `done`.
    pending_preedit: Option<Preedit>,

    /// Whether the window was last sent a preedit rather than its end.
    preedit_shown: bool,

    /// The surrounding text to delete on `done`.
    pending_delete: Option<ImeDeleteSurrounding>,
}

/// The state of the preedit.
struct Preedit {
    text: String,
    cursor_begin: Option<usize>,
    cursor_end: Option<usize>,
}

delegate_dispatch!(WinitState: [ZwpTextInputManagerV3: GlobalData] => TextInputState);
delegate_dispatch!(WinitState: [ZwpTextInputV3: TextInputData] => TextInputState);

/// The longest surrounding text text-input-v3 can carry.
const MAX_SURROUNDING_TEXT_BYTES: usize = 4000;

/// Clamps `cursor` and `anchor` to `text` and moves them back to character boundaries, then cuts
/// text longer than [`MAX_SURROUNDING_TEXT_BYTES`] to a window around the cursor and the
/// selection, or around the cursor alone when the selection doesn't fit. Returns the text and the
/// cursor and anchor offsets in it.
pub fn surrounding_text_excerpt(text: &str, cursor: usize, anchor: usize) -> (&str, usize, usize) {
    let cursor = floor_char_boundary(text, cursor);
    let anchor = floor_char_boundary(text, anchor);
    if text.len() <= MAX_SURROUNDING_TEXT_BYTES {
        return (text, cursor, anchor);
    }

    let (low, high) = (cursor.min(anchor), cursor.max(anchor));
    let (low, high) =
        if high - low <= MAX_SURROUNDING_TEXT_BYTES { (low, high) } else { (cursor, cursor) };
    // Split the spare bytes evenly on both sides, giving one side's unused share to the other.
    let spare = MAX_SURROUNDING_TEXT_BYTES - (high - low);
    let end = (low.saturating_sub(spare / 2) + MAX_SURROUNDING_TEXT_BYTES).min(text.len());
    let start = ceil_char_boundary(text, end - MAX_SURROUNDING_TEXT_BYTES);
    let end = floor_char_boundary(text, end);

    let anchor = anchor.clamp(start, end);
    (&text[start..end], cursor - start, anchor - start)
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::{surrounding_text_excerpt, MAX_SURROUNDING_TEXT_BYTES as MAX};

    #[test]
    fn short_text_is_kept_and_offsets_are_clamped() {
        assert_eq!(surrounding_text_excerpt("foobar", 3, 3), ("foobar", 3, 3));
        assert_eq!(surrounding_text_excerpt("foobar", 1, 4), ("foobar", 1, 4));
        assert_eq!(surrounding_text_excerpt("foo", 10, 99), ("foo", 3, 3));
        // `и` is two bytes; an offset inside it moves back to its start.
        assert_eq!(surrounding_text_excerpt("граница", 1, 5), ("граница", 0, 4));
    }

    #[test]
    fn long_text_is_cut_around_the_cursor() {
        let text = "a".repeat(10_000);
        let (excerpt, cursor, anchor) = surrounding_text_excerpt(&text, 5_000, 5_000);
        assert_eq!(excerpt.len(), MAX);
        assert_eq!((cursor, anchor), (MAX / 2, MAX / 2));

        // Near either end, the unused share goes to the other side.
        assert_eq!(surrounding_text_excerpt(&text, 100, 100), (&text[..MAX], 100, 100));
        let (excerpt, cursor, _) = surrounding_text_excerpt(&text, 9_900, 9_900);
        assert_eq!((excerpt.len(), cursor), (MAX, MAX - 100));
    }

    #[test]
    fn long_text_keeps_the_selection() {
        let text = "a".repeat(10_000);
        let (excerpt, cursor, anchor) = surrounding_text_excerpt(&text, 6_000, 3_000);
        assert_eq!(excerpt.len(), MAX);
        assert_eq!(cursor - anchor, 3_000);
        assert_eq!(anchor, 500);

        // A selection longer than the limit keeps the cursor; the anchor stops at the cut.
        let (excerpt, cursor, anchor) = surrounding_text_excerpt(&text, 9_000, 1_000);
        assert_eq!(excerpt.len(), MAX);
        assert_eq!((cursor, anchor), (3_000, 0));
    }

    #[test]
    fn long_text_is_cut_on_char_boundaries() {
        // Three-byte characters: 4000 isn't a multiple of 3, so the cut must move inward.
        let text = "ấ".repeat(3_000);
        for cursor in [0, 4_500, 4_501, 9_000] {
            let (excerpt, cursor_in, anchor_in) = surrounding_text_excerpt(&text, cursor, cursor);
            assert!(excerpt.len() <= MAX && excerpt.len() > MAX - 6, "{cursor}");
            assert!(excerpt.chars().all(|c| c == 'ấ'), "{cursor}: cut inside a character");
            assert_eq!(cursor_in % 3, 0, "{cursor}");
            assert_eq!(cursor_in, anchor_in);
        }
    }
}
