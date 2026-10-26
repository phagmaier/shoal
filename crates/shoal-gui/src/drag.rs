//! Dragging a divider (M1 spec §7.2): where a divider can be grabbed, the
//! ratio a pointer position asks for, and sending it at most 60 times a
//! second.

use std::time::{Duration, Instant};

use shoal_model::{Axis, Divider, Layout, SplitId};

/// The narrowest grab target, centred on the gap, so a small or zero
/// `window.gap` can still be dragged.
const MIN_GRAB: f32 = 6.0;
/// `split.set_ratio` goes at most this often.
pub const RATIO_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);

/// Whether `point`, in pane-area pixels, grabs `divider`.
pub fn grabs(divider: &Divider, (x, y): (f32, f32)) -> bool {
    let Grab {
        left,
        top,
        right,
        bottom,
    } = grab_area(divider);
    (left..right).contains(&x) && (top..bottom).contains(&y)
}

/// Where a divider can be grabbed, in pane-area pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grab {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// A divider's gap, widened to [`MIN_GRAB`] across the axis.
pub fn grab_area(divider: &Divider) -> Grab {
    let gap = divider.gap;
    let widen = |start: u32, extent: u32| {
        let extra = (MIN_GRAB - extent as f32).max(0.0) / 2.0;
        (start as f32 - extra, start as f32 + extent as f32 + extra)
    };
    let ((left, right), (top, bottom)) = match divider.axis {
        Axis::Row => (widen(gap.x, gap.width), span(gap.y, gap.height)),
        Axis::Column => (span(gap.x, gap.width), widen(gap.y, gap.height)),
    };
    Grab {
        left,
        top,
        right,
        bottom,
    }
}

fn span(start: u32, extent: u32) -> (f32, f32) {
    (start as f32, start as f32 + extent as f32)
}

/// A divider being dragged.
#[derive(Debug, Clone, PartialEq)]
pub struct Drag {
    pub split: SplitId,
    pub axis: Axis,
    /// The ratio the pointer asks for now.
    pub ratio: f32,
    /// Where the split's area starts along the axis.
    start: f32,
    usable: u32,
    /// From the gap's leading edge to where it was grabbed, so the divider
    /// doesn't jump to the pointer.
    grab: f32,
    /// The last ratio sent, at first the one in force.
    sent: f32,
    sent_at: Option<Instant>,
}

impl Drag {
    pub fn new(divider: &Divider, point: (f32, f32)) -> Self {
        let (start, gap_start) = match divider.axis {
            Axis::Row => (divider.area.x, divider.gap.x),
            Axis::Column => (divider.area.y, divider.gap.y),
        };
        Self {
            split: divider.split,
            axis: divider.axis,
            ratio: divider.ratio,
            start: start as f32,
            usable: divider.usable(),
            grab: along(divider.axis, point) - gap_start as f32,
            sent: divider.ratio,
            sent_at: None,
        }
    }

    /// Follow the pointer at `point`. Returns whether the ratio changed.
    pub fn move_to(&mut self, point: (f32, f32)) -> bool {
        if self.usable == 0 {
            return false;
        }
        let usable = self.usable as f32;
        let first = (along(self.axis, point) - self.grab - self.start)
            .round()
            .clamp(0.0, usable);
        // Half a pixel past the edge, so the split's floor lands on it.
        let ratio = ((first + 0.5) / usable)
            .clamp(*Layout::RATIO_RANGE.start(), *Layout::RATIO_RANGE.end());
        let changed = ratio != self.ratio;
        self.ratio = ratio;
        changed
    }

    /// The ratio to send now, if it hasn't been sent and the last send was
    /// at least [`RATIO_INTERVAL`] ago; otherwise how long until it may go.
    pub fn due(&self, now: Instant) -> Result<Option<f32>, Duration> {
        match self.sent_at {
            _ if self.sent == self.ratio => Ok(None),
            Some(at) if now < at + RATIO_INTERVAL => Err(at + RATIO_INTERVAL - now),
            _ => Ok(Some(self.ratio)),
        }
    }

    pub fn sent(&mut self, ratio: f32, now: Instant) {
        self.sent = ratio;
        self.sent_at = Some(now);
    }
}

fn along(axis: Axis, (x, y): (f32, f32)) -> f32 {
    match axis {
        Axis::Row => x,
        Axis::Column => y,
    }
}

#[cfg(test)]
mod tests {
    use shoal_model::{PaneId, Rect, dividers, rects, set_ratio};

    use super::*;

