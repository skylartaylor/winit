//! # Wayland
//!
//! **Note:** Windows don't appear on Wayland until you draw/present to them.
//!
//! By default, Winit loads system libraries using `dlopen`. This can be
//! disabled by disabling the `"wayland-dlopen"` cargo feature.
//!
//! ## Client-side decorations
//!
//! Winit provides client-side decorations by default, but the behaviour can
//! be controlled with the following feature flags:
//!
//! * `wayland-csd-adwaita` (default).
//! * `wayland-csd-adwaita-crossfont`.
//! * `wayland-csd-adwaita-notitle`.

use std::ffi::c_void;
use std::ptr::NonNull;

use crate::event_loop::{ActiveEventLoop, EventLoop, EventLoopBuilder};
use crate::monitor::MonitorHandle;
use crate::window::{Window, WindowAttributes};

pub use crate::platform_impl::wayland::clipboard::WaylandClipboard;
pub use crate::window::Theme;

/// Additional methods on [`ActiveEventLoop`] that are specific to Wayland.
pub trait ActiveEventLoopExtWayland {
    /// True if the [`ActiveEventLoop`] uses Wayland.
    fn is_wayland(&self) -> bool;
}

impl ActiveEventLoopExtWayland for ActiveEventLoop {
    #[inline]
    fn is_wayland(&self) -> bool {
        self.p.is_wayland()
    }
}

/// Additional methods on [`EventLoop`] that are specific to Wayland.
pub trait EventLoopExtWayland {
    /// True if the [`EventLoop`] uses Wayland.
    fn is_wayland(&self) -> bool;
}

impl<T: 'static> EventLoopExtWayland for EventLoop<T> {
    #[inline]
    fn is_wayland(&self) -> bool {
        self.event_loop.is_wayland()
    }
}

/// Additional methods on [`EventLoopBuilder`] that are specific to Wayland.
pub trait EventLoopBuilderExtWayland {
    /// Force using Wayland.
    fn with_wayland(&mut self) -> &mut Self;

    /// Whether to allow the event loop to be created off of the main thread.
    ///
    /// By default, the window is only allowed to be created on the main
    /// thread, to make platform compatibility easier.
    fn with_any_thread(&mut self, any_thread: bool) -> &mut Self;
}

impl<T> EventLoopBuilderExtWayland for EventLoopBuilder<T> {
    #[inline]
    fn with_wayland(&mut self) -> &mut Self {
        self.platform_specific.forced_backend = Some(crate::platform_impl::Backend::Wayland);
        self
    }

    #[inline]
    fn with_any_thread(&mut self, any_thread: bool) -> &mut Self {
        self.platform_specific.any_thread = any_thread;
        self
    }
}

/// Native tablet touch IDs are distinct from wl_touch contact IDs.
pub const TABLET_TOUCH_TAG: u64 = 1 << 63;

/// A native tablet phase on one Wayland window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabletPhase {
    Entered,
    Moved,
    Pressed,
    Released,
    Left,
}
/// A coherent tablet-v2 frame sample. Position is in physical window pixels; angles are degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabletInput {
    /// Connection-local tool identifier, valid for the tool's lifetime.
    pub tool_id: u64,
    /// Hover, contact or proximity transition.
    pub phase: TabletPhase,
    /// Pointer position relative to this window.
    pub position: crate::dpi::PhysicalPosition<f64>,
    /// Pressure in 0..=1.
    pub pressure: f64,
    /// X/Y tilt in -90..=90 degrees.
    pub tilt: [f64; 2],
    /// Barrel rotation in 0..360 degrees.
    pub rotation: f64,
    /// The tool is the pen's eraser end.
    pub eraser: bool,
}

/// A file-transfer portal key received from an external drag source.
#[derive(Clone, Debug)]
pub struct FileTransferToken {
    /// Unique for this event loop; late acknowledgements cannot finish a newer transfer.
    pub id: u64,
    pub key: String,
    pub position: crate::dpi::LogicalPosition<f64>,
}

/// Text an input method asks to delete around the cursor, in bytes of UTF-8 text.
///
/// The counts exclude the selection and the preedit: `before_bytes` ends where the preedit or
/// selection starts and `after_bytes` starts where it ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ImeDeleteSurrounding {
    /// Bytes to delete before the cursor.
    pub before_bytes: usize,
    /// Bytes to delete after the cursor.
    pub after_bytes: usize,
}

/// Local file paths from a clipboard or drop URI list. Invalid, remote and oversized lists
/// are rejected; percent-encoded POSIX filename bytes are preserved.
pub fn local_file_paths(data: &[u8]) -> Vec<std::path::PathBuf> {
    if data.len() > 16 << 20 {
        return Vec::new();
    }
    crate::platform_impl::wayland::parse_uri_list(data)
}

