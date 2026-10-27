//! The frame of undecorated top-level windows: where Windows puts them when maximized.
//!
//! An undecorated window has no `WS_CAPTION` (see `WindowFlags::to_window_styles`), and Windows
//! maximizes a caption-less window over its whole monitor. `WM_GETMINMAXINFO`,
//! `WM_WINDOWPOSCHANGING` and `WM_NCCALCSIZE` keep it to the work area instead; the geometry is in
//! `frame_math.rs`.

use std::{iter, ptr};

use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    MonitorFromPoint, MonitorFromRect, MonitorFromWindow, HMONITOR, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetWindowRect, IsWindowVisible, MINMAXINFO, SWP_FRAMECHANGED, SWP_NOMOVE,
    SWP_NOSIZE, WINDOWPOS,
};

use super::frame_math::{self, Edge, Rect};
use super::window_state::WindowState;
use super::{monitor, util};

fn rect(rect: RECT) -> Rect {
    Rect::new(rect.left, rect.top, rect.right, rect.bottom)
}

fn win32_rect(rect: Rect) -> RECT {
    RECT { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom }
}

/// The monitor and work-area rects of `hmonitor`.
fn monitor_rects(hmonitor: HMONITOR) -> Option<(Rect, Rect)> {
    if hmonitor == 0 {
        return None;
    }
    let info = monitor::get_monitor_info(hmonitor).ok()?.monitorInfo;
    Some((rect(info.rcMonitor), rect(info.rcWork)))
}

/// The outer rect an undecorated window with maximum outer size `max_size` gets when maximized on
/// `hmonitor`, with the monitor's rect.
fn maximized_rect(hmonitor: HMONITOR, max_size: Option<(i32, i32)>) -> Option<(Rect, Rect)> {
    let (monitor, work) = monitor_rects(hmonitor)?;
    let target = frame_math::maximized_rect(monitor, work, &taskbar_edges(monitor), max_size)?;
    Some((monitor, target))
}

/// The window's maximum outer size (`max_inner_size` with its frame), if it has one.
pub(super) fn max_outer_size(window: HWND, state: &WindowState) -> Option<(i32, i32)> {
    let max_size = state.max_size?.to_physical(state.scale_factor);
    let (width, height): (u32, u32) = state.window_flags.adjust_size(window, max_size).into();
    Some((i32::try_from(width).unwrap_or(i32::MAX), i32::try_from(height).unwrap_or(i32::MAX)))
}

/// The edges of `monitor` that have a taskbar (the primary `Shell_TrayWnd` or a secondary
/// monitor's `Shell_SecondaryTrayWnd`), shown or auto-hidden. Taskbars that aren't visible at
/// all (Explorer restarting, a taskbar turned off) are skipped.
///
/// `FindWindowExW` and `GetWindowRect` send no messages, so this can't block on the shell while
/// the window procedure runs (`SHAppBarMessage` can).
fn taskbar_edges(monitor: Rect) -> Vec<Edge> {
    let mut edges = Vec::new();
    for class in ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"] {
        let class: Vec<u16> = class.encode_utf16().chain(iter::once(0)).collect();
        let mut after: HWND = 0;
        // One taskbar per monitor; the bound only guards against a misbehaving shell.
        for _ in 0..64 {
            let tray = unsafe { FindWindowExW(0, after, class.as_ptr(), ptr::null()) };
            if tray == 0 {
                break;
            }
            let mut tray_rect: RECT = unsafe { std::mem::zeroed() };
            if unsafe { IsWindowVisible(tray) } != 0
                && unsafe { GetWindowRect(tray, &mut tray_rect) } != 0
            {
                edges.extend(frame_math::taskbar_edge(rect(tray_rect), monitor));
            }
            after = tray;
        }
    }
    edges
}

/// `WM_GETMINMAXINFO` of an undecorated top-level window: a maximize fills the work area of the
/// window's monitor. Where no `ptMaxSize` lands on it, Windows' value stays and
/// [`keep_maximized_rect`] corrects the maximize.
pub(super) fn set_max_info(window: HWND, mmi: &mut MINMAXINFO, max_size: Option<(i32, i32)>) {
    let hmonitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    let Some((monitor, target)) = maximized_rect(hmonitor, max_size) else {
        return;
    };
    let primary = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let Some((primary, _)) = monitor_rects(primary) else {
        return;
    };
    let primary_size = (primary.right - primary.left, primary.bottom - primary.top);
    if let Some(info) = frame_math::max_info(target, monitor, primary_size) {
        mmi.ptMaxPosition = POINT { x: info.position.0, y: info.position.1 };
        mmi.ptMaxSize = POINT { x: info.size.0, y: info.size.1 };
    }
}

/// `WM_WINDOWPOSCHANGING` of a maximized undecorated top-level window: the window goes to its
/// maximize rect on the monitor it is moving to, whatever size Windows derived from
/// `MINMAXINFO`. A frame change counts too: a window maximized while decorated still has the
/// decorated maximize rect, reaching past the work area, when its decorations go. A change of
/// z-order alone is left alone.
pub(super) fn keep_maximized_rect(
    window: HWND,
    window_pos: &mut WINDOWPOS,
    max_size: Option<(i32, i32)>,
) {
    let keeps_rect = window_pos.flags & (SWP_NOMOVE | SWP_NOSIZE) == SWP_NOMOVE | SWP_NOSIZE;
    if (keeps_rect && window_pos.flags & SWP_FRAMECHANGED == 0) || !util::is_maximized(window) {
        return;
    }
    let Some(current) = util::WindowArea::Outer.get_rect(window).ok().map(rect) else {
        return;
    };
    let (left, top) = if window_pos.flags & SWP_NOMOVE != 0 {
        (current.left, current.top)
    } else {
        (window_pos.x, window_pos.y)
    };
    let (width, height) = if window_pos.flags & SWP_NOSIZE != 0 {
        (current.right - current.left, current.bottom - current.top)
    } else {
        (window_pos.cx, window_pos.cy)
    };
    let proposed =
        RECT { left, top, right: left.saturating_add(width), bottom: top.saturating_add(height) };
    let hmonitor = unsafe { MonitorFromRect(&proposed, MONITOR_DEFAULTTONEAREST) };
    let Some((_, target)) = maximized_rect(hmonitor, max_size) else {
        return;
    };
    if target == rect(proposed) {
        return;
    }
    window_pos.x = target.left;
    window_pos.y = target.top;
    window_pos.cx = target.right - target.left;
    window_pos.cy = target.bottom - target.top;
    window_pos.flags &= !(SWP_NOMOVE | SWP_NOSIZE);
}

/// `WM_NCCALCSIZE` of a maximized undecorated window with outer rect `window_rect` on a monitor
/// with work area `work`: the client rect.
pub(super) fn maximized_client_rect(window_rect: RECT, work: RECT) -> RECT {
    win32_rect(frame_math::maximized_client_rect(rect(window_rect), rect(work)))
}
