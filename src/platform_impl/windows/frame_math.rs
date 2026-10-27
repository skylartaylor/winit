//! Geometry for undecorated windows: where a maximized window goes, and what Windows must be told
//! to put it there.
//!
//! Plain Rust over desktop rects, without Win32 types, so `tests/windows_frame.rs` runs these
//! functions and their tests on every platform. `frame.rs` connects them to the window messages.

/// A rect in desktop pixels; `right` and `bottom` are exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self { left, top, right, bottom }
    }

    pub fn is_empty(self) -> bool {
        self.left >= self.right || self.top >= self.bottom
    }

    fn contains(self, other: Rect) -> bool {
        other.left >= self.left
            && other.top >= self.top
            && other.right <= self.right
            && other.bottom <= self.bottom
    }

    fn intersection(self, other: Rect) -> Option<Rect> {
        let rect = Rect::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        );
        (!rect.is_empty()).then_some(rect)
    }

    fn size(self) -> (i64, i64) {
        (i64::from(self.right) - i64::from(self.left), i64::from(self.bottom) - i64::from(self.top))
    }
}

/// A monitor edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Top,
    Right,
    Bottom,
}

/// The edge of `monitor` a taskbar with rect `taskbar` sits on: the one nearest its centre, or
/// `None` when it is on another monitor. An auto-hidden taskbar is parked mostly past its edge
/// with a sliver left on screen, so its centre may lie outside the monitor.
pub fn taskbar_edge(taskbar: Rect, monitor: Rect) -> Option<Edge> {
    let (tl, tt, tr, tb) = (
        i64::from(taskbar.left),
        i64::from(taskbar.top),
        i64::from(taskbar.right),
        i64::from(taskbar.bottom),
    );
    let (ml, mt, mr, mb) = (
        i64::from(monitor.left),
        i64::from(monitor.top),
        i64::from(monitor.right),
        i64::from(monitor.bottom),
    );
    if tl >= tr || tt >= tb || ml >= mr || mt >= mb || tr <= ml || tl >= mr || tb <= mt || tt >= mb
    {
        return None;
    }
    // Doubled centre and distances, so nothing rounds.
    let (cx, cy) = (tl + tr, tt + tb);
    [
        (Edge::Left, cx - 2 * ml),
        (Edge::Top, cy - 2 * mt),
        (Edge::Right, 2 * mr - cx),
        (Edge::Bottom, 2 * mb - cy),
    ]
    .into_iter()
    .min_by_key(|&(_, distance)| distance)
    .map(|(edge, _)| edge)
}

/// The outer rect of an undecorated window maximized on `monitor`: the work area, stopping one
/// pixel short of each edge where a taskbar (`taskbars`) reserves no work area because it
/// auto-hides.
///
/// Without a caption, Windows sizes a maximize to the whole monitor, taskbar included, and the
/// shell takes a caption-less window covering its monitor for a full-screen app: it drops the
/// taskbar's backdrop (photocraft#3144) and no longer reveals an auto-hidden taskbar at its edge
/// (photocraft#4845). A window with a maximum outer size (`max_size`) keeps the top-left corner
/// and stops there, as Windows does for a decorated window. `None` when the work area is empty
/// or lies outside the monitor.
pub fn maximized_rect(
    monitor: Rect,
    work: Rect,
    taskbars: &[Edge],
    max_size: Option<(i32, i32)>,
) -> Option<Rect> {
    if monitor.is_empty() || work.is_empty() || !monitor.contains(work) {
        return None;
    }
    let mut rect = work;
    for edge in taskbars {
        match edge {
            Edge::Left if work.left == monitor.left => rect.left = monitor.left.checked_add(1)?,
            Edge::Top if work.top == monitor.top => rect.top = monitor.top.checked_add(1)?,
            Edge::Right if work.right == monitor.right => {
                rect.right = monitor.right.checked_sub(1)?
            },
            Edge::Bottom if work.bottom == monitor.bottom => {
                rect.bottom = monitor.bottom.checked_sub(1)?
            },
            _ => {},
        }
    }
    let mut rect = if rect.is_empty() { work } else { rect };
    if let Some((width, height)) = max_size {
        rect.right = rect.right.min(rect.left.saturating_add(width.max(1)));
        rect.bottom = rect.bottom.min(rect.top.saturating_add(height.max(1)));
    }
    Some(rect)
}

/// The `ptMaxPosition` and `ptMaxSize` of `MINMAXINFO`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaxInfo {
    pub position: (i32, i32),
    pub size: (i32, i32),
}

