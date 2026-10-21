//! Guards for the IME fixes this branch carries on top of winit 0.30.13.
//!
//! Most of these fixes can only be exercised with a real input method on the platform, so each
//! test checks that the fix is still in the source and fails if a rebase drops it. When a winit
//! release carries an equivalent fix, drop the patch together with its test.

use std::path::Path;

fn source(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform_impl").join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

/// The text after `start`, up to the first `end`.
fn section<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let (_, rest) = text.split_once(start).unwrap_or_else(|| panic!("`{start}` not found"));
    rest.split_once(end).map_or(rest, |(body, _)| body)
}

#[test]
fn wayland_text_input_lives_until_the_seat_goes() {
    // winit 0.30.13 destroys a seat's text input whenever the seat loses any capability (a touch
    // screen or tablet unplugged, a remote-desktop session ending) and recreates it only when a
    // capability is added, so IME stops working until the app restarts. The text input belongs to
    // the seat instead (rust-windowing/winit#4747).
    let seat = source("linux/wayland/seat/mod.rs");
    let remove_capability = section(&seat, "fn remove_capability(", "\n    }\n");
    let remove_seat = section(&seat, "fn remove_seat(", "\n    }\n");
    assert!(
        !remove_capability.contains("text_input.destroy()"),
        "losing a capability destroys the seat's text input"
    );
    assert!(
        remove_seat.contains("text_input.destroy()"),
        "removing the seat doesn't destroy its text input"
    );
}

#[test]
fn windows_ime_cursor_is_read_only_when_reported() {
    // winit 0.30.13 reads `GCS_CURSORPOS` for every composition without a target clause, whether
    // or not `WM_IME_COMPOSITION` says the IME set it. The Microsoft Korean IME never sets it, so
    // the cursor read as 0, before the syllable being composed. Without the flag the cursor is at
    // the end of the composition (rust-windowing/winit#4746).
    let ime = source("windows/ime.rs");
    assert!(
        ime.contains("if composition_flags & GCS_CURSORPOS != 0 {"),
        "GCS_CURSORPOS is read unconditionally"
    );
    let event_loop = source("windows/event_loop.rs");
    assert!(
        event_loop.contains("get_composing_text_and_cursor(lparam as u32)"),
        "WM_IME_COMPOSITION doesn't pass its flags on"
    );
}

#[test]
fn x11_replaced_input_context_is_focused() {
    // Toolkits allow IME when a text field gains focus and disallow it when the field loses it
    // (egui does). winit 0.30.13 replaces the window's input context each time and leaves focusing
    // the new one to the next focus event, which never comes while the window keeps the focus, so
    // the input method doesn't follow the field. The replacement is focused once the queued
    // requests are handled (rust-windowing/winit#4727).
    let ime = source("linux/x11/ime/mod.rs");
    assert!(
        ime.contains("allowed: bool,\n    ) -> Result<bool, ImeContextCreationError> {"),
        "set_ime_allowed doesn't say whether it replaced the context"
    );
    let events = source("linux/x11/event_processor.rs");
    assert!(
        events.contains("Ok(true) => ime_focus = Some(window_id),"),
        "a replaced input context isn't remembered for focusing"
    );
    assert!(
        events.contains("let _ = ime.get_mut().focus(window_id);"),
        "a replaced input context isn't focused"
    );
}

/// A method in `WinitView`'s `define_class!` block.
fn view_method<'a>(view: &'a str, signature: &str) -> &'a str {
    section(view, signature, "\n        }\n")
}

#[test]
fn macos_composition_commits_after_marked_text_is_cleared() {
    // winit 0.30.13 treats `insertText` as an IME commit only while marked text is left. Input
    // methods that end a composition with `unmarkText` first lost the committed text, and Apple
    // Korean, which clears its marked text before inserting the key typed after a syllable (`한`
    // then `5`), lost that key. `insertText` commits when a composition was started
    // (rust-windowing/winit#4650, the 0.30 backport of #4651), and a commit outside a key press
    // resets the state itself, since no key follows to do it (#4748).
    let view = source("macos/view.rs");
    let set_marked_text = view_method(&view, "fn set_marked_text(");
    assert!(
        set_marked_text.contains("self.ivars().pending_commit.set(true);"),
        "setMarkedText doesn't start a pending commit"
    );
    let insert_text = view_method(&view, "fn insert_text(");
    assert!(
        insert_text.contains("if pending_commit && ime_enabled && !is_control {"),
        "insertText commits only while marked text is left"
    );
    assert!(
        insert_text.contains("if self.ivars().in_key_down.get() {")
            && insert_text.contains("self.ivars().ime_state.set(ImeState::Ground);"),
        "a commit outside a key press swallows the next key"
    );
    let attributes = view_method(&view, "fn valid_attributes_for_marked_text(");
    assert!(
        attributes.contains("NSString::from_str(\"NSUnderline\")"),
        "validAttributesForMarkedText lost the underline attribute"
    );
}

