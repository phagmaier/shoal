//! Reserve identities and targets before moving blocking spawn work off-actor.

use std::path::PathBuf;

use shoal_model::{
    GridSize, Layout, PaneId, PaneKind, PaneStatus, Session, SessionId, SplitDirection, SplitId,
    Tab, TabId, Timestamp,
};
use shoal_proto::{Call, RpcError, error_code};

use super::{Actor, ConnectionId, Message, Reply, error, json, pane};

pub(super) struct Spawn {
    reply: Reply,
    record: shoal_model::Pane,
    target: Target,
}

pub(super) struct Prepared {
    record: shoal_model::Pane,
    target: Target,
}

enum Target {
    Session {
        session: SessionId,
        tab: TabId,
        name: String,
        smart: bool,
    },
    Tab {
        session: SessionId,
        tab: TabId,
        index: Option<usize>,
    },
    Split {
        source: PaneId,
        split: SplitId,
        direction: SplitDirection,
    },
}

const INITIAL_SIZE: GridSize = GridSize { cols: 80, rows: 24 };

impl Actor {
    pub(super) fn smart_session(&mut self, connection: ConnectionId) -> Option<SessionId> {
        let id = self
            .workspace
            .sessions
            .iter()
            .filter(|session| {
                session.windows == 0
                    && !self
                        .connections
                        .values()
                        .any(|state| state.reservations.contains(&session.id))
            })
            .max_by_key(|session| session.last_active_at)
            .map(|session| session.id)?;
        self.connections
            .get_mut(&connection)?
            .reservations
            .insert(id);
        Some(id)
    }

    pub(super) fn name_taken(&self, name: &str, except: Option<SessionId>) -> bool {
        self.workspace.sessions.iter().any(|session| Some(session.id) != except && session.name == name)
            || self.pending.values().any(|spawn| {
                self.connections.contains_key(&spawn.reply.connection)
                    && matches!(&spawn.target, Target::Session { name: reserved, .. } if reserved == name)
            })
    }

    fn default_name(&self) -> String {
        if self.workspace.sessions.is_empty() && !self.name_taken("main", None) {
            return "main".into();
        }
        (2_u64..)
            .map(|number| number.to_string())
            .find(|name| !self.name_taken(name, None))
            .expect("invariant: session name space is not exhausted")
    }

    fn allocate(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("invariant: workspace id space is not exhausted");
        id
    }

    pub(super) fn prepare_spawn(&mut self, call: Call) -> Result<Prepared, RpcError> {
        let (target, cwd, argv, size) = match call {
            Call::SessionOpenSmart { cwd } => {
                let name = self.default_name();
                let session = SessionId::new(self.allocate());
                let tab = TabId::new(self.allocate());
                (
                    Target::Session {
                        session,
                        tab,
                        name,
                        smart: true,
                    },
                    cwd,
                    Vec::new(),
                    INITIAL_SIZE,
                )
            }
            Call::SessionCreate { name, cwd } => {
                let name = name.unwrap_or_else(|| self.default_name());
                if self.name_taken(&name, None) {
                    return Err(error(
                        error_code::INVALID_STATE,
                        format!("session name {name:?} is taken"),
                    ));
                }
                let session = SessionId::new(self.allocate());
                let tab = TabId::new(self.allocate());
                (
                    Target::Session {
                        session,
                        tab,
                        name,
                        smart: false,
                    },
                    cwd,
                    Vec::new(),
                    INITIAL_SIZE,
                )
            }
            Call::TabCreate {
                session,
                cwd,
                index,
                argv,
            } => {
                let si = self
                    .session_index(session)
                    .ok_or_else(|| missing("session", session))?;
                let existing = &self.workspace.sessions[si];
                let index = index
                    .map(|index| tab_index(index, existing.tabs.len(), true))
                    .transpose()?;
                let source_tab = existing
                    .tabs
                    .iter()
                    .find(|tab| tab.id == existing.active_tab)
                    .expect("invariant: active tab belongs to session");
                let source = &self.workspace.panes[&source_tab.focused];
                let size = source.size;
                let cwd = cwd.or_else(|| source.cwd.clone());
                let tab = TabId::new(self.allocate());
                (
                    Target::Tab {
                        session,
                        tab,
                        index,
                    },
                    cwd,
                    argv.unwrap_or_default(),
                    size,
                )
            }
            Call::PaneSplit {
                pane,
                direction,
                cwd,
            } => {
                let source = self
                    .workspace
                    .panes
                    .get(&pane)
                    .ok_or_else(|| missing("pane", pane))?;
                let size = source.size;
                let cwd = cwd.or_else(|| source.cwd.clone());
                let split = SplitId::new(self.allocate());
                (
                    Target::Split {
                        source: pane,
                        split,
                        direction,
                    },
                    cwd,
                    Vec::new(),
                    size,
                )
            }
            _ => unreachable!("only spawning calls reach prepare_spawn"),
        };
        let id = PaneId::new(self.allocate());
        Ok(Prepared {
            record: shoal_model::Pane {
                id,
                kind: PaneKind::Shell { argv },
                size,
                cwd,
                program: None,
                title: String::new(),
                status: PaneStatus::Running,
                bell: false,
                restored_from: None,
            },
            target,
        })
    }

