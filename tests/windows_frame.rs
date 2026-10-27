//! Guards for W32, the borderless frame of undecorated Windows windows.
//!
//! The maximize geometry is plain Rust, so its tests (in `frame_math.rs`) run here on every
//! platform. The rest checks that the patch is still wired into the window styles and messages,
//! and fails if a rebase drops it.

use std::path::Path;

#[path = "../src/platform_impl/windows/frame_math.rs"]
mod frame_math;

fn source(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/platform_impl/windows").join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

/// The text after `start`, up to the first `end`.
fn section<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let (_, rest) = text.split_once(start).unwrap_or_else(|| panic!("`{start}` not found"));
    rest.split_once(end).map_or(rest, |(body, _)| body)
}

#[test]
fn windows_undecorated_windows_have_no_caption() {
    // With `WS_CAPTION`, some compositors offset an undecorated window's surface by the frame
    // `WM_NCCALCSIZE` hid (storytold/photocraft#2246). Every undecorated window drops it, not
    // only child windows.
    let state = source("window_state.rs");
    let styles = section(&state, "pub fn to_window_styles(self)", "\n    }\n");
    let (before_child, _) =
        styles.split_once("if self.contains(WindowFlags::CHILD)").expect("child branch");
    let undecorated = section(
        before_child,
        "if !self.contains(WindowFlags::MARKER_DECORATIONS) {",
        "\n        }\n",
    );
    assert!(
        undecorated.contains("style &= !(WS_CAPTION | WS_BORDER);"),
        "undecorated top-level windows keep WS_CAPTION"
    );
    // Windows adds `WS_CAPTION` to a new overlapped window; it is taken off again on creation.
    let window = source("window.rs");
    let on_create = section(&window, "pub unsafe fn on_create(&mut self)", "\n    }\n");
    assert!(on_create.contains("SetWindowLongW(win.hwnd(), GWL_STYLE, style as i32);"));
}

#[test]
fn windows_undecorated_maximize_stays_in_the_work_area() {
    // Without a caption Windows maximizes over the taskbar (storytold/photocraft#3144, #4845).
    let events = source("event_loop.rs");
    let min_max = section(&events, "WM_GETMINMAXINFO => {", "\n        },\n");
    assert!(min_max.contains("frame::set_max_info(window,"), "MINMAXINFO not set");
    let changing = section(&events, "WM_WINDOWPOSCHANGING => {", "\n        },\n");
    assert!(changing.contains("frame::keep_maximized_rect(window,"), "maximize rect not kept");
    let nc_calc_size = section(&events, "WM_NCCALCSIZE => {", "\n        },\n");
    assert!(
        nc_calc_size.contains("frame::maximized_client_rect(params.rgrc[0], work)"),
        "maximized client not clipped to the window"
    );
}

#[test]
fn windows_undecorated_windows_get_rounded_corners() {
    let window = source("window.rs");
    let on_create = section(&window, "pub unsafe fn on_create(&mut self)", "\n    }\n");
    assert!(on_create.contains("apply_default_corner_preference(win.hwnd(), &win.window_state);"));
    let set_decorations = section(&window, "pub fn set_decorations(", "\n    }\n");
    assert!(set_decorations.contains("apply_default_corner_preference(window, &window_state);"));
    let state = source("window_state.rs");
    let corners = section(&state, "pub fn default_corner_preference(self)", "\n    }\n");
    assert!(corners.contains("CornerPreference::Round"));
}
