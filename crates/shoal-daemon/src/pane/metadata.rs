//! Event-driven metadata scheduling and OSC 7 decoding. Owned by the pane thread.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64},
    time::{Duration, Instant},
};

use shoal_term::TermEvent;

use super::Metadata;

const QUIET: Duration = Duration::from_millis(300);
const INTERVAL: Duration = Duration::from_millis(250);

/// Non-waking diagnostics the pane thread publishes for its handle.
#[derive(Debug, Default)]
pub(super) struct Activity {
    pub samples: AtomicU64,
    pub pending: AtomicBool,
    /// A scrollback save is scheduled (`runtime::persist`).
    pub persist_pending: AtomicBool,
}

#[derive(Default)]
pub(super) struct Tracker {
    pub value: Metadata,
    reported_directory: Option<PathBuf>,
    quiet: Option<Instant>,
    triggered: Option<Instant>,
    last: Option<Instant>,
}

impl Tracker {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            value: Metadata {
                cwd: Some(cwd),
                ..Metadata::default()
            },
            ..Self::default()
        }
    }

    pub fn output(&mut self, now: Instant) {
        self.quiet = Some(now + QUIET);
    }

    pub fn enter(&mut self, now: Instant) {
        // Give the shell one throttle interval to hand off its foreground
        // group. This also catches silent commands with terminal echo off.
        self.trigger(now + INTERVAL);
    }

    fn trigger(&mut self, at: Instant) {
        self.triggered = Some(self.triggered.map_or(at, |old| old.min(at)));
    }

    pub fn event(&mut self, event: TermEvent, now: Instant) {
        match event {
            TermEvent::Bell => self.value.bell = Some(now),
            TermEvent::Title(bytes) => {
                self.value.osc_title =
                    (!bytes.is_empty()).then(|| String::from_utf8_lossy(&bytes).into_owned());
            }
            TermEvent::Directory(bytes) => {
                let Some(path) = directory(&bytes) else {
                    return;
                };
                self.reported_directory = Some(path);
            }
            // Command completion events and durations belong to M2.
            TermEvent::Prompt(_) => return,
        }
        self.trigger(now);
    }

    pub fn deadline(&self) -> Option<Instant> {
        let pending = self.quiet.into_iter().chain(self.triggered).min()?;
        Some(
            self.last
                .map_or(pending, |last| pending.max(last + INTERVAL)),
        )
    }

    pub fn due(&mut self, now: Instant) -> bool {
        if self.deadline().is_none_or(|at| now < at) {
            return false;
        }
        if self.quiet.is_some_and(|at| at <= now) {
            self.quiet = None;
        }
        if self.triggered.is_some_and(|at| at <= now) {
            self.triggered = None;
        }
        self.last = Some(now);
        true
    }

    pub fn collect(&mut self, foreground: Option<i32>, shell: i32) -> Metadata {
        self.value.program = foreground.filter(|&pid| pid != shell).and_then(|pid| {
            let mut comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
            // proc appends exactly one newline; other whitespace is part of the name.
            if comm.ends_with('\n') {
                comm.pop();
            }
            (!comm.is_empty()).then_some(comm)
        });
        if let Some(path) = self
            .reported_directory
            .clone()
            .or_else(|| std::fs::read_link(format!("/proc/{}/cwd", foreground?)).ok())
            && path.to_str().is_some()
        {
            self.value.cwd = Some(path);
        }
        let value = self.value.clone();
        self.value.bell = None;
        value
    }
}

/// Decode a file URI bytewise. Invalid reports, including paths the JSON
/// workspace cannot represent, leave the previous directory in force.
fn directory(uri: &[u8]) -> Option<PathBuf> {
    let rest = uri.strip_prefix(b"file://")?;
    let start = rest.iter().position(|&b| b == b'/')?;
    if !rest[..start]
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(b))
    {
        return None;
    }
    let mut bytes = Vec::with_capacity(rest.len() - start);
    let mut path = rest[start..].iter().copied();
    while let Some(b) = path.next() {
        let b = if b == b'%' {
            hex(path.next()?)? * 16 + hex(path.next()?)?
        } else {
            b
        };
        if b == 0 {
            return None;
        }
        bytes.push(b);
    }
    String::from_utf8(bytes).ok().map(PathBuf::from)
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadlines_debounce_output_throttle_events_and_stop_when_idle() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut tracker = Tracker::default();
        assert_eq!(tracker.deadline(), None);
        tracker.output(at(0));
        tracker.output(at(200));
        assert_eq!(tracker.deadline(), Some(at(500)));
        assert!(!tracker.due(at(499)));
        assert!(tracker.due(at(500)));
        assert_eq!(tracker.deadline(), None);
        tracker.event(TermEvent::Bell, at(501));
        tracker.event(TermEvent::Title(b"first".to_vec()), at(510));
        tracker.event(TermEvent::Title(b"latest".to_vec()), at(740));
        assert_eq!(tracker.deadline(), Some(at(750)));
        assert!(!tracker.due(at(749)));
        assert!(tracker.due(at(750)));
        let value = tracker.collect(None, 0);
        assert_eq!(value.osc_title.as_deref(), Some("latest"));
        assert_eq!(value.bell, Some(at(501)));
        assert_eq!(tracker.collect(None, 0).bell, None);
        assert_eq!(tracker.deadline(), None);
        assert!(!tracker.due(at(5750)));
        tracker.enter(at(6000));
        assert_eq!(tracker.deadline(), Some(at(6250)));
        // Continuous output cannot postpone the Enter trigger.
        tracker.output(at(6200));
        assert!(tracker.due(at(6250)));
        assert_eq!(tracker.deadline(), Some(at(6500)));
        assert!(tracker.due(at(6500)));
        assert_eq!(tracker.deadline(), None);
    }

    #[test]
    fn directory_uris_decode_path_bytes_and_reject_unrepresentable_reports() {
        assert_eq!(
            directory(b"file://host-1/a%20b/%25/%c3%a9/%09"),
            Some(PathBuf::from("/a b/%/é/\t"))
        );
        assert_eq!(directory(b"file:///"), Some(PathBuf::from("/")));
        for invalid in [
            b"/tmp".as_slice(),
            b"https://host/tmp",
            b"file://host",
            b"file://bad host/tmp",
            b"file://h/%",
            b"file://h/%0",
            b"file://h/%GG",
            b"file://h/%00",
            b"file://h/%FF",
        ] {
            assert_eq!(directory(invalid), None, "{invalid:?}");
        }
        let now = Instant::now();
        let mut tracker = Tracker::default();
        tracker.event(TermEvent::Directory(b"file://h/valid".to_vec()), now);
        tracker.event(TermEvent::Directory(b"file://h/%GG".to_vec()), now);
        assert_eq!(tracker.collect(None, 0).cwd, Some(PathBuf::from("/valid")));
    }
}