/// What `WM_GETMINMAXINFO` reports so that Windows maximizes the window to `target` on `monitor`,
/// with the primary monitor `primary` pixels in size.
///
/// Windows reads `ptMaxPosition` relative to the window's own monitor, but `ptMaxSize` as if
/// for the primary monitor: a size at least as large as the primary monitor grows by the
/// difference between the two monitors (<https://devblogs.microsoft.com/oldnewthing/20150501-00/?p=44964>).
/// Whether that comparison is made per axis or for both axes together isn't documented, so the
/// size chosen must land on `target` either way. `None` when no size does (a monitor larger than
/// the primary one, with a taskbar); `WM_WINDOWPOSCHANGING` then puts the window in place.
pub fn max_info(target: Rect, monitor: Rect, primary: (i32, i32)) -> Option<MaxInfo> {
    let (want_w, want_h) = target.size();
    let (mon_w, mon_h) = monitor.size();
    let (pri_w, pri_h) = (i64::from(primary.0), i64::from(primary.1));
    if want_w <= 0 || want_h <= 0 || pri_w <= 0 || pri_h <= 0 {
        return None;
    }
    let per_axis =
        |size: i64, mon: i64, pri: i64| if size >= pri { size + mon - pri } else { size };
    let lands = |(w, h): (i64, i64)| {
        let separate = (per_axis(w, mon_w, pri_w), per_axis(h, mon_h, pri_h));
        let together =
            if w >= pri_w && h >= pri_h { (w + mon_w - pri_w, h + mon_h - pri_h) } else { (w, h) };
        separate == (want_w, want_h) && together == (want_w, want_h)
    };
    let widths = [want_w, want_w - mon_w + pri_w];
    let heights = [want_h, want_h - mon_h + pri_h];
    let (w, h) = widths
        .into_iter()
        .flat_map(|w| heights.into_iter().map(move |h| (w, h)))
        .find(|&(w, h)| w > 0 && h > 0 && lands((w, h)))?;
    Some(MaxInfo {
        position: (
            i32::try_from(i64::from(target.left) - i64::from(monitor.left)).ok()?,
            i32::try_from(i64::from(target.top) - i64::from(monitor.top)).ok()?,
        ),
        size: (i32::try_from(w).ok()?, i32::try_from(h).ok()?),
    })
}

/// The client rect of a maximized undecorated window whose outer rect is `window`: the part of
/// it inside the monitor's work area. A maximize that still reaches past the work area (Windows'
/// default frame overhang) keeps its pixels clear of the taskbar, and one stopping short of an
/// auto-hidden taskbar has no client row outside the window.
pub fn maximized_client_rect(window: Rect, work: Rect) -> Rect {
    window.intersection(work).unwrap_or(work)
}

#[cfg(test)]
mod tests {
    use super::{
        max_info, maximized_client_rect, maximized_rect, taskbar_edge, Edge, MaxInfo, Rect,
    };

    const MONITOR: Rect = Rect::new(0, 0, 1920, 1080);

    #[test]
    fn a_maximize_fills_the_work_area_beside_a_taskbar_on_any_edge() {
        for (work, edge) in [
            (Rect::new(0, 0, 1920, 1032), Edge::Bottom),
            (Rect::new(0, 48, 1920, 1080), Edge::Top),
            (Rect::new(48, 0, 1920, 1080), Edge::Left),
            (Rect::new(0, 0, 1872, 1080), Edge::Right),
            (Rect::new(0, 2, 1920, 1080), Edge::Top),
        ] {
            assert_eq!(maximized_rect(MONITOR, work, &[edge], None), Some(work));
        }
    }

    #[test]
    fn a_maximize_on_a_monitor_left_of_and_above_the_primary_keeps_its_work_area() {
        let monitor = Rect::new(-1920, -1080, 0, 0);
        let work = Rect::new(-1920, -1032, 0, 0);
        assert_eq!(maximized_rect(monitor, work, &[Edge::Top], None), Some(work));
    }

    #[test]
    fn a_maximize_leaves_a_pixel_strip_on_an_auto_hidden_taskbar_edge() {
        for (edge, expected) in [
            (Edge::Bottom, Rect::new(0, 0, 1920, 1079)),
            (Edge::Top, Rect::new(0, 1, 1920, 1080)),
            (Edge::Left, Rect::new(1, 0, 1920, 1080)),
            (Edge::Right, Rect::new(0, 0, 1919, 1080)),
        ] {
            assert_eq!(maximized_rect(MONITOR, MONITOR, &[edge], None), Some(expected));
        }
        assert_eq!(maximized_rect(MONITOR, MONITOR, &[], None), Some(MONITOR), "no taskbar");
        // A second monitor's taskbar auto-hides at the bottom while the primary's is shown on
        // the left: only the auto-hidden edge gives up a pixel.
        let work = Rect::new(48, 0, 1920, 1080);
        assert_eq!(
            maximized_rect(MONITOR, work, &[Edge::Left, Edge::Bottom], None),
            Some(Rect::new(48, 0, 1920, 1079))
        );
    }

