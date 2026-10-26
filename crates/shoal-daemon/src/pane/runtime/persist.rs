//! When a pane thread hands its scrollback to the writer (M1 spec §6.3): after
//! 2 s without output, at most once every 30 s. Deadlines exist only while
//! unsaved output is waiting, never while idle.

use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use shoal_term::{Persisted, SnapshotError};

use super::{Event, Runtime};

const QUIET: Duration = Duration::from_secs(2);
const INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub(super) struct Schedule {
    enabled: bool,
    /// Output arrived since the last capture.
    dirty: bool,
    /// The last capture failed; wait for more output before trying again.
    blocked: bool,
    last_output: Option<Instant>,
    last_capture: Option<Instant>,
}

impl Schedule {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            dirty: false,
            blocked: false,
            last_output: None,
            last_capture: None,
        }
    }

    pub fn output(&mut self, now: Instant) {
        if self.enabled {
            self.dirty = true;
            self.blocked = false;
            self.last_output = Some(now);
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        if !self.enabled || !self.dirty || self.blocked {
            return None;
        }
        let quiet = self.last_output? + QUIET;
        Some(
            self.last_capture
                .map_or(quiet, |last| quiet.max(last + INTERVAL)),
        )
    }

    /// Turning persistence back on saves what the pane already shows.
    pub fn set_enabled(&mut self, enabled: bool, now: Instant) {
        if enabled && !self.enabled {
            self.enabled = true;
            self.output(now);
        }
        self.enabled = enabled;
        self.dirty &= enabled;
    }

    fn captured(&mut self, result: &Result<Persisted, SnapshotError>, now: Instant) {
        if result.is_ok() {
            self.dirty = false;
            self.last_capture = Some(now);
        } else {
            self.blocked = true;
        }
    }
}

impl Runtime {
    /// The pane thread's half of saving: formatter output, or a snapshot when
    /// a full-screen program hides the primary screen.
    pub(super) fn capture_scrollback(&mut self, now: Instant) -> Result<Persisted, SnapshotError> {
        let result = self.terminal.persisted();
        self.persist.captured(&result, now);
        result
    }

    pub(super) fn save_due(&mut self, now: Instant) {
        if self.persist.deadline().is_some_and(|due| now >= due) {
            match self.capture_scrollback(now) {
                Ok(persisted) => {
                    let _ = self.events.send((self.id, Event::Scrollback(persisted)));
                }
                // Skipped until the next output, keeping the previous file.
                Err(SnapshotError::MidSequence) => {}
                Err(error) => {
                    tracing::warn!(pane = %self.id, %error, "could not capture scrollback");
                }
            }
        }
        self.metadata_activity
            .persist_pending
            .store(self.persist.deadline().is_some(), Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_wait_for_two_quiet_seconds_and_thirty_since_the_last() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut schedule = Schedule::new(true);
        assert_eq!(schedule.deadline(), None);
        schedule.output(at(0));
        schedule.output(at(1));
        assert_eq!(schedule.deadline(), Some(at(3)));
        schedule.captured(&Ok(Persisted::Vt(Vec::new())), at(3));
        assert_eq!(schedule.deadline(), None);
        schedule.output(at(4));
        assert_eq!(schedule.deadline(), Some(at(33)));
        schedule.output(at(40));
        assert_eq!(schedule.deadline(), Some(at(42)));
    }

    #[test]
    fn a_failed_capture_waits_for_output_and_disabling_drops_pending_work() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut schedule = Schedule::new(true);
        schedule.output(at(0));
        schedule.captured(&Err(SnapshotError::MidSequence), at(2));
        assert_eq!(schedule.deadline(), None);
        schedule.output(at(3));
        assert_eq!(schedule.deadline(), Some(at(5)));
        schedule.set_enabled(false, at(4));
        assert_eq!(schedule.deadline(), None);
        schedule.output(at(5));
        assert_eq!(schedule.deadline(), None);
        schedule.set_enabled(true, at(6));
        assert_eq!(schedule.deadline(), Some(at(8)));
    }
}
