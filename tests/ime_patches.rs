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
fn x11_input_context_outlives_disallowing_ime() {
    // Toolkits allow IME when a text field gains focus and disallow it when the field loses it
    // (egui does). winit 0.30.13 destroys the window's input context each time and creates a new
    // one, which resets the input method's state for the window (Vietnamese Telex turns off) and
    // stays unfocused until the next focus event, which never comes while the window keeps the
    // focus. The window keeps one context: allowing IME focuses it, disallowing resets and
    // unfocuses it. This also covers rust-windowing/winit#4727.
    let ime = source("linux/x11/ime/mod.rs");
    let set_ime_allowed = section(&ime, "pub fn set_ime_allowed(", "\n    }\n");
    assert!(
        !set_ime_allowed.contains("remove_context"),
        "set_ime_allowed destroys the input context"
    );
    let context = source("linux/x11/ime/context.rs");
    assert!(
        !context.contains("none_style"),
        "a context with IME disallowed is created with another input style"
    );
    let events = source("linux/x11/event_processor.rs");
    let allow =
        section(&events, "ImeRequest::Allow(window_id, allowed) => {", "\n                },\n");
    for call in ["ime.focus(window_id)", "ime.reset(window_id)", "ime.unfocus(window_id)"] {
        assert!(allow.contains(call), "an IME allow request doesn't call `{call}`");
    }
}

#[test]
fn x11_ime_spot_waits_for_the_input_method() {
    // Setting the IME spot is a synchronous XIM request. Keys an input method forwards back while
    // the client waits for the reply are filtered again by Xlib and reach the input method as new
    // key presses; Vietnamese engines that fix words with forwarded BackSpaces garble the text
    // (rust-windowing/winit#4777, storytold/wordcraft#2932). Spots are queued and sent once no
    // event is queued and the input method has been quiet for a while.
    let events = source("linux/x11/event_processor.rs");
    let position = section(&events, "ImeRequest::Position(window_id, x, y) => {", "},");
    assert!(!position.contains("send_xim_spot"), "an IME spot is sent as soon as it's requested");
    let send = section(&events, "pub fn send_ime_spots(&mut self) {", "\n    }\n");
    assert!(send.contains("self.ime_busy_until"), "IME spots don't wait for the input method");
    assert!(send.contains("self.poll()"), "IME spots are sent while events are queued");
    let filtered = section(&events, "// Don't process event if it was filtered.", "return;");
    assert!(
        filtered.contains("self.ime_busy_until = Some("),
        "events taken by the input method don't hold IME spots back"
    );
    let event_loop = source("linux/x11/mod.rs");
    assert!(
        event_loop.contains("self.event_processor.send_ime_spots();"),
        "the event loop doesn't send IME spots held back"
    );
}

#[allow(dead_code)]
fn reset_ime_is_public(window: &winit::window::Window) {
    window.reset_ime();
}

#[test]
fn linux_reset_ime_discards_the_composition() {
    // `Window::reset_ime` resets the input method when focus moves between fields. X11 resets the
    // input context before the next key reaches the input method; Wayland disables and re-enables
    // text input in two commits, so the compositor deactivates the input method, and sends the
    // state that enabling resets again.
    let events = source("linux/x11/event_processor.rs");
    let process_event = section(&events, "pub fn process_event<", "\n    }\n");
    let (before, after) = process_event
        .split_once("self.process_xevent(xev, &mut callback);")
        .expect("process_event doesn't process the event");
    assert!(
        before.contains("self.handle_ime_requests(&mut callback);")
            && after.contains("self.handle_ime_requests(&mut callback);"),
        "IME requests aren't handled before and after each event"
    );
    let context = source("linux/x11/ime/context.rs");
    assert!(context.contains("(xconn.xlib.XmbResetIC)(self.ic)"), "X11 doesn't reset the context");

    let state = source("linux/wayland/window/state.rs");
    let reset = section(&state, "pub fn reset_ime(&self) -> bool {", "\n    }\n");
    let disable = reset.find("text_input.disable();").expect("Wayland reset doesn't disable");
    let enable = reset.find("text_input.enable();").expect("Wayland reset doesn't enable");
    assert!(
        reset[disable..enable].contains("text_input.commit();"),
        "Wayland reset doesn't commit between disabling and enabling"
    );
    for request in ["set_content_type_by_purpose", "set_cursor_rectangle"] {
        assert!(reset[enable..].contains(request), "Wayland reset doesn't restore `{request}`");
    }
}

#[test]
fn windows_reset_ime_cancels_the_composition() {
    // `CPS_CANCEL` discards the composition string; `CPS_COMPLETE` would commit it instead. The
    // IME's own end-of-composition messages clear the preedit.
    let ime = source("windows/ime.rs");
    assert!(
        ime.contains("ImmNotifyIME(self.himc, NI_COMPOSITIONSTR, CPS_CANCEL, 0)"),
        "the composition isn't cancelled"
    );
    let window = source("windows/window.rs");
    let reset = section(&window, "pub fn reset_ime(&self) {", "\n    }\n");
    assert!(reset.contains(".cancel_composition()"), "reset_ime doesn't cancel the composition");
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
        insert_text.contains(
            "} else if !self.ivars().in_key_down.get()\n                && \
             self.ivars().ime_allowed.get()"
        ),
        "text inserted outside a key press is dropped"
    );
    // #4748's `selectedRange` change for dictation is left out: reporting a selection turns on
    // press-and-hold for every held key (an accent menu instead of key repeat).
    let selected_range = view_method(&view, "fn selected_range(&self) -> NSRange {");
    assert!(!selected_range.contains("marked_text"), "selectedRange reports a selection");
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

// `ImeStartup` is plain Rust, so its policy tests run on every platform.
#[allow(dead_code)]
#[path = "../src/platform_impl/macos/ime_startup.rs"]
mod ime_startup;

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
