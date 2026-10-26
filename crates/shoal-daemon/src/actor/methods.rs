//! Typed dispatch; long-running work completes through actor messages.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use shoal_proto::{Call, CaptureFormat, DaemonStatusResult, RpcError, error_code};

use super::{Actor, ConnectionId, Message, Reply, Value, error, json, spawning::missing};
use tokio::sync::oneshot;

impl Actor {
    pub(super) fn request(
        &mut self,
        connection: ConnectionId,
        request: shoal_proto::Request,
        done: oneshot::Sender<()>,
    ) {
        let reply = Reply {
            connection,
            id: request.id.clone(),
            done,
        };
        let call = match request.call() {
            Ok(call) => call,
            Err(reason) => {
                self.finish(reply, Err(error(reason.code(), reason.to_string())));
                return;
            }
        };
        if let Call::SessionOpenSmart { .. } = &call {
            self.connections
                .get_mut(&connection)
                .expect("invariant: connected request")
                .reservations
                .clear();
            if let Some(session) = self.smart_session(connection) {
                self.finish(reply, Ok(json!({ "session": session })));
                return;
            }
        }
        if matches!(
            &call,
            Call::SessionOpenSmart { .. }
                | Call::SessionCreate { .. }
                | Call::TabCreate { .. }
                | Call::PaneSplit { .. }
        ) {
            match self.prepare_spawn(call) {
                Ok(prepared) => self.start_spawn(reply, prepared),
                Err(reason) => {
                    self.finish(reply, Err(reason));
                }
            }
            return;
        }
        if let Call::PaneCapture {
            pane,
            format,
            scrollback,
        } = call
        {
            let Some(runtime) = self.panes.get(&pane) else {
                self.finish(reply, Err(missing("pane", pane)));
                return;
            };
            let receiver = runtime.capture(
                match format {
                    CaptureFormat::Text => shoal_term::CaptureFormat::Text,
                    CaptureFormat::Vt => shoal_term::CaptureFormat::Vt,
                },
                scrollback,
            );
            let sender = self.sender.clone();
            tokio::spawn(async move {
                let result = match receiver.await {
                    Ok(Ok(bytes)) => tokio::task::spawn_blocking(move || {
                        let data = match format {
                            CaptureFormat::Text => String::from_utf8(bytes).map_err(|reason| {
                                error(error_code::INTERNAL, reason.to_string())
                            })?,
                            CaptureFormat::Vt => STANDARD.encode(bytes),
                        };
                        Ok(json!({ "data": data }))
                    })
                    .await
                    .unwrap_or_else(|reason| Err(error(error_code::INTERNAL, reason.to_string()))),
                    Ok(Err(reason)) => Err(error(error_code::INTERNAL, reason.to_string())),
                    Err(_) => Err(error(error_code::INTERNAL, "pane capture thread ended")),
                };
                let _ = sender.send(Message::Captured { reply, result });
            });
            return;
        }
        if let Call::PaneAttach { pane } = call {
            self.attach(reply, pane);
            return;
        }
        if let Call::PaneSetSize { pane, cols, rows } = call {
            self.set_size(reply, pane, cols, rows);
            return;
        }
        if let Call::DaemonPersist(_) = call {
            // Replies once everything is written (§5.4).
            self.flush(Some(reply));
            return;
        }
        let shutdown = matches!(&call, Call::DaemonShutdown(_));
        let result = self.execute(connection, call);
        let enqueued = self.finish(reply, result);
        if shutdown && enqueued {
            let _ = self.shutdown.send(true);
        }
    }

