//! `shoal daemon start | stop | status | persist` (M1 spec §11, §4.1, §5.3).
//! Only `start` starts anything.

use std::time::Duration;

use shoal_client::{
    Client, Connected,
    lifecycle::{self, Mismatch, Stopped, SystemdOrSpawn},
    model::Timestamp,
    proto::{Call, DaemonStatusResult, Empty},
};

use super::{Failure, connect_running, failure, internal, options, print, request};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Status,
    Persist,
}

pub(super) fn run(action: Action) -> Result<(), Failure> {
    match action {
        Action::Start => {
            let (verb, connected) = match connect_running() {
                Ok(connected) => ("shoald is already running", connected),
                Err(Failure::Unreachable(_)) => (
                    "Started shoald",
                    Client::connect(options()?, SystemdOrSpawn::new()).map_err(failure)?,
                ),
                Err(other) => return Err(other),
            };
            print(format!("{verb} (pid {}).\n", connected.hello.pid).as_bytes())
        }
        Action::Stop => match lifecycle::stop(&options()?).map_err(failure)? {
            Stopped::Stopped => print(b"Stopped shoald.\n"),
            Stopped::NotRunning => print(b"shoald isn't running.\n"),
        },
        Action::Status => {
            let Connected { client, hello, .. } = running()?;
            let result = request(&client, Call::DaemonStatus(Empty {}))?;
            let status: DaemonStatusResult = serde_json::from_value(result).map_err(internal)?;
            let mut text = describe(&status, Timestamp::now());
            if let Some(Mismatch::Version { daemon_version }) =
                Mismatch::of_hello(&options()?, &hello)
            {
                text.push_str(&format!(
                    "shoald is {daemon_version} but this shoal is {}. Restart shoald to update it.\n",
                    env!("CARGO_PKG_VERSION")
                ));
            }
            print(text.as_bytes())
        }
        Action::Persist => {
            let Connected { client, .. } = running()?;
            request(&client, Call::DaemonPersist(Empty {}))?;
            print(b"Saved.\n")
        }
    }
}

/// The running daemon, or exit 3 saying it isn't running.
fn running() -> Result<Connected, Failure> {
    connect_running().map_err(|failure| match failure {
        Failure::Unreachable(_) => Failure::Unreachable("it isn't running".into()),
        other => other,
    })
}

fn describe(status: &DaemonStatusResult, now: Timestamp) -> String {
    let saved = match status.last_persist_at {
        Some(at) => {
            let ago = now.duration_since(at).unsigned_abs();
            format!("Last saved {} ago.", span(ago))
        }
        None => "Not saved yet.".into(),
    };
    format!(
        "shoald {} is running (pid {}, up {}).\n{}, {}. {saved}\n",
        status.version,
        status.pid,
        span(Duration::from_secs(status.uptime_s)),
        count(status.sessions, "session"),
        count(status.panes, "pane"),
    )
}

fn count(n: u64, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// `4 s`, `12 min`, `2 h 5 min`, `3 d 4 h`.
fn span(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let (minutes, hours, days) = (seconds / 60, seconds / 3600, seconds / 86_400);
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3600 => format!("{minutes} min"),
        3600..86_400 => format!("{hours} h {} min", minutes % 60),
        _ => format!("{days} d {} h", hours % 24),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reads_as_sentences() {
        let now: Timestamp = "2026-10-25T20:30:14Z".parse().unwrap();
        let mut status = DaemonStatusResult {
            version: "0.1.0".into(),
            pid: 4312,
            uptime_s: 7500,
            sessions: 2,
            panes: 1,
            last_persist_at: Some("2026-10-25T20:30:02Z".parse().unwrap()),
        };
        assert_eq!(
            describe(&status, now),
            "shoald 0.1.0 is running (pid 4312, up 2 h 5 min).\n2 sessions, 1 pane. Last saved 12 s ago.\n"
        );
        status.last_persist_at = None;
        status.sessions = 1;
        status.panes = 0;
        status.uptime_s = 3;
        assert_eq!(
            describe(&status, now),
            "shoald 0.1.0 is running (pid 4312, up 3 s).\n1 session, 0 panes. Not saved yet.\n"
        );
    }

    #[test]
    fn spans_round_down_to_two_units() {
        let span = |seconds| span(Duration::from_secs(seconds));
        assert_eq!(span(59), "59 s");
        assert_eq!(span(60), "1 min");
        assert_eq!(span(3599), "59 min");
        assert_eq!(span(3600), "1 h 0 min");
        assert_eq!(span(90_000), "1 d 1 h");
    }
}
