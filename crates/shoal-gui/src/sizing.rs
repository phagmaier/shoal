//! When the window asks for a pane's grid size (M1 spec §7.2, §5.6): as
//! soon as a shown pane's grid changes, or once it has held still for
//! 50 ms while a divider is being dragged.

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use shoal_model::{GridSize, PaneId};

/// How long a grid must hold still during a drag before it is asked for.
pub const DRAG_DEBOUNCE: Duration = Duration::from_millis(50);

#[derive(Debug, Default)]
pub struct Sizer {
    /// What this window last asked for each pane.
    asked: BTreeMap<PaneId, GridSize>,
    /// During a drag: the changes waiting, and when they may go.
    waiting: Option<(Vec<(PaneId, GridSize)>, Instant)>,
}

impl Sizer {
    /// Forget what was asked, so each shown pane's size is asked for again:
    /// after this window becomes the one whose sizes count, or reconnects.
    pub fn reset(&mut self) {
        self.asked.clear();
        self.waiting = None;
    }

    /// The `pane.set_size` requests to send now for the shown panes'
    /// `wanted` grids. `current` is each pane's size in force, `None` once
    /// it is gone. A size differs if it isn't what this window last asked
    /// for, or, if it never asked, what is in force; a window whose sizes
    /// don't count gets the size in force back and doesn't ask again. While
    /// `dragging`, changes wait for [`DRAG_DEBOUNCE`]; [`Sizer::deadline`]
    /// says when to call again.
    pub fn update(
        &mut self,
        wanted: &[(PaneId, GridSize)],
        current: impl Fn(PaneId) -> Option<GridSize>,
        now: Instant,
        dragging: bool,
    ) -> Vec<(PaneId, GridSize)> {
        self.asked.retain(|pane, _| current(*pane).is_some());
        let changed: Vec<_> = wanted
            .iter()
            .copied()
            .filter(|(pane, size)| {
                let known = self.asked.get(pane).copied().or_else(|| current(*pane));
                known.is_some() && known != Some(*size)
            })
            .collect();
        if changed.is_empty() {
            self.waiting = None;
            return changed;
        }
        if dragging {
            match &self.waiting {
                Some((waiting, due)) if *waiting == changed && now >= *due => {}
                Some((waiting, _)) if *waiting == changed => return Vec::new(),
                _ => {
                    self.waiting = Some((changed, now + DRAG_DEBOUNCE));
                    return Vec::new();
                }
            }
        }
        self.waiting = None;
        self.asked.extend(changed.iter().copied());
        changed
    }

    /// When the waiting changes may go.
    pub fn deadline(&self) -> Option<Instant> {
        self.waiting.as_ref().map(|(_, due)| *due)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: u64) -> PaneId {
        PaneId::new(id)
    }

    fn g(cols: u16, rows: u16) -> GridSize {
        GridSize { cols, rows }
    }

    /// Panes 1 and 2, both 80 × 24 in force.
    fn in_force(pane: PaneId) -> Option<GridSize> {
        [p(1), p(2)].contains(&pane).then_some(g(80, 24))
    }

    #[test]
    fn a_changed_grid_is_asked_for_once() {
        let mut sizer = Sizer::default();
        let now = Instant::now();
        // Pane 1 already has its size; pane 2 grows.
        let wanted = [(p(1), g(80, 24)), (p(2), g(100, 30))];
        assert_eq!(
            sizer.update(&wanted, in_force, now, false),
            vec![(p(2), g(100, 30))]
        );
        // Asked for: not again, even though another window keeps 80 × 24.
        assert!(sizer.update(&wanted, in_force, now, false).is_empty());
        // After a reset (this window regained focus) it asks again.
        sizer.reset();
        assert_eq!(
            sizer.update(&wanted, in_force, now, false),
            vec![(p(2), g(100, 30))]
        );
        // A pane that is gone is never asked for.
        assert!(
            sizer
                .update(&[(p(9), g(10, 10))], in_force, now, false)
                .is_empty()
        );
    }

    #[test]
    fn during_a_drag_a_grid_waits_until_it_holds_still() {
        let mut sizer = Sizer::default();
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let first = [(p(1), g(70, 24)), (p(2), g(90, 24))];
        let second = [(p(1), g(69, 24)), (p(2), g(91, 24))];
        assert!(sizer.update(&first, in_force, at(0), true).is_empty());
        assert_eq!(sizer.deadline(), Some(at(50)));
        // The same grids again leave the deadline alone.
        assert!(sizer.update(&first, in_force, at(30), true).is_empty());
        assert_eq!(sizer.deadline(), Some(at(50)));
        // A change starts the wait over.
        assert!(sizer.update(&second, in_force, at(40), true).is_empty());
        assert_eq!(sizer.deadline(), Some(at(90)));
        assert!(sizer.update(&second, in_force, at(89), true).is_empty());
        assert_eq!(sizer.update(&second, in_force, at(90), true), second);
        assert_eq!(sizer.deadline(), None);
        assert!(sizer.update(&second, in_force, at(91), true).is_empty());
    }

    #[test]
    fn letting_go_sends_what_was_waiting_at_once() {
        let mut sizer = Sizer::default();
        let now = Instant::now();
        let wanted = [(p(1), g(70, 24))];
        assert!(sizer.update(&wanted, in_force, now, true).is_empty());
        assert_eq!(sizer.update(&wanted, in_force, now, false), wanted);
        assert_eq!(sizer.deadline(), None);
        // Dragging back to the size in force cancels what was waiting.
        assert!(
            sizer
                .update(&[(p(2), g(81, 24))], in_force, now, true)
                .is_empty()
        );
        assert!(
            sizer
                .update(&[(p(2), g(80, 24))], in_force, now, true)
                .is_empty()
        );
        assert_eq!(sizer.deadline(), None);
    }
}
