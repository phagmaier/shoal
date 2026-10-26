//! A window's point-size offset from the live configured font.
use shoal_config::Action;

#[derive(Debug, Default)]
pub struct Zoom {
    offset: f64,
}

impl Zoom {
    pub fn size(&self, configured: f64) -> f64 {
        if self.offset == 0.0 {
            configured
        } else {
            (configured + self.offset).clamp(4.0, 72.0)
        }
    }

    pub fn change(&mut self, action: Action, configured: f64) -> bool {
        let before = self.size(configured);
        let next = match action {
            Action::FontIncrease if before < 72.0 => (before + 1.0).clamp(4.0, 72.0),
            Action::FontDecrease if before > 4.0 => (before - 1.0).clamp(4.0, 72.0),
            Action::FontReset => configured,
            _ => return false,
        };
        self.offset = next - configured;
        next != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_is_per_window_and_reset_uses_the_live_config() {
        let mut first = Zoom::default();
        let second = Zoom::default();
        first.change(Action::FontIncrease, 11.0);
        first.change(Action::FontIncrease, 11.0);
        assert_eq!(first.size(11.0), 13.0);
        assert_eq!(second.size(11.0), 11.0);
        assert_eq!(first.size(14.0), 16.0, "config reload preserves the offset");
        first.change(Action::FontDecrease, 14.0);
        assert_eq!(first.size(14.0), 15.0);
        first.change(Action::FontReset, 14.0);
        assert_eq!(first.size(14.0), 14.0);
    }

    #[test]
    fn shortcuts_are_bounded_without_restricting_explicit_config() {
        let mut zoom = Zoom::default();
        for _ in 0..100 {
            zoom.change(Action::FontIncrease, 11.5);
        }
        assert_eq!(zoom.size(11.5), 72.0);
        for _ in 0..100 {
            zoom.change(Action::FontDecrease, 11.5);
        }
        assert_eq!(zoom.size(11.5), 4.0);
        zoom.change(Action::FontReset, 100.0);
        assert_eq!(zoom.size(100.0), 100.0);
        assert!(!zoom.change(Action::FontIncrease, 100.0));
        zoom.change(Action::FontReset, 2.0);
        assert_eq!(zoom.size(2.0), 2.0);
        assert!(!zoom.change(Action::FontDecrease, 2.0));
    }
}