    fn execute(&mut self, connection: ConnectionId, call: Call) -> Result<Value, RpcError> {
        match call {
            Call::WorkspaceGet(_) => {
                return Ok(json!({ "rev": self.workspace.rev, "workspace": self.workspace }));
            }
            Call::WorkspaceSubscribe(_) => {
                let state = self
                    .connections
                    .get_mut(&connection)
                    .expect("invariant: connected request");
                state.subscribed = true;
                state.dirty = true;
            }
            Call::SessionRename { session, name } => {
                let si = self
                    .session_index(session)
                    .ok_or_else(|| missing("session", session))?;
                if self.name_taken(&name, Some(session)) {
                    return Err(error(
                        error_code::INVALID_STATE,
                        format!("session name {name:?} is taken"),
                    ));
                }
                if self.workspace.sessions[si].name != name {
                    self.workspace.sessions[si].name = name;
                    self.changed();
                }
            }
            Call::SessionClose { session } => {
                let si = self
                    .session_index(session)
                    .ok_or_else(|| missing("session", session))?;
                self.close_session(si);
                self.changed();
            }
            Call::WindowShowSession { session } => self.show_session(connection, session)?,
            Call::WindowSetFocused { focused } => {
                let state = self
                    .connections
                    .get_mut(&connection)
                    .expect("invariant: connected request");
                if focused {
                    self.size_owner = Some(connection);
                }
                let shown = state.shown;
                if state.focused != focused {
                    state.focused = focused;
                    self.clear_focused_bells();
                }
                if focused
                    && let Some(session) = shown
                    && let Some(si) = self.session_index(session)
                {
                    self.workspace.sessions[si].last_active_at = shoal_model::Timestamp::now();
                    self.changed();
                }
            }
            Call::TabClose { tab } => {
                let (si, ti) = self.tab_location(tab).ok_or_else(|| missing("tab", tab))?;
                self.close_tab(si, ti);
                self.changed();
            }
            Call::TabRename { tab, name } => {
                let (si, ti) = self.tab_location(tab).ok_or_else(|| missing("tab", tab))?;
                let target = &mut self.workspace.sessions[si].tabs[ti];
                if target.name != name {
                    target.name = name;
                    self.changed();
                }
            }
            Call::TabMove { tab, index } => self.move_tab(tab, index)?,
            Call::TabActivate { tab } => {
                let (si, ti) = self.tab_location(tab).ok_or_else(|| missing("tab", tab))?;
                self.activate(si, ti);
                self.changed();
            }
            Call::PaneClose { pane } => {
                if !self.workspace.panes.contains_key(&pane) {
                    return Err(missing("pane", pane));
                }
                self.close_pane(pane);
                self.changed();
            }
            Call::PaneFocus { pane } => {
                let (si, ti) = self
                    .pane_location(pane)
                    .ok_or_else(|| missing("pane", pane))?;
                let tab = &mut self.workspace.sessions[si].tabs[ti];
                tab.focused = pane;
                if tab.zoomed.is_some() {
                    tab.zoomed = Some(pane);
                }
                self.workspace
                    .panes
                    .get_mut(&pane)
                    .expect("invariant: layout pane exists")
                    .bell = false;
                self.activate(si, ti);
                self.changed();
            }
            Call::PaneResize {
                pane,
                direction,
                cells,
            } => self.resize_pane(pane, direction, cells)?,
            Call::SplitSetRatio { split, ratio } => {
                let (si, ti) = self
                    .split_location(split)
                    .ok_or_else(|| missing("split", split))?;
                if shoal_model::set_ratio(
                    &mut self.workspace.sessions[si].tabs[ti].layout,
                    split,
                    ratio,
                )
                .map_err(|reason| error(error_code::BAD_PARAMS, reason.to_string()))?
                {
                    self.changed();
                }
            }
            Call::PaneZoom { pane, zoomed } => {
                let (si, ti) = self
                    .pane_location(pane)
                    .ok_or_else(|| missing("pane", pane))?;
                let target = &mut self.workspace.sessions[si].tabs[ti];
                target.focused = pane;
                target.zoomed = zoomed.then_some(pane);
                self.workspace
                    .panes
                    .get_mut(&pane)
                    .expect("invariant: layout pane exists")
                    .bell = false;
                self.activate(si, ti);
                self.changed();
            }
            Call::PaneSendText { pane, text } => {
                let runtime = self.panes.get(&pane).ok_or_else(|| missing("pane", pane))?;
                runtime.write(text.into_bytes());
            }
            Call::DaemonStatus(_) => {
                return Ok(serde_json::to_value(DaemonStatusResult {
                    version: env!("CARGO_PKG_VERSION").into(),
                    pid: std::process::id(),
                    uptime_s: self.started.elapsed().as_secs(),
                    sessions: self.workspace.sessions.len() as u64,
                    panes: self.workspace.panes.len() as u64,
                    last_persist_at: self.persistence.last_persist_at,
                })
                .expect("invariant: daemon status is serializable"));
            }
            Call::DaemonShutdown(_) => {}
            Call::PaneAttach { .. } => unreachable!("attach dispatched first"),
            Call::PaneDetach { pane } => self.detach(connection, pane)?,
            Call::PaneSetSize { .. } => unreachable!("size dispatched first"),
            Call::DaemonPersist(_) => unreachable!("persist dispatched first"),
            Call::Hello { .. } => {
                return Err(error(error_code::INVALID_STATE, "hello was already sent"));
            }
            Call::SessionOpenSmart { .. }
            | Call::SessionCreate { .. }
            | Call::TabCreate { .. }
            | Call::PaneSplit { .. }
            | Call::PaneCapture { .. } => unreachable!("asynchronous calls dispatched first"),
        }
        Ok(json!({}))
    }
}
