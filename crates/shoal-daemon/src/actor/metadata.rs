//! Apply coalesced pane-thread metadata; display policy lives with the workspace.

use std::{collections::BTreeSet, path::Path};

use shoal_config::schema::TabTitle;
use shoal_model::{PaneId, PaneStatus};

use super::{Actor, pane};

impl Actor {
    pub(super) fn metadata(&mut self, id: PaneId, value: pane::Metadata) {
        let focused = self.pane_is_focused(id);
        let Some(record) = self.workspace.panes.get_mut(&id) else {
            return;
        };
        let before = record.clone();
        record.cwd = value.cwd;
        record.program = (record.status == PaneStatus::Running)
            .then_some(value.program)
            .flatten();
        record.bell |= value.bell.is_some_and(|at| {
            !focused && self.last_pane_focus.get(&id).is_none_or(|&last| last < at)
        });
        match value.osc_title {
            Some(title) => {
                self.osc_titles.insert(id, title);
            }
            None => {
                self.osc_titles.remove(&id);
            }
        }
        let changed = *record != before;
        if self.refresh_metadata_titles() || changed {
            self.changed();
        }
    }

    pub(super) fn refresh_metadata_titles(&mut self) -> bool {
        let mut changed = false;
        for session in &self.workspace.sessions {
            for tab in &session.tabs {
                tab.layout.for_each_pane(&mut |id| {
                    if let Some(pane) = self.workspace.panes.get_mut(&id) {
                        let title = title(
                            tab.name.as_deref(),
                            pane.program.as_deref(),
                            pane.cwd.as_deref(),
                            self.osc_titles.get(&id).map(String::as_str),
                            self.settings.config.tabs.title,
                            &self.pane_settings.home,
                        );
                        changed |= title != pane.title;
                        pane.title = title;
                    }
                });
            }
        }
        changed
    }

    fn pane_is_focused(&self, id: PaneId) -> bool {
        self.workspace.sessions.iter().any(|session| {
            session
                .tabs
                .iter()
                .any(|tab| tab.id == session.active_tab && tab.focused == id)
                && self
                    .connections
                    .values()
                    .any(|window| window.focused && window.shown == Some(session.id))
        })
    }

    pub(super) fn clear_focused_bells(&mut self) {
        let focused: BTreeSet<_> = self
            .workspace
            .panes
            .keys()
            .copied()
            .filter(|&id| self.pane_is_focused(id))
            .collect();
        // Retain the end of a focus interval too: a bell received while the
        // pane was focused must stay suppressed after focus leaves it.
        let now = std::time::Instant::now();
        for &id in self.focused_panes.union(&focused) {
            self.last_pane_focus.insert(id, now);
        }
        for id in &focused {
            if let Some(pane) = self.workspace.panes.get_mut(id) {
                pane.bell = false;
            }
        }
        self.focused_panes = focused;
    }
}

fn title(
    name: Option<&str>,
    program: Option<&str>,
    cwd: Option<&Path>,
    osc: Option<&str>,
    mode: TabTitle,
    home: &Path,
) -> String {
    if mode == TabTitle::Program
        && let Some(title) = osc
    {
        return title.to_owned();
    }
    if let Some(title) = name.or(program) {
        return title.to_owned();
    }
    match cwd {
        Some(cwd) if cwd == home => "~".into(),
        Some(cwd) => cwd
            .file_name()
            .unwrap_or(cwd.as_os_str())
            .to_string_lossy()
            .into_owned(),
        None => String::new(),
    }
}
