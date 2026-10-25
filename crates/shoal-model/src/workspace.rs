//! The workspace tree (M1 spec §3): sessions › tabs › layouts of panes.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{PaneId, SessionId, SplitId, TabId};

/// A point in time, serialized as an RFC 3339 UTC string. The model never reads
/// the clock: callers pass the current time in.
pub type Timestamp = jiff::Timestamp;

/// Everything the daemon owns. Clients receive full copies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Workspace {
    /// Incremented on every change.
    pub rev: u64,
    /// The next id to hand out, of any kind. Ids are never reused.
    pub next_id: u64,
    /// In the user's order.
    pub sessions: Vec<Session>,
    /// Every pane in any tab's layout, keyed by its id.
    pub panes: BTreeMap<PaneId, Pane>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Session {
    pub id: SessionId,
    /// Unique; the first is `main`, then `2`, `3`, ...
    pub name: String,
    /// Never empty: closing the last tab closes the session.
    pub tabs: Vec<Tab>,
    pub active_tab: TabId,
    pub created_at: Timestamp,
    pub last_active_at: Timestamp,
    /// Runtime only, not persisted: windows showing this session now.
    pub windows: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Tab {
    pub id: TabId,
    /// Set by the user; `None` means an automatic title.
    pub name: Option<String>,
    pub layout: Layout,
    pub focused: PaneId,
    pub zoomed: Option<PaneId>,
}

/// A binary tree of splits with panes at the leaves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Pane(PaneId),
    Split {
        id: SplitId,
        axis: Axis,
        /// Share of `first`, clamped to [`Layout::RATIO_RANGE`].
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    /// Side by side.
    Row,
    /// Stacked.
    Column,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Pane {
    pub id: PaneId,
    pub kind: PaneKind,
    pub size: GridSize,
    pub cwd: Option<PathBuf>,
    /// Foreground process name, e.g. `nvim`.
    pub program: Option<String>,
    /// Computed display title (M1 spec §4.3).
    pub title: String,
    pub status: PaneStatus,
    /// Rang since it was last focused.
    pub bell: bool,
    pub restored_from: Option<Timestamp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaneKind {
    /// An empty `argv` runs the user's shell.
    Shell { argv: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PaneStatus {
    Running,
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

impl Layout {
    /// The allowed share of a split's `first` child.
    pub const RATIO_RANGE: RangeInclusive<f32> = 0.1..=0.9;

    /// Calls `f` for each pane, first subtree before second.
    pub fn for_each_pane(&self, f: &mut impl FnMut(PaneId)) {
        match self {
            Layout::Pane(id) => f(*id),
            Layout::Split { first, second, .. } => {
                first.for_each_pane(f);
                second.for_each_pane(f);
            }
        }
    }

    /// Calls `f` for each split, parent before children.
    pub fn for_each_split(&self, f: &mut impl FnMut(SplitId)) {
        if let Layout::Split {
            id, first, second, ..
        } = self
        {
            f(*id);
            first.for_each_split(f);
            second.for_each_split(f);
        }
    }

    pub fn contains_pane(&self, pane: PaneId) -> bool {
        match self {
            Layout::Pane(id) => *id == pane,
            Layout::Split { first, second, .. } => {
                first.contains_pane(pane) || second.contains_pane(pane)
            }
        }
    }
}
