//! Workspace edits and close cascades, always completed before publication.

use shoal_model::{Layout, PaneId, SessionId, SplitId, TabId, Timestamp};
use shoal_proto::RpcError;

use super::{
    Actor, ConnectionId,
    spawning::{missing, tab_index},
};

impl Actor {
    pub(super) fn session_index(&self, id: SessionId) -> Option<usize> {
        self.workspace
            .sessions
            .iter()
            .position(|session| session.id == id)
    }

    pub(super) fn tab_location(&self, id: TabId) -> Option<(usize, usize)> {
        self.workspace
            .sessions
            .iter()
            .enumerate()
            .find_map(|(si, session)| {
                session
                    .tabs
                    .iter()
                    .position(|tab| tab.id == id)
                    .map(|ti| (si, ti))
            })
    }

    pub(super) fn pane_location(&self, id: PaneId) -> Option<(usize, usize)> {
        self.workspace
            .sessions
            .iter()
            .enumerate()
            .find_map(|(si, session)| {
                session
                    .tabs
                    .iter()
                    .position(|tab| tab.layout.contains_pane(id))
                    .map(|ti| (si, ti))
            })
    }

    pub(super) fn split_location(&self, id: SplitId) -> Option<(usize, usize)> {
        self.workspace
            .sessions
            .iter()
            .enumerate()
            .find_map(|(si, session)| {
                session
                    .tabs
                    .iter()
                    .position(|tab| contains_split(&tab.layout, id))
                    .map(|ti| (si, ti))
            })
    }

    pub(super) fn show_session(
        &mut self,
        connection: ConnectionId,
        session: SessionId,
    ) -> Result<(), RpcError> {
        let si = self
            .session_index(session)
            .ok_or_else(|| missing("session", session))?;
        let state = self
            .connections
            .get_mut(&connection)
            .expect("invariant: connected request");
        let previous = state.shown.replace(session);
        state.reservations.clear();
        if previous != Some(session) {
            self.workspace.sessions[si].windows += 1;
            if let Some(previous) = previous
                && let Some(previous) = self.session_index(previous)
            {
                self.workspace.sessions[previous].windows -= 1;
            }
        }
        self.workspace.sessions[si].last_active_at = Timestamp::now();
        self.changed();
        Ok(())
    }

    pub(super) fn activate(&mut self, si: usize, ti: usize) {
        let session = &mut self.workspace.sessions[si];
        session.active_tab = session.tabs[ti].id;
        session.last_active_at = Timestamp::now();
        let focused = session.tabs[ti].focused;
        self.workspace
            .panes
            .get_mut(&focused)
            .expect("invariant: focused pane exists")
            .bell = false;
    }

    pub(super) fn move_tab(&mut self, tab: TabId, index: u64) -> Result<(), RpcError> {
        let (si, ti) = self.tab_location(tab).ok_or_else(|| missing("tab", tab))?;
        let tabs = &mut self.workspace.sessions[si].tabs;
        let index = tab_index(index, tabs.len(), false)?;
        if ti != index {
            let tab = tabs.remove(ti);
            tabs.insert(index, tab);
            self.changed();
        }
        Ok(())
    }

    pub(super) fn close_session(&mut self, si: usize) {
        let session = self.workspace.sessions.remove(si);
        for tab in session.tabs {
            tab.layout.for_each_pane(&mut |id| self.remove_pane(id));
        }
        for state in self.connections.values_mut() {
            if state.shown == Some(session.id) {
                state.shown = None;
            }
            state.reservations.remove(&session.id);
        }
    }

    pub(super) fn close_tab(&mut self, si: usize, ti: usize) {
        if self.workspace.sessions[si].tabs.len() == 1 {
            self.close_session(si);
            return;
        }
        let session = &mut self.workspace.sessions[si];
        let tab = session.tabs.remove(ti);
        if session.active_tab == tab.id {
            session.active_tab = session.tabs[ti.min(session.tabs.len() - 1)].id;
        }
        tab.layout.for_each_pane(&mut |id| self.remove_pane(id));
    }

    pub(super) fn close_pane(&mut self, pane: PaneId) {
        let Some((si, ti)) = self.pane_location(pane) else {
            return;
        };
        let remaining = shoal_model::close(&mut self.workspace.sessions[si].tabs[ti], pane)
            .expect("invariant: located pane exists");
        self.remove_pane(pane);
        if remaining.is_none() {
            self.close_tab(si, ti);
        }
    }

    fn remove_pane(&mut self, id: PaneId) {
        let running = self
            .workspace
            .panes
            .remove(&id)
            .is_some_and(|pane| pane.status == shoal_model::PaneStatus::Running);
        if let Some(runtime) = self.panes.remove(&id) {
            self.retire(runtime, !running);
        }
    }

    pub(super) fn retire(&mut self, runtime: shoal_daemon::pane::Pane, exited: bool) {
        runtime.kill();
        if !exited {
            self.retiring.insert(runtime.id(), runtime);
        }
    }
}

fn contains_split(layout: &Layout, split: SplitId) -> bool {
    match layout {
        Layout::Pane(_) => false,
        Layout::Split {
            id, first, second, ..
        } => *id == split || contains_split(first, split) || contains_split(second, split),
    }
}
