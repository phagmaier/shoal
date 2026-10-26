//! `workspace.json`, format 1 (M1 spec §6.2). Separate from the live workspace
//! JSON: runtime-only fields stay out, and a format change bumps [`FORMAT`].

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use shoal_model::{GridSize, Layout, PaneId, PaneKind, SessionId, TabId, Timestamp, Workspace};

pub const FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Document {
    pub format: u32,
    pub saved_at: Timestamp,
    pub shoal_version: String,
    pub next_id: u64,
    pub sessions: Vec<SavedSession>,
    pub panes: BTreeMap<PaneId, SavedPane>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SavedSession {
    pub id: SessionId,
    pub name: String,
    pub created_at: Timestamp,
    pub last_active_at: Timestamp,
    pub active_tab: TabId,
    pub tabs: Vec<SavedTab>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SavedTab {
    pub id: TabId,
    pub name: Option<String>,
    pub focused: PaneId,
    pub zoomed: Option<PaneId>,
    pub layout: Layout,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SavedPane {
    pub kind: PaneKind,
    pub cwd: Option<PathBuf>,
    pub size: GridSize,
    pub program: Option<String>,
    /// Relative to the state folder; `None` when no scrollback file is saved.
    pub scrollback: Option<String>,
}

/// A pane's scrollback file, relative to the state folder.
pub fn scrollback_path(pane: PaneId) -> String {
    format!("panes/{pane}/scrollback.vt.zst")
}

impl Document {
    /// The persisted part of `workspace`. `has_scrollback` says which panes
    /// have a saved scrollback file to point at.
    pub fn new(
        workspace: &Workspace,
        saved_at: Timestamp,
        mut has_scrollback: impl FnMut(PaneId) -> bool,
    ) -> Self {
        Self {
            format: FORMAT,
            saved_at,
            shoal_version: env!("CARGO_PKG_VERSION").into(),
            next_id: workspace.next_id,
            sessions: workspace
                .sessions
                .iter()
                .map(|session| SavedSession {
                    id: session.id,
                    name: session.name.clone(),
                    created_at: session.created_at,
                    last_active_at: session.last_active_at,
                    active_tab: session.active_tab,
                    tabs: session
                        .tabs
                        .iter()
                        .map(|tab| SavedTab {
                            id: tab.id,
                            name: tab.name.clone(),
                            focused: tab.focused,
                            zoomed: tab.zoomed,
                            layout: tab.layout.clone(),
                        })
                        .collect(),
                })
                .collect(),
            panes: workspace
                .panes
                .iter()
                .map(|(&id, pane)| {
                    let saved = SavedPane {
                        kind: pane.kind.clone(),
                        cwd: pane.cwd.clone(),
                        size: pane.size,
                        program: pane.program.clone(),
                        scrollback: has_scrollback(id).then(|| scrollback_path(id)),
                    };
                    (id, saved)
                })
                .collect(),
        }
    }

    /// Equal apart from when it was saved.
    pub fn same_state(&self, other: &Self) -> bool {
        self.next_id == other.next_id
            && self.sessions == other.sessions
            && self.panes == other.panes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The example in M1 spec §6.2, verbatim.
    const SPEC_EXAMPLE: &str = r#"{
      "format": 1,
      "saved_at": "2026-10-24T14:02:11Z",
      "shoal_version": "0.1.0",
      "next_id": 57,
      "sessions": [{
        "id": "s1", "name": "main",
        "created_at": "2026-10-20T09:00:00Z", "last_active_at": "2026-10-24T14:01:58Z",
        "active_tab": "t2",
        "tabs": [{
          "id": "t2", "name": null, "focused": "p4", "zoomed": null,
          "layout": { "split": { "id": "x3", "axis": "row", "ratio": 0.6,
                      "first": { "pane": "p4" }, "second": { "pane": "p5" } } }
        }]
      }],
      "panes": {
        "p4": { "kind": { "kind": "shell", "argv": [] },
                "cwd": "/home/parker/code/shoal", "size": { "cols": 120, "rows": 40 },
                "program": "nvim", "scrollback": "panes/p4/scrollback.vt.zst" }
      }
    }"#;

    #[test]
    fn the_spec_example_round_trips_through_format_one() {
        let expected: serde_json::Value = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        let document: Document = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        assert_eq!(document.format, FORMAT);
        assert_eq!(
            document.panes[&PaneId::new(4)].scrollback.as_deref(),
            Some(scrollback_path(PaneId::new(4)).as_str())
        );
        // Through text, as the file is written: an f32 ratio prints as 0.6.
        let written = serde_json::to_string_pretty(&document).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&written).unwrap(),
            expected
        );
    }
}
