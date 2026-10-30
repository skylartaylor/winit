//! Window startup notification to handle window raising.
//!
//! The [`ActivationToken`] is essential to ensure that your newly
//! created window will obtain the focus, otherwise the user could
//! be requered to click on the window.
//!
//! Such token is usually delivered via the environment variable and
//! could be read from it with the [`EventLoopExtStartupNotify::read_token_from_env`].
//!
//! Such token must also be reset after reading it from your environment with
//! [`reset_activation_token_env`] otherwise child processes could inherit it.
//!
//! When starting a new child process with a newly obtained [`ActivationToken`] from
//! [`WindowExtStartupNotify::request_activation_token`] the [`set_activation_token_env`]
//! must be used to propagate it to the child
//!
//! To ensure the delivery of such token by other processes to you, the user should
//! set `StartupNotify=true` inside the `.desktop` file of their application.
//!
//! The specification could be found [`here`].
//!
//! [`here`]: https://specifications.freedesktop.org/startup-notification-spec/startup-notification-latest.txt

use std::env;

use crate::error::NotSupportedError;
use crate::event_loop::{ActiveEventLoop, AsyncRequestSerial};
use crate::window::{ActivationToken, Window, WindowAttributes};

/// The variable which is used mostly on X11.
const X11_VAR: &str = "DESKTOP_STARTUP_ID";

/// The variable which is used mostly on Wayland.
const WAYLAND_VAR: &str = "XDG_ACTIVATION_TOKEN";

pub trait EventLoopExtStartupNotify {
    /// Read the token from the environment.
    ///
    /// It's recommended **to unset** this environment variable for child processes.
    fn read_token_from_env(&self) -> Option<ActivationToken>;
}

pub trait WindowExtStartupNotify {
    /// Request a new activation token.
    ///
    /// The token will be delivered inside
    fn request_activation_token(&self) -> Result<AsyncRequestSerial, NotSupportedError>;

    /// Ask the windowing system to activate this existing window with `token`.
    ///
    /// [`WindowAttributesExtStartupNotify::with_activation_token`] applies a token only while a
    /// window is created. Use this method for a window that is already mapped. A typical case is
    /// an application that gets a token from another process over IPC, because the user asked
    /// that process to hand focus over.
    ///
    /// The token stays in memory. This method does not read or write the process environment.
    ///
    /// # Platform-specific
    ///
    /// - **Wayland:** Sends `xdg_activation_v1.activate(token, surface)` for this window. The
    ///   compositor decides what happens. A token without a valid input serial, or one that is too
    ///   old, may only mark the window as needing attention. The call gives no result, so watch for
    ///   [`WindowEvent::Focused`] to see whether it worked. Returns [`NotSupportedError`] when the
    ///   compositor has no `xdg_activation_v1` global.
    /// - **X11:** Sends a `_NET_ACTIVE_WINDOW` client message to the root window with source
    ///   indication 1 (application). The timestamp comes from the `_TIME` suffix of the token (the
    ///   form [`request_activation_token`] produces). A token without that suffix uses
    ///   `CurrentTime`. The window manager decides, so focus stealing prevention can still refuse.
    ///
    /// This is not [`Window::focus_window`]. That method takes no token and does nothing on
    /// Wayland.
    ///
    /// An invalid token is not an error. The windowing system ignores it.
    ///
    /// [`request_activation_token`]: Self::request_activation_token
    /// [`WindowEvent::Focused`]: crate::event::WindowEvent::Focused
    fn activate_with_token(&self, token: ActivationToken) -> Result<(), NotSupportedError>;
}

pub trait WindowAttributesExtStartupNotify {
    /// Use this [`ActivationToken`] during window creation.
    ///
    /// Not using such a token upon a window could make your window not gaining
    /// focus until the user clicks on the window.
    fn with_activation_token(self, token: ActivationToken) -> Self;
}

impl EventLoopExtStartupNotify for ActiveEventLoop {
    fn read_token_from_env(&self) -> Option<ActivationToken> {
        match self.p {
            #[cfg(wayland_platform)]
            crate::platform_impl::ActiveEventLoop::Wayland(_) => env::var(WAYLAND_VAR),
            #[cfg(x11_platform)]
            crate::platform_impl::ActiveEventLoop::X(_) => env::var(X11_VAR),
        }
        .ok()
        .map(ActivationToken::from_raw)
    }
}

impl WindowExtStartupNotify for Window {
    fn request_activation_token(&self) -> Result<AsyncRequestSerial, NotSupportedError> {
        self.window.request_activation_token()
    }

    fn activate_with_token(&self, token: ActivationToken) -> Result<(), NotSupportedError> {
        self.window.activate_with_token(token)
    }
}

impl WindowAttributesExtStartupNotify for WindowAttributes {
    fn with_activation_token(mut self, token: ActivationToken) -> Self {
        self.platform_specific.activation_token = Some(token);
        self
    }
}

/// Remove the activation environment variables from the current process.
///
/// This is wise to do before running child processes,
/// which may not to support the activation token.
pub fn reset_activation_token_env() {
    env::remove_var(X11_VAR);
    env::remove_var(WAYLAND_VAR);
}

/// Set environment variables responsible for activation token.
///
/// This could be used before running daemon processes.
pub fn set_activation_token_env(token: ActivationToken) {
    env::set_var(X11_VAR, &token.token);
    env::set_var(WAYLAND_VAR, token.token);
}