#[test]
fn macos_disallowing_ime_discards_the_composition() {
    // Toolkits disallow IME when a text field loses focus. winit 0.30.13 then forgets the marked
    // text but doesn't tell the input method, which continues the composition once IME is allowed
    // again: the abandoned syllable comes back in the next field (`하`, then `가` in the next
    // field, became `학`). The input method is told to discard it (rust-windowing/winit#4745).
    let view = source("macos/view.rs");
    let set_ime_allowed = section(&view, "pub(super) fn set_ime_allowed(", "\n    }\n");
    assert!(
        set_ime_allowed.contains("input_context.discardMarkedText();"),
        "disallowing IME leaves the input method's composition"
    );
}

#[test]
fn macos_text_inserted_outside_a_key_press_is_committed() {
    // The emoji picker inserts its text with `insertText` outside any key press and without a
    // composition. winit 0.30.13 reports `insertText` only as part of a key press or a
    // composition, so the emoji was dropped. Such text is committed (rust-windowing/winit#4749,
    // the 0.30 backport of #4748).
    let view = source("macos/view.rs");
    let insert_text = view_method(&view, "fn insert_text(");
    assert!(
        insert_text.contains("match plain_insert(&string, key_characters.as_deref()) {")
            && insert_text.contains("PlainInsert::Commit if self.ivars().ime_allowed.get() => {"),
        "text inserted outside a key press is dropped"
    );
}

#[test]
fn macos_text_typed_without_a_composition_is_committed() {
    // Chinese input methods type full-width punctuation (`，` for `,`) and Japanese ones an
    // ideographic space for Space with `insertText` and no composition. winit 0.30.13 sends the
    // key instead, with the key's own text. Text other than the key's is committed and the key's
    // text isn't sent (`plain_insert`).
    let view = source("macos/view.rs");
    let key_down = view_method(&view, "fn key_down(");
    assert!(
        key_down.contains("self.ivars().key_characters.replace(Some(characters));"),
        "keyDown doesn't tell insertText which characters the key types"
    );
}

#[test]
fn macos_input_methods_see_a_caret_and_replace_text() {
    // winit 0.30.13 reports no selection, so dictation inserts nothing (rust-windowing/winit#4666),
    // and ignores replacement ranges, so a press-and-hold accent is typed after the letter instead
    // of replacing it (#3617). The view reports a caret after the text typed since it last moved
    // (`ImeDocument`) and has the app delete replaced text with the commit that replaces it.
    let view = source("macos/view.rs");
    let selected_range = view_method(&view, "fn selected_range(&self) -> NSRange {");
    assert!(selected_range.contains("ime_document"), "selectedRange reports no caret");
    // With a caret, AppKit's press-and-hold takes the repeats of a held letter; they aren't typed.
    let key_down = view_method(&view, "fn key_down(");
    assert!(
        key_down.contains("&& !held_for_accents {"),
        "a held letter repeats under the accent menu"
    );
    for method in ["fn set_marked_text(", "fn insert_text("] {
        assert!(
            view_method(&view, method).contains("delete = self.replaced(replacement_range);"),
            "{method} ignores its replacement range"
        );
    }
}

#[test]
fn macos_replaced_text_is_deleted_by_byte_count() {
    // Toolkits delete a whole grapheme cluster per Backspace, so one Backspace per replaced
    // character deleted text before the replaced range after a combining accent, a flag or a ZWJ
    // emoji. The deletion goes with a commit (`WindowExtMacOS::take_ime_delete_surrounding`), as
    // on Wayland.
    let view = source("macos/view.rs");
    assert!(!view.contains("NamedKey::Backspace"), "replaced text is deleted with Backspace");
    let queue_commit = section(&view, "fn queue_commit(", "\n    }\n");
    assert!(
        queue_commit.contains("view.ivars().ime_delete_surrounding.set(Some(delete));"),
        "the app can't take the deletion while it handles the commit"
    );
    let set_marked_text = view_method(&view, "fn set_marked_text(");
    assert!(
        set_marked_text.contains("self.queue_commit(String::new(), delete);"),
        "a deletion before a preedit isn't delivered"
    );
}

