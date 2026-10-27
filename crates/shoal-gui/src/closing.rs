//! Close policy uses daemon metadata, never tab titles or guessed shell names.
use shoal_client::proto::Call;
use shoal_model::{PaneId, PaneStatus, SessionId, TabId, Workspace};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Pane(PaneId),
    Tab(TabId),
    Session(SessionId),
}

impl Target {
    pub fn from_call(call: &Call) -> Option<Self> {
        match *call {
            Call::PaneClose { pane } => Some(Self::Pane(pane)),
            Call::TabClose { tab } => Some(Self::Tab(tab)),
            Call::SessionClose { session } => Some(Self::Session(session)),
            _ => None,
        }
    }

    /// Resolve the original id again, so Enter cannot close a replacement target.
    pub fn call(self, workspace: &Workspace) -> Option<Call> {
        self.panes(workspace)?;
        Some(match self {
            Self::Pane(pane) => Call::PaneClose { pane },
            Self::Tab(tab) => Call::TabClose { tab },
            Self::Session(session) => Call::SessionClose { session },
        })
    }

    fn panes(self, workspace: &Workspace) -> Option<Vec<PaneId>> {
        let mut panes = Vec::new();
        match self {
            Self::Pane(pane) => panes.push(workspace.panes.get(&pane)?.id),
            Self::Tab(id) => workspace
                .sessions
                .iter()
                .flat_map(|s| &s.tabs)
                .find(|t| t.id == id)?
                .layout
                .for_each_pane(&mut |p| panes.push(p)),
            Self::Session(id) => {
                for tab in &workspace.sessions.iter().find(|s| s.id == id)?.tabs {
                    tab.layout.for_each_pane(&mut |p| panes.push(p));
                }
            }
        }
        Some(panes)
    }

    pub fn prompt(self, workspace: &Workspace, confirm_running: bool) -> Option<String> {
        let panes = self.panes(workspace)?;
        let mut programs = Vec::new();
        if confirm_running {
            for id in panes {
                if let Some(pane) = workspace.panes.get(&id)
                    && pane.status == PaneStatus::Running
                    && let Some(program) = pane.program.as_deref().filter(|p| !p.is_empty())
                    && !programs.contains(&program)
                {
                    programs.push(program);
                }
            }
        }
        let running = (!programs.is_empty()).then(|| {
            format!(
                "{} {} running.",
                programs.join(", "),
                if programs.len() == 1 { "is" } else { "are" }
            )
        });
        match self {
            Self::Pane(_) => running.map(|text| format!("{text} Close it?")),
            Self::Tab(_) => running.map(|text| format!("{text} Close tab?")),
            Self::Session(id) => {
                let name = &workspace.sessions.iter().find(|s| s.id == id)?.name;
                Some(match running {
                    Some(text) => format!("Close session {name}? {text}"),
                    None => format!("Close session {name}?"),
                })
            }
        }
    }
}

/// Kept with the exact target while keyboard and text input are intercepted.
pub struct Confirmation {
    pub target: Target,
    pub text: String,
}

