//! M1 methods and result/notification payloads (spec §5.3–5.5).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use shoal_model::{
    Direction, PaneId, SessionId, SplitDirection, SplitId, TabId, Timestamp, Workspace,
};

/// Parameters of known methods. Conversion from a Request classifies unknown
/// method names separately from bad parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Call {
    #[serde(rename = "hello")]
    Hello {
        protocol: u32,
        client: ClientKind,
        version: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        core: Option<String>,
        env: BTreeMap<String, String>,
    },
    #[serde(rename = "workspace.get")]
    WorkspaceGet(Empty),
    #[serde(rename = "workspace.subscribe")]
    WorkspaceSubscribe(Empty),
    #[serde(rename = "session.open_smart")]
    SessionOpenSmart {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
    },
    #[serde(rename = "session.create")]
    SessionCreate {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
    },
    #[serde(rename = "session.rename")]
    SessionRename { session: SessionId, name: String },
    #[serde(rename = "session.close")]
    SessionClose { session: SessionId },
    #[serde(rename = "window.show_session")]
    WindowShowSession { session: SessionId },
    #[serde(rename = "window.set_focused")]
    WindowSetFocused { focused: bool },
    #[serde(rename = "tab.create")]
    TabCreate {
        session: SessionId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        index: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        argv: Option<Vec<String>>,
    },
    #[serde(rename = "tab.close")]
    TabClose { tab: TabId },
    #[serde(rename = "tab.rename")]
    TabRename {
        tab: TabId,
        // Required, but null clears a name. Missing is not a rename operation.
        #[serde(deserialize_with = "Option::deserialize")]
        name: Option<String>,
    },
    #[serde(rename = "tab.move")]
    TabMove { tab: TabId, index: u64 },
    #[serde(rename = "tab.activate")]
    TabActivate { tab: TabId },
    #[serde(rename = "pane.split")]
    PaneSplit {
        pane: PaneId,
        direction: SplitDirection,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
    },
    #[serde(rename = "pane.close")]
    PaneClose { pane: PaneId },
    #[serde(rename = "pane.focus")]
    PaneFocus { pane: PaneId },
    #[serde(rename = "pane.resize")]
    PaneResize {
        pane: PaneId,
        direction: Direction,
        cells: u64,
    },
    #[serde(rename = "split.set_ratio")]
    SplitSetRatio {
        split: SplitId,
        #[serde(with = "finite_ratio")]
        ratio: f32,
    },
    #[serde(rename = "pane.zoom")]
    PaneZoom { pane: PaneId, zoomed: bool },
    #[serde(rename = "pane.attach")]
    PaneAttach { pane: PaneId },
    #[serde(rename = "pane.detach")]
    PaneDetach { pane: PaneId },
    #[serde(rename = "pane.set_size")]
    PaneSetSize { pane: PaneId, cols: u16, rows: u16 },
    #[serde(rename = "pane.send_text")]
    PaneSendText { pane: PaneId, text: String },
    #[serde(rename = "pane.capture")]
    PaneCapture {
        pane: PaneId,
        format: CaptureFormat,
        scrollback: bool,
    },
    #[serde(rename = "daemon.status")]
    DaemonStatus(Empty),
    #[serde(rename = "daemon.persist")]
    DaemonPersist(Empty),
    #[serde(rename = "daemon.shutdown")]
    DaemonShutdown(Empty),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct Empty {}

impl<'de> Deserialize<'de> for Empty {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let object = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;
        if object.is_empty() {
            Ok(Self {})
        } else {
            Err(serde::de::Error::custom("expected an empty object"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Gui,
    Cli,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureFormat {
    Text,
    Vt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HelloResult {
    pub protocol: u32,
    pub daemon_version: String,
    pub core: String,
    pub pid: u32,
}

/// Both workspace.get results and workspace.changed parameters have this shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkspaceState {
    pub rev: u64,
    pub workspace: Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SessionResult {
    pub session: SessionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TabCreateResult {
    pub tab: TabId,
    pub pane: PaneId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PaneResult {
    pub pane: PaneId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AttachResult {
    pub seq: u64,
    pub cols: u16,
    pub rows: u16,
}

pub type SizeResult = shoal_model::GridSize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CaptureResult {
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DaemonStatusResult {
    pub version: String,
    pub pid: u32,
    pub uptime_s: u64,
    pub sessions: u64,
    pub panes: u64,
    pub last_persist_at: Option<Timestamp>,
}

/// Only the three M1 notifications; pane.event remains reserved for M3.
/// Wrap in RawNotification for a complete jsonrpc envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Notification {
    #[serde(rename = "workspace.changed")]
    WorkspaceChanged(WorkspaceState),
    #[serde(rename = "daemon.notice")]
    DaemonNotice {
        level: NoticeLevel,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },
    #[serde(rename = "pane.detached")]
    PaneDetached { pane: PaneId, reason: DetachReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetachReason {
    MidSequence,
}

impl crate::RawNotification {
    pub fn from_notification(notification: &Notification) -> Result<Self, serde_json::Error> {
        #[derive(Deserialize)]
        struct Parts {
            method: String,
            params: serde_json::Value,
        }
        let parts: Parts = serde_json::from_value(serde_json::to_value(notification)?)?;
        Ok(Self {
            method: parts.method,
            params: parts.params,
        })
    }

    pub fn notification(&self) -> Result<Notification, serde_json::Error> {
        if !self.params.is_object() {
            return Err(serde::de::Error::custom("parameters must be an object"));
        }
        serde_json::from_value(serde_json::json!({"method": self.method, "params": self.params}))
    }
}

impl Call {
    /// Dispatch names, checked before parameter deserialization so even unknown
    /// methods with malformed parameters receive -32601 rather than 1004.
    pub const METHOD_NAMES: &[&str] = &[
        "hello",
        "workspace.get",
        "workspace.subscribe",
        "session.open_smart",
        "session.create",
        "session.rename",
        "session.close",
        "window.show_session",
        "window.set_focused",
        "tab.create",
        "tab.close",
        "tab.rename",
        "tab.move",
        "tab.activate",
        "pane.split",
        "pane.close",
        "pane.focus",
        "pane.resize",
        "split.set_ratio",
        "pane.zoom",
        "pane.attach",
        "pane.detach",
        "pane.set_size",
        "pane.send_text",
        "pane.capture",
        "daemon.status",
        "daemon.persist",
        "daemon.shutdown",
    ];
}

// JSON has no nonfinite numbers; serde's default f32 conversion can overflow a
// valid f64 to infinity, and its serializer would silently turn NaN into null.
mod finite_ratio {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(ratio: &f32, serializer: S) -> Result<S::Ok, S::Error> {
        if ratio.is_finite() {
            serializer.serialize_f32(*ratio)
        } else {
            Err(serde::ser::Error::custom("ratio must be finite"))
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
        let ratio = f32::deserialize(deserializer)?;
        if ratio.is_finite() {
            Ok(ratio)
        } else {
            Err(serde::de::Error::custom("ratio must be finite"))
        }
    }
}