    #[test]
    fn a_work_area_outside_its_monitor_is_left_to_windows() {
        assert_eq!(maximized_rect(MONITOR, Rect::new(0, 0, 0, 0), &[], None), None);
        assert_eq!(maximized_rect(MONITOR, Rect::new(-10, 0, 1920, 1080), &[], None), None);
        let tiny = Rect::new(0, 0, 1, 1);
        assert_eq!(maximized_rect(tiny, tiny, &[Edge::Left, Edge::Right], None), Some(tiny));
    }

    #[test]
    fn a_maximize_stops_at_the_maximum_size() {
        let work = Rect::new(0, 48, 1920, 1080);
        assert_eq!(
            maximized_rect(MONITOR, work, &[Edge::Top], Some((1200, 800))),
            Some(Rect::new(0, 48, 1200, 848))
        );
        assert_eq!(
            maximized_rect(MONITOR, work, &[Edge::Top], Some((4000, 4000))),
            Some(work),
            "larger than the work area"
        );
        assert_eq!(
            maximized_rect(MONITOR, MONITOR, &[Edge::Left], Some((i32::MAX, 500))),
            Some(Rect::new(1, 0, 1920, 500)),
            "no overflow"
        );
    }

    #[test]
    fn a_taskbar_sits_on_the_monitor_edge_nearest_its_centre() {
        assert_eq!(taskbar_edge(Rect::new(0, 1032, 1920, 1080), MONITOR), Some(Edge::Bottom));
        assert_eq!(
            taskbar_edge(Rect::new(0, 1078, 1920, 1126), MONITOR),
            Some(Edge::Bottom),
            "auto-hidden"
        );
        assert_eq!(taskbar_edge(Rect::new(0, -46, 1920, 2), MONITOR), Some(Edge::Top));
        assert_eq!(taskbar_edge(Rect::new(-60, 0, 2, 1080), MONITOR), Some(Edge::Left));
        assert_eq!(taskbar_edge(Rect::new(1918, 0, 1980, 1080), MONITOR), Some(Edge::Right));
        assert_eq!(taskbar_edge(Rect::new(1920, 1032, 3840, 1080), MONITOR), None, "other monitor");
        assert!(
            taskbar_edge(Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX), MONITOR).is_some(),
            "no overflow"
        );
    }

    #[test]
    fn the_primary_monitor_takes_the_maximize_rect_as_it_is() {
        let primary = (1920, 1080);
        for target in [
            Rect::new(0, 0, 1920, 1032),
            Rect::new(0, 48, 1920, 1080),
            Rect::new(0, 0, 1920, 1079),
            Rect::new(1, 0, 1920, 1080),
            MONITOR,
        ] {
            let info = max_info(target, MONITOR, primary).unwrap();
            assert_eq!(info.position, (target.left, target.top));
            assert_eq!(info.size, (target.right - target.left, target.bottom - target.top));
        }
    }

    #[test]
    fn a_larger_second_monitor_gets_a_size_in_primary_terms() {
        let primary = (1920, 1080);
        let monitor = Rect::new(1920, 0, 4480, 1440);
        // No taskbar there: Windows adds the 640 × 360 difference to a primary-sized maximize.
        assert_eq!(
            max_info(monitor, monitor, primary),
            Some(MaxInfo { position: (0, 0), size: (1920, 1080) })
        );
        // With a taskbar, any size Windows would grow is too large and any it keeps too small.
        assert_eq!(max_info(Rect::new(1920, 0, 4480, 1400), monitor, primary), None);
    }

    #[test]
    fn a_smaller_second_monitor_takes_its_work_area_as_it_is() {
        let monitor = Rect::new(-1366, 0, 0, 768);
        let work = Rect::new(-1366, 0, 0, 728);
        assert_eq!(
            max_info(work, monitor, (1920, 1080)),
            Some(MaxInfo { position: (0, 0), size: (1366, 728) })
        );
    }

    #[test]
    fn a_maximized_client_stays_inside_the_window_and_the_work_area() {
        let work = Rect::new(0, 0, 1920, 1032);
        assert_eq!(maximized_client_rect(Rect::new(-8, -8, 1928, 1040), work), work, "overhang");
        assert_eq!(maximized_client_rect(work, work), work);
        let strip = Rect::new(0, 0, 1920, 1079);
        assert_eq!(maximized_client_rect(strip, MONITOR), strip, "auto-hide strip");
        assert_eq!(
            maximized_client_rect(Rect::new(3000, 0, 4000, 100), work),
            work,
            "another monitor"
        );
    }
}