pub fn exit_text(status: PaneStatus) -> Option<String> {
    match status {
        PaneStatus::Running => None,
        PaneStatus::Exited {
            signal: Some(signal),
            ..
        } => Some(format!("Process ended by signal {signal}.")),
        PaneStatus::Exited {
            code: Some(code), ..
        } => Some(format!("Process exited with code {code}.")),
        PaneStatus::Exited { .. } => Some("Process exited.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shoal_model::{Axis, GridSize, Layout, Pane, PaneKind, Session, SplitId, Tab};

    fn workspace() -> Workspace {
        let pane = |id, program: Option<&str>| Pane {
            id: PaneId::new(id),
            kind: PaneKind::Shell { argv: vec![] },
            size: GridSize { cols: 80, rows: 24 },
            cwd: None,
            program: program.map(str::to_owned),
            title: "nvim".into(),
            status: PaneStatus::Running,
            bell: false,
            restored_from: None,
        };
        let tab = |id, layout, focused| Tab {
            id: TabId::new(id),
            name: None,
            layout,
            focused: PaneId::new(focused),
            zoomed: None,
        };
        let now = "2026-10-26T00:00:00Z".parse().unwrap();
        Workspace {
            rev: 1,
            next_id: 10,
            sessions: vec![Session {
                id: SessionId::new(1),
                name: "work".into(),
                active_tab: TabId::new(2),
                created_at: now,
                last_active_at: now,
                windows: 1,
                tabs: vec![
                    tab(
                        2,
                        Layout::Split {
                            id: SplitId::new(3),
                            axis: Axis::Row,
                            ratio: 0.5,
                            first: Box::new(Layout::Pane(PaneId::new(4))),
                            second: Box::new(Layout::Pane(PaneId::new(5))),
                        },
                        4,
                    ),
                    tab(6, Layout::Pane(PaneId::new(7)), 7),
                ],
            }],
            panes: [pane(4, None), pane(5, Some("nvim")), pane(7, Some("less"))]
                .into_iter()
                .map(|p| (p.id, p))
                .collect(),
        }
    }

    #[test]
    fn running_rule_uses_program_and_status_and_obeys_config() {
        let mut ws = workspace();
        let target = Target::Pane(PaneId::new(5));
        assert_eq!(
            target.prompt(&ws, true).as_deref(),
            Some("nvim is running. Close it?")
        );
        assert_eq!(target.prompt(&ws, false), None);
        ws.panes.get_mut(&PaneId::new(5)).unwrap().program = Some("bash".into());
        assert_eq!(
            target.prompt(&ws, true).as_deref(),
            Some("bash is running. Close it?")
        );
        ws.panes.get_mut(&PaneId::new(5)).unwrap().program = Some(String::new());
        assert_eq!(target.prompt(&ws, true), None);
        ws.panes.get_mut(&PaneId::new(5)).unwrap().program = Some("nvim".into());
        // A shell's misleading title never triggers a confirmation.
        assert_eq!(Target::Pane(PaneId::new(4)).prompt(&ws, true), None);
        ws.panes.get_mut(&PaneId::new(5)).unwrap().status = PaneStatus::Exited {
            code: Some(7),
            signal: None,
        };
        assert_eq!(target.prompt(&ws, true), None);
    }

    #[test]
    fn tab_and_session_include_hidden_panes_and_keep_session_confirmation() {
        let mut ws = workspace();
        ws.sessions[0].tabs[0].zoomed = Some(PaneId::new(4));
        assert_eq!(
            Target::Tab(TabId::new(2)).prompt(&ws, true).as_deref(),
            Some("nvim is running. Close tab?")
        );
        let session = Target::Session(SessionId::new(1));
        assert_eq!(
            session.prompt(&ws, true).as_deref(),
            Some("Close session work? nvim, less are running.")
        );
        assert_eq!(
            session.prompt(&ws, false).as_deref(),
            Some("Close session work?")
        );
    }

    #[test]
    fn confirmation_keeps_the_original_target_and_drops_removed_targets() {
        let mut ws = workspace();
        let target = Target::Tab(TabId::new(2));
        ws.sessions[0].active_tab = TabId::new(6);
        assert_eq!(
            target.call(&ws),
            Some(Call::TabClose { tab: TabId::new(2) })
        );
        ws.sessions[0].tabs.remove(0);
        assert_eq!(target.call(&ws), None);
        assert_eq!(target.prompt(&ws, true), None);
    }

    #[test]
    fn retained_exit_status_distinguishes_failure_success_and_signal() {
        assert_eq!(exit_text(PaneStatus::Running), None);
        for (code, signal, expected) in [
            (Some(7), None, "Process exited with code 7."),
            (Some(0), None, "Process exited with code 0."),
            (None, Some(1), "Process ended by signal 1."),
            (None, None, "Process exited."),
        ] {
            assert_eq!(
                exit_text(PaneStatus::Exited { code, signal }).as_deref(),
                Some(expected)
            );
        }
    }
}