/// Additional methods on [`Window`] that are specific to Wayland.
///
/// [`Window`]: crate::window::Window
pub trait WindowExtWayland {
    /// Shares this event loop's focused data device and clipboard selection.
    fn wayland_clipboard(&self) -> Option<WaylandClipboard>;
    /// Clones the owned connection for another event queue, such as a focused-app clipboard.
    /// Returns `None` for an X11 window. The clone retains the backend's lifetime.
    fn wayland_connection(&self) -> Option<wayland_client::Connection>;
    /// Returns `xdg_toplevel` of the window or [`None`] if the window is X11 window.
    fn xdg_toplevel(&self) -> Option<NonNull<c_void>>;
    /// Drain bounded tablet-v2 samples for this window. X11 returns an empty list.
    fn take_tablet_input(&self) -> Vec<TabletInput>;
    /// Whether the compositor advertised tablet-v2 when this Wayland window was created.
    fn tablet_input_available(&self) -> bool;
    /// Caps Lock reported for the focused Wayland window; unknown before keyboard focus.
    fn caps_lock_state(&self) -> Option<bool>;
    /// Drain portal drops for this window. Complete each after the portal retrieval finishes.
    fn take_file_transfer_tokens(&self) -> Vec<FileTransferToken>;
    fn finish_file_transfer(&self, id: u64);
    /// Tells the input method the text around the cursor of the focused text field, so it can
    /// base suggestions and corrections on it (text-input-v3 surrounding text).
    ///
    /// `cursor` and `anchor` are byte offsets into `text`, the other end of the selection being
    /// `anchor`; without a selection they are equal. Leave the preedit out of `text`. Offsets
    /// are clamped to the text and moved back to a character boundary, and text longer than
    /// 4000 bytes is cut around the cursor and selection. Call it again whenever the text, cursor
    /// or selection changes. Disallowing IME forgets the text. X11 ignores it.
    fn set_ime_surrounding_text(&self, text: &str, cursor: usize, anchor: usize);
    /// The surrounding text to delete before inserting the [`Ime::Commit`] being handled.
    ///
    /// Call this when handling each `Ime::Commit`, after removing the preedit and before
    /// inserting the committed text, which may be empty when an input method only deletes.
    /// Returns `None` when the commit deletes nothing, and always on X11.
    ///
    /// [`Ime::Commit`]: crate::event::Ime::Commit
    fn take_ime_delete_surrounding(&self) -> Option<ImeDeleteSurrounding>;
}

impl WindowExtWayland for Window {
    fn wayland_clipboard(&self) -> Option<WaylandClipboard> {
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => None,
            crate::platform_impl::Window::Wayland(window) => Some(window.wayland_clipboard()),
        }
    }
    fn wayland_connection(&self) -> Option<wayland_client::Connection> {
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => None,
            crate::platform_impl::Window::Wayland(window) => window.wayland_connection(),
        }
    }
    fn take_file_transfer_tokens(&self) -> Vec<FileTransferToken> {
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => Vec::new(),
            #[cfg(wayland_platform)]
            crate::platform_impl::Window::Wayland(window) => window.take_file_transfer_tokens(),
        }
    }
    fn finish_file_transfer(&self, id: u64) {
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => {},
            #[cfg(wayland_platform)]
            crate::platform_impl::Window::Wayland(window) => window.finish_file_transfer(id),
        }
    }
    fn caps_lock_state(&self) -> Option<bool> {
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => None,
            #[cfg(wayland_platform)]
            crate::platform_impl::Window::Wayland(window) => window.caps_lock_state(),
        }
    }
    fn take_tablet_input(&self) -> Vec<TabletInput> {
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => Vec::new(),
            #[cfg(wayland_platform)]
            crate::platform_impl::Window::Wayland(window) => window.take_tablet_input(),
        }
    }
    fn tablet_input_available(&self) -> bool {
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => false,
            #[cfg(wayland_platform)]
            crate::platform_impl::Window::Wayland(window) => window.tablet_input_available(),
        }
    }
    fn set_ime_surrounding_text(&self, text: &str, cursor: usize, anchor: usize) {
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => {},
            #[cfg(wayland_platform)]
            crate::platform_impl::Window::Wayland(window) => {
                window.set_ime_surrounding_text(text, cursor, anchor)
            },
        }
    }
    fn take_ime_delete_surrounding(&self) -> Option<ImeDeleteSurrounding> {
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => None,
            #[cfg(wayland_platform)]
            crate::platform_impl::Window::Wayland(window) => window.take_ime_delete_surrounding(),
        }
    }
    #[inline]
    fn xdg_toplevel(&self) -> Option<NonNull<c_void>> {
        #[allow(clippy::single_match)]
        match &self.window {
            #[cfg(x11_platform)]
            crate::platform_impl::Window::X(_) => None,
            #[cfg(wayland_platform)]
            crate::platform_impl::Window::Wayland(window) => window.xdg_toplevel(),
        }
    }
}

/// Additional methods on [`WindowAttributes`] that are specific to Wayland.
pub trait WindowAttributesExtWayland {
    /// Build window with the given name.
    ///
    /// The `general` name sets an application ID, which should match the `.desktop`
    /// file distributed with your program. The `instance` is a `no-op`.
    ///
    /// For details about application ID conventions, see the
    /// [Desktop Entry Spec](https://specifications.freedesktop.org/desktop-entry-spec/desktop-entry-spec-latest.html#desktop-file-id)
    fn with_name(self, general: impl Into<String>, instance: impl Into<String>) -> Self;
}

impl WindowAttributesExtWayland for WindowAttributes {
    #[inline]
    fn with_name(mut self, general: impl Into<String>, instance: impl Into<String>) -> Self {
        self.platform_specific.name =
            Some(crate::platform_impl::ApplicationName::new(general.into(), instance.into()));
        self
    }
}

/// Additional methods on `MonitorHandle` that are specific to Wayland.
pub trait MonitorHandleExtWayland {
    /// Returns the inner identifier of the monitor.
    fn native_id(&self) -> u32;
}

impl MonitorHandleExtWayland for MonitorHandle {
    #[inline]
    fn native_id(&self) -> u32 {
        self.inner.native_identifier()
    }
}