#[test]
fn macos_input_methods_can_call_a_view_without_a_window() {
    // An input method can ask for the caret rectangle or send text while the window is torn down.
    // winit 0.30.13 then panics on the missing window inside an Objective-C callback, which aborts
    // (rust-windowing/winit#4333, fixed on master by d6f7a28).
    let view = source("macos/view.rs");
    let first_rect = view_method(&view, "fn first_rect_for_character_range(");
    assert!(
        first_rect.contains("let Some(window) = (**self).window() else {"),
        "firstRectForCharacterRange needs a window"
    );
    let queue_event = section(&view, "fn queue_event(", "\n    }\n");
    assert!(
        queue_event.contains("let Some(window) = self.ivars()._ns_window.load() else {"),
        "queuing an event needs a window"
    );
    assert!(!view.contains(".expect(\"input context\")"), "a missing input context panics");
}

#[test]
fn macos_key_committed_through_ime_is_sent_once() {
    // Apple Korean answers Space with no syllable in progress with `setMarkedText(" ")`,
    // `insertText(" ")` and `setMarkedText("")`. The last call resets winit 0.30.13's IME state,
    // so the key arrived both as `Ime::Commit(" ")` and as `KeyboardInput` with text " ": two
    // spaces. A key whose text was committed isn't sent as `KeyboardInput` unless AppKit forwards
    // it as a command (Enter); the goal of rust-windowing/winit#4478, whose changes don't reach
    // this call order.
    let view = source("macos/view.rs");
    let insert_text = view_method(&view, "fn insert_text(");
    assert!(
        insert_text.contains("self.ivars().committed_in_key_down.set(true);"),
        "a commit in keyDown isn't recorded"
    );
    let key_down = view_method(&view, "fn key_down(");
    assert!(
        key_down.contains(
            "let had_ime_input = self.ivars().committed_in_key_down.replace(false) || \
             had_ime_input;"
        ),
        "a key committed through IME is sent again as KeyboardInput"
    );
}

#[test]
fn macos_reset_ime_discards_the_composition() {
    // `Window::reset_ime` ends a composition when the caret moves to other text with IME still
    // allowed. On macOS it has the input method discard its marked text, so the abandoned text
    // isn't continued or committed in the next field.
    let view = source("macos/view.rs");
    let reset_ime = section(&view, "pub(super) fn reset_ime(", "\n    }\n");
    assert!(
        reset_ime.contains("input_context.discardMarkedText();"),
        "reset_ime leaves the input method's composition"
    );
    let delegate = source("macos/window_delegate.rs");
    assert!(delegate.contains("self.view().reset_ime();"), "reset_ime isn't passed to the view");
}

// `ImeStartup` and `ImeDocument` are plain Rust, so their tests run on every platform.
#[allow(dead_code)]
#[path = "../src/platform_impl/macos/ime_startup.rs"]
mod ime_startup;

#[allow(dead_code)]
#[path = "../src/platform_impl/macos/ime_document.rs"]
mod ime_document;

/// `winit::platform::macos::ImeDeleteSurrounding`, which only macOS builds have.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ImeDeleteSurrounding {
    pub before_bytes: usize,
    pub after_bytes: usize,
}

#[test]
fn macos_first_apple_korean_key_is_retried_once() {
    // On the first key in a freshly started app, Apple Korean can answer with `insertText` and the
    // raw compatibility jamo instead of starting a composition, so g k s typed ㅎㅏㄴ instead of
    // 한. That one key is interpreted again before it is sent to the app, under the conditions
    // `ImeStartup` checks (rust-windowing/winit#4744).
    let view = source("macos/view.rs");
    let key_down = view_method(&view, "fn key_down(");
    assert!(
        key_down.contains(
            "let retry = self.ivars().ime_startup.borrow_mut().finish(serial, same_context);"
        ),
        "keyDown doesn't retry the first key"
    );
    let insert_text = view_method(&view, "fn insert_text(");
    assert!(
        insert_text.contains("self.ivars().ime_startup.borrow_mut().insert(&string);"),
        "insertText doesn't report to the retry policy"
    );
}