    fn r(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    /// Panes 1 and 2 side by side in a 406 px wide area from x 8, with a
    /// 6 px gap: 400 px to share, the gap at 208..214.
    fn side_by_side(ratio: f32) -> Layout {
        Layout::Split {
            id: SplitId::new(3),
            axis: Axis::Row,
            ratio,
            first: Box::new(Layout::Pane(PaneId::new(1))),
            second: Box::new(Layout::Pane(PaneId::new(2))),
        }
    }

    const AREA: Rect = Rect {
        x: 8,
        y: 8,
        width: 406,
        height: 300,
    };

    #[test]
    fn the_gap_is_the_grab_target() {
        let divider = dividers(&side_by_side(0.5), AREA, 6).unwrap()[0];
        assert_eq!(divider.gap, r(208, 8, 6, 300));
        assert!(grabs(&divider, (208.0, 100.0)));
        assert!(grabs(&divider, (213.9, 307.9)));
        assert!(!grabs(&divider, (207.9, 100.0)));
        assert!(!grabs(&divider, (214.0, 100.0)));
        assert!(!grabs(&divider, (210.0, 308.0)));
        // A 2 px gap still grabs 6 px: 2 more on each side.
        let thin = dividers(&side_by_side(0.5), AREA, 2).unwrap()[0];
        assert_eq!(thin.gap, r(210, 8, 2, 300));
        assert!(grabs(&thin, (208.0, 100.0)));
        assert!(grabs(&thin, (213.9, 100.0)));
        assert!(!grabs(&thin, (214.0, 100.0)));
        // A stacked split's gap runs across.
        let mut stacked = side_by_side(0.5);
        if let Layout::Split { axis, .. } = &mut stacked {
            *axis = Axis::Column;
        }
        let across = dividers(&stacked, AREA, 6).unwrap()[0];
        assert_eq!(across.gap, r(8, 155, 406, 6));
        assert!(grabs(&across, (300.0, 157.0)));
        assert!(!grabs(&across, (300.0, 161.0)));
    }

    #[test]
    fn the_divider_follows_the_pointer_to_the_pixel() {
        let mut layout = side_by_side(0.5);
        let divider = dividers(&layout, AREA, 6).unwrap()[0];
        // Grabbed 2 px into the gap, then moved 37 px right and 0.4 down.
        let mut drag = Drag::new(&divider, (210.0, 50.0));
        assert!(drag.move_to((247.0, 50.4)));
        // Laid out again, the gap starts 37 px further right, at 245, and
        // the first pane is 237 wide.
        set_ratio(&mut layout, drag.split, drag.ratio).unwrap();
        assert_eq!(dividers(&layout, AREA, 6).unwrap()[0].gap.x, 245);
        assert_eq!(rects(&layout, AREA, 6).unwrap()[&PaneId::new(1)].width, 237);
        // Every pixel lands exactly within the ratio range: 0.1 of 400 puts
        // the gap at 48, and 0.9, which f32 holds as slightly less, at 367.
        for x in 48..=367 {
            drag.move_to((x as f32 + 2.0, 50.0));
            set_ratio(&mut layout, drag.split, drag.ratio).unwrap();
            assert_eq!(dividers(&layout, AREA, 6).unwrap()[0].gap.x, x, "x {x}");
        }
    }

    #[test]
    fn the_ratio_stops_at_its_range() {
        let divider = dividers(&side_by_side(0.5), AREA, 6).unwrap()[0];
        let mut drag = Drag::new(&divider, (208.0, 0.0));
        drag.move_to((-500.0, 0.0));
        assert_eq!(drag.ratio, 0.1);
        drag.move_to((5000.0, 0.0));
        assert_eq!(drag.ratio, 0.9);
        // Moving across the axis changes nothing.
        assert!(!drag.move_to((5000.0, 200.0)));
    }

    #[test]
    fn ratios_go_at_most_sixty_times_a_second() {
        let divider = dividers(&side_by_side(0.5), AREA, 6).unwrap()[0];
        let mut drag = Drag::new(&divider, (208.0, 0.0));
        let start = Instant::now();
        // Nothing to send before the pointer moves; the first move goes at
        // once.
        assert_eq!(drag.due(start), Ok(None));
        drag.move_to((250.0, 0.0));
        let moved = drag.ratio;
        assert_eq!(drag.due(start), Ok(Some(moved)));
        drag.sent(moved, start);
        assert_eq!(drag.due(start), Ok(None));
        drag.move_to((260.0, 0.0));
        let moved = drag.ratio;
        // 10 ms later it must wait the rest of the 16.7 ms.
        let soon = start + Duration::from_millis(10);
        assert_eq!(
            drag.due(soon),
            Err(RATIO_INTERVAL - Duration::from_millis(10))
        );
        assert_eq!(drag.due(start + RATIO_INTERVAL), Ok(Some(moved)));
    }
}
