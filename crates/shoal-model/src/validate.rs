//! The workspace invariants (M1 spec §3, `validate`).

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::{PaneId, SessionId, SplitId, TabId, Workspace};

/// An id of any kind. All kinds share one counter, so no two ids share a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnyId {
    Session(SessionId),
    Tab(TabId),
    Pane(PaneId),
    Split(SplitId),
}

impl AnyId {
    pub const fn get(self) -> u64 {
        match self {
            AnyId::Session(id) => id.get(),
            AnyId::Tab(id) => id.get(),
            AnyId::Pane(id) => id.get(),
            AnyId::Split(id) => id.get(),
        }
    }
}

impl fmt::Display for AnyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnyId::Session(id) => id.fmt(f),
            AnyId::Tab(id) => id.fmt(f),
            AnyId::Pane(id) => id.fmt(f),
            AnyId::Split(id) => id.fmt(f),
        }
    }
}

/// The first broken invariant `validate` found.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error("pane {id} is stored under key {key}")]
    PaneKeyMismatch { key: PaneId, id: PaneId },
    #[error("session {0} has no tabs")]
    EmptySession(SessionId),
    #[error("session {session} has active tab {tab}, which is not one of its tabs")]
    ActiveTabOutside { session: SessionId, tab: TabId },
    #[error("tab {tab} focuses pane {pane}, which is not in its layout")]
    FocusedOutside { tab: TabId, pane: PaneId },
    #[error("tab {tab} zooms pane {pane}, which is not in its layout")]
    ZoomedOutside { tab: TabId, pane: PaneId },
    #[error("pane {0} appears more than once in tab layouts")]
    PaneInManyPlaces(PaneId),
    #[error("pane {0} is in a tab layout but not in the workspace's panes")]
    UnknownPane(PaneId),
    #[error("pane {0} is in no tab layout")]
    PaneNotInLayout(PaneId),
    #[error("ids {first} and {second} share the number {}", first.get())]
    DuplicateId { first: AnyId, second: AnyId },
    #[error("id {id} is not below next_id {next_id}")]
    IdNotBelowNextId { id: AnyId, next_id: u64 },
}

/// Checks every workspace invariant and returns the first one broken.
pub fn validate(workspace: &Workspace) -> Result<(), ValidationError> {
    check_pane_keys(workspace)?;
    check_references(workspace)?;
    check_pane_membership(workspace)?;
    check_ids(workspace)
}

fn check_pane_keys(workspace: &Workspace) -> Result<(), ValidationError> {
    match workspace.panes.iter().find(|(key, pane)| **key != pane.id) {
        Some((key, pane)) => Err(ValidationError::PaneKeyMismatch {
            key: *key,
            id: pane.id,
        }),
        None => Ok(()),
    }
}

fn check_references(workspace: &Workspace) -> Result<(), ValidationError> {
    for session in &workspace.sessions {
        if session.tabs.is_empty() {
            return Err(ValidationError::EmptySession(session.id));
        }
        if !session.tabs.iter().any(|tab| tab.id == session.active_tab) {
            return Err(ValidationError::ActiveTabOutside {
                session: session.id,
                tab: session.active_tab,
            });
        }
        for tab in &session.tabs {
            if !tab.layout.contains_pane(tab.focused) {
                return Err(ValidationError::FocusedOutside {
                    tab: tab.id,
                    pane: tab.focused,
                });
            }
            if let Some(zoomed) = tab.zoomed.filter(|&pane| !tab.layout.contains_pane(pane)) {
                return Err(ValidationError::ZoomedOutside {
                    tab: tab.id,
                    pane: zoomed,
                });
            }
        }
    }
    Ok(())
}

/// Every pane in `panes` appears in exactly one tab's layout, and every layout
/// pane is in `panes`.
fn check_pane_membership(workspace: &Workspace) -> Result<(), ValidationError> {
    let mut placed = BTreeSet::new();
    let mut repeated = None;
    for tab in workspace.sessions.iter().flat_map(|session| &session.tabs) {
        tab.layout.for_each_pane(&mut |pane| {
            if !placed.insert(pane) {
                repeated.get_or_insert(pane);
            }
        });
    }
    if let Some(pane) = repeated {
        return Err(ValidationError::PaneInManyPlaces(pane));
    }
    if let Some(pane) = placed
        .iter()
        .find(|pane| !workspace.panes.contains_key(pane))
    {
        return Err(ValidationError::UnknownPane(*pane));
    }
    if let Some(pane) = workspace.panes.keys().find(|pane| !placed.contains(pane)) {
        return Err(ValidationError::PaneNotInLayout(*pane));
    }
    Ok(())
}

/// Ids are unique across all kinds and all below `next_id`. Runs after
/// membership, so the keys of `panes` are exactly the panes in layouts.
fn check_ids(workspace: &Workspace) -> Result<(), ValidationError> {
    let mut ids = Vec::new();
    for session in &workspace.sessions {
        ids.push(AnyId::Session(session.id));
        for tab in &session.tabs {
            ids.push(AnyId::Tab(tab.id));
            tab.layout
                .for_each_split(&mut |split| ids.push(AnyId::Split(split)));
        }
    }
    ids.extend(workspace.panes.keys().copied().map(AnyId::Pane));

    let mut seen = BTreeMap::new();
    for id in ids {
        if id.get() >= workspace.next_id {
            return Err(ValidationError::IdNotBelowNextId {
                id,
                next_id: workspace.next_id,
            });
        }
        match seen.entry(id.get()) {
            Entry::Vacant(slot) => {
                slot.insert(id);
            }
            Entry::Occupied(first) => {
                return Err(ValidationError::DuplicateId {
                    first: *first.get(),
                    second: id,
                });
            }
        }
    }
    Ok(())
}