    pub(super) fn start_spawn(&mut self, reply: Reply, prepared: Prepared) {
        let id = prepared.record.id;
        let PaneKind::Shell { argv } = &prepared.record.kind;
        let spec = pane::PaneSpec {
            id,
            size: prepared.record.size,
            argv: argv.clone(),
            cwd: prepared.record.cwd.clone(),
            window_env: self.window_env.clone(),
        };
        self.pending.insert(
            id,
            Spawn {
                reply,
                record: prepared.record,
                target: prepared.target,
            },
        );
        let settings = self.pane_settings.clone();
        let events = self.events.clone();
        let sender = self.sender.clone();
        let shutdown_cutoff = self.shutdown_cutoff.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                if shutdown_cutoff.get().is_some() {
                    return Err("daemon is stopping".into());
                }
                let cwd = pane::spawn::directory(spec.cwd.as_deref(), &settings.home);
                if shutdown_cutoff.get().is_some() {
                    return Err("daemon is stopping".into());
                }
                pane::spawn(spec, &settings, events)
                    .map(|pane| {
                        if let Some(&cutoff) = shutdown_cutoff.get() {
                            // A late worker must apply even an expired cutoff
                            // before returning its child to a stopped actor.
                            let _ = pane.kill_by(cutoff).blocking_recv();
                        }
                        (pane, cwd)
                    })
                    .map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|error| Err(format!("pane spawn worker failed: {error}")));
            // In-flight work applies the shared cutoff before delivering a pane;
            // dropping an undeliverable result also closes its command channel.
            let _ = sender.send(Message::Spawned { pane: id, result });
        });
    }

    pub(super) fn spawned(&mut self, id: PaneId, result: Result<(pane::Pane, PathBuf), String>) {
        let Some(mut spawn) = self.pending.remove(&id) else {
            return;
        };
        let events = self.early_events.remove(&id).unwrap_or_default();
        let (runtime, cwd) = match result {
            Ok(result) => result,
            Err(reason) => {
                self.finish(spawn.reply, Err(error(error_code::INTERNAL, reason)));
                return;
            }
        };
        let exited = events
            .iter()
            .any(|e| matches!(e, pane::Event::Exited { .. }));
        if !self.connections.contains_key(&spawn.reply.connection) {
            self.retire(runtime, exited);
            return;
        }
        spawn.record.cwd = Some(cwd);
        let result = self.commit_spawn(&spawn);
        match result {
            Ok(result) => {
                runtime.apply_settings(
                    self.pane_settings.theme,
                    self.pane_settings.scrollback,
                    self.pane_settings.persist,
                );
                self.panes.insert(id, runtime);
                self.workspace.panes.insert(id, spawn.record);
                self.changed();
                for event in events {
                    self.event(id, event);
                }
                self.finish(spawn.reply, Ok(result));
            }
            Err(reason) => {
                self.retire(runtime, exited);
                self.finish(spawn.reply, Err(reason));
            }
        }
    }

    fn commit_spawn(&mut self, spawn: &Spawn) -> Result<serde_json::Value, RpcError> {
        let id = spawn.record.id;
        match &spawn.target {
            Target::Session {
                session,
                tab,
                name,
                smart,
            } => {
                let now = Timestamp::now();
                self.workspace.sessions.push(Session {
                    id: *session,
                    name: name.clone(),
                    tabs: vec![new_tab(*tab, id)],
                    active_tab: *tab,
                    created_at: now,
                    last_active_at: now,
                    windows: 0,
                });
                if *smart {
                    self.connections
                        .get_mut(&spawn.reply.connection)
                        .expect("invariant: connection checked before commit")
                        .reservations
                        .insert(*session);
                }
                Ok(json!({ "session": session }))
            }
            Target::Tab {
                session,
                tab,
                index,
            } => {
                let si = self
                    .session_index(*session)
                    .ok_or_else(|| missing("session", session))?;
                let session = &mut self.workspace.sessions[si];
                let index = index.unwrap_or(session.tabs.len());
                if index > session.tabs.len() {
                    return Err(error(
                        error_code::INVALID_STATE,
                        "tab insertion index no longer exists",
                    ));
                }
                session.tabs.insert(index, new_tab(*tab, id));
                session.active_tab = *tab;
                session.last_active_at = Timestamp::now();
                Ok(json!({ "tab": tab, "pane": id }))
            }
            Target::Split {
                source,
                split,
                direction,
            } => {
                let (si, ti) = self
                    .pane_location(*source)
                    .ok_or_else(|| missing("pane", source))?;
                let session = &mut self.workspace.sessions[si];
                let tab = &mut session.tabs[ti];
                shoal_model::split(tab, *source, *direction, id, *split)
                    .map_err(|reason| error(error_code::INVALID_STATE, reason.to_string()))?;
                session.active_tab = tab.id;
                session.last_active_at = Timestamp::now();
                Ok(json!({ "pane": id }))
            }
        }
    }
}

fn new_tab(id: TabId, pane: PaneId) -> Tab {
    Tab {
        id,
        name: None,
        layout: Layout::Pane(pane),
        focused: pane,
        zoomed: None,
    }
}

pub(super) fn missing(kind: &str, id: impl std::fmt::Display) -> RpcError {
    error(error_code::NOT_FOUND, format!("{kind} {id} was not found"))
}

pub(super) fn tab_index(index: u64, len: usize, insertion: bool) -> Result<usize, RpcError> {
    let converted = usize::try_from(index).ok();
    converted
        .filter(|&index| if insertion { index <= len } else { index < len })
        .ok_or_else(|| {
            error(
                error_code::INVALID_STATE,
                format!("tab index {index} is out of range"),
            )
        })
}
