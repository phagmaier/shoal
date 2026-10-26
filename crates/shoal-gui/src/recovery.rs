//! Window-level connection and notice state (§5.3, §7.7). No clocks or I/O.
use shoal_client::{Error, Options, lifecycle::Mismatch, proto::HelloResult};

pub const BUILD_MISMATCH: &str = "shoald is from a different build. Restart it to show panes. Running programs will end; layout and scrollback come back.";
const VERSION_MISMATCH: &str = "shoald is from a different version. Running programs will end; layout and scrollback come back.";

#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    Connected,
    Reconnecting,
    Incompatible,
    Restarting,
    Failed(String),
}

pub struct Recovery {
    pub status: Status,
    pub mismatch: Option<Mismatch>,
}

impl Recovery {
    pub fn new() -> Self {
        Self {
            status: Status::Connected,
            mismatch: None,
        }
    }

    pub fn connected(&mut self, options: &Options, hello: &HelloResult) {
        self.mismatch = Mismatch::of_hello(options, hello);
        self.status = if matches!(self.mismatch, Some(Mismatch::Core { .. })) {
            Status::Incompatible
        } else {
            Status::Connected
        };
    }

    pub fn disconnected(&mut self, error: &Error) {
        self.mismatch = Mismatch::of_error(error);
        self.status = if self.mismatch.is_some() {
            Status::Incompatible
        } else {
            Status::Reconnecting
        };
    }

    pub fn panes_allowed(&self) -> bool {
        self.status == Status::Connected
    }

    pub fn begin_restart(&mut self) -> bool {
        if self.mismatch.is_none() || self.status == Status::Restarting {
            return false;
        }
        self.status = Status::Restarting;
        true
    }

    pub fn restart_failed(&mut self, error: &Error) {
        self.status = Status::Failed(match error {
            Error::DidNotStop => "shoald didn't stop. Run `shoal daemon stop`.".into(),
            _ => format!("Couldn't restart shoald: {error}"),
        });
    }

    pub fn explanation(&self) -> Option<&'static str> {
        self.mismatch.as_ref().map(|mismatch| match mismatch {
            Mismatch::Version { .. } => VERSION_MISMATCH,
            _ => BUILD_MISMATCH,
        })
    }

    pub fn overlay(&self) -> Option<&str> {
        match &self.status {
            Status::Reconnecting => Some("Reconnecting to shoald…"),
            Status::Restarting => Some("Restarting shoald…"),
            Status::Failed(message) => Some(message),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct Notice {
    pub id: usize,
    key: Option<String>,
    pub text: String,
    dismissed: bool,
}

#[derive(Default)]
pub struct Notices(Vec<Notice>);

impl Notices {
    pub fn push(&mut self, key: Option<String>, message: String) {
        let text = message.split_whitespace().collect::<Vec<_>>().join(" ");
        if let Some(notice) = self
            .0
            .iter_mut()
            .find(|n| n.text == text || (key.is_some() && n.key == key))
        {
            if notice.text != text {
                notice.dismissed = false;
            }
            notice.text = text;
            // The daemon may have reported the same problem without a key
            // before the local watcher supplied its stable replacement key.
            if notice.key.is_none() {
                notice.key = key;
            }
            return;
        }
        self.0.push(Notice {
            id: self.0.len(),
            key,
            text,
            dismissed: false,
        });
    }

    pub fn dismiss(&mut self, id: usize) {
        if let Some(notice) = self.0.get_mut(id) {
            notice.dismissed = true;
        }
    }

    pub fn visible(&self) -> impl Iterator<Item = &Notice> {
        self.0.iter().filter(|n| !n.dismissed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shoal_client::proto::{ClientKind, RpcError};

    fn options() -> Options {
        let mut options = Options::new("/unused", ClientKind::Gui);
        options.core = Some("current".into());
        options
    }

    fn hello(core: &str, version: &str) -> HelloResult {
        HelloResult {
            protocol: 1,
            pid: 7,
            core: core.into(),
            daemon_version: version.into(),
        }
    }

    #[test]
    fn lost_connections_blank_panes_and_a_refused_reconnect_requires_a_click() {
        let options = options();
        let mut recovery = Recovery::new();
        recovery.connected(&options, &hello("current", &options.version));
        recovery.disconnected(&Error::NotConnected);
        assert!(!recovery.panes_allowed());
        assert_eq!(recovery.overlay(), Some("Reconnecting to shoald…"));
        assert!(
            !recovery.begin_restart(),
            "ordinary loss must never start a daemon"
        );
        recovery.disconnected(&Error::Rpc(RpcError {
            code: 1003,
            message: "protocol mismatch".into(),
            data: Some(serde_json::json!({"daemon_protocol": 2, "pid": 7})),
        }));
        assert_eq!(recovery.status, Status::Incompatible);
        assert_eq!(recovery.explanation(), Some(BUILD_MISMATCH));
        assert!(recovery.begin_restart());
        assert!(!recovery.begin_restart(), "one restart at a time");
        recovery.restart_failed(&Error::DidNotStop);
        assert_eq!(
            recovery.overlay(),
            Some("shoald didn't stop. Run `shoal daemon stop`.")
        );
        assert!(!recovery.panes_allowed());
        recovery.connected(&options, &hello("current", &options.version));
        assert!(recovery.panes_allowed());
        assert_eq!(recovery.explanation(), None);
    }

    #[test]
    fn core_and_version_mismatches_have_different_attach_rules() {
        let options = options();
        let mut recovery = Recovery::new();
        recovery.connected(&options, &hello("old", "old-version"));
        assert!(!recovery.panes_allowed());
        assert_eq!(recovery.explanation(), Some(BUILD_MISMATCH));
        recovery.connected(&options, &hello("current", "old-version"));
        assert!(recovery.panes_allowed());
        assert!(
            recovery
                .explanation()
                .unwrap()
                .contains("Running programs will end")
        );
    }

    #[test]
    fn dismissed_notices_stay_dismissed_until_the_problem_changes() {
        let mut notices = Notices::default();
        notices.push(None, "/config.toml:3: bad\n value".into());
        notices.push(
            Some("local-config".into()),
            "/config.toml:3: bad value".into(),
        );
        assert_eq!(notices.visible().count(), 1);
        notices.push(
            Some("local-config".into()),
            "/config.toml:4: bad value".into(),
        );
        assert_eq!(
            notices
                .visible()
                .map(|n| n.text.as_str())
                .collect::<Vec<_>>(),
            ["/config.toml:4: bad value"]
        );
        notices.dismiss(0);
        notices.push(None, "/config.toml:4: bad value".into());
        assert_eq!(notices.visible().count(), 0);
        notices.push(Some("theme".into()), "missing theme".into());
        notices.dismiss(1);
        notices.push(Some("theme".into()), "unreadable theme".into());
        assert_eq!(
            notices
                .visible()
                .map(|n| n.text.as_str())
                .collect::<Vec<_>>(),
            ["unreadable theme"]
        );
    }
}
