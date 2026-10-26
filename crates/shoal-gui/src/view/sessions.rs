//! Session RPCs and window launching. Replies wait off the GPUI thread.
use super::ShoalWindow;
use crate::{
    session,
    switcher::{Mode, Switcher},
};
use gpui::{Context, Window};
use shoal_client::{
    model::{SessionId, Workspace},
    proto::Call,
};

impl ShoalWindow {
    pub(super) fn toggle_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.switcher.is_some() {
            self.dismiss_switcher(window, cx);
        } else if !self.session_busy {
            self.finish_rename(None, window, cx);
            self.drag = None;
            self.ratio_timer = None;
            self.mouse_capture = None;
            self.selection = None;
            self.preedit = None;
            self.switcher = Some(Switcher::new(self.session));
            self.switcher_scroll = gpui::ScrollHandle::new();
            window.focus(&self.switcher_focus, cx);
            self.sync_terminal_focus();
            cx.notify();
        }
    }

    pub(super) fn dismiss_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switcher = None;
        window.focus(&self.focus, cx);
        self.sync_terminal_focus();
        cx.notify();
    }

    /// The request timeout belongs to shoal-client. Only its reply channel
    /// blocks here, on a background worker; completion runs on the main thread.
    fn session_rpc(
        &self,
        call: Call,
        window: &Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Result<serde_json::Value, String>, &mut Window, &mut Context<Self>)
        + 'static,
    ) {
        let reply = self.client.send(call);
        let wait = cx.background_executor().spawn(async move {
            reply
                .recv()
                .unwrap_or(Err(shoal_client::Error::Stopped))
                .map_err(|e| e.to_string())
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = wait.await;
            view.update_in(cx, |view, window, cx| done(view, result, window, cx))
                .ok();
        })
        .detach();
    }

    fn session_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.session_busy = false;
        if let Some(picker) = &mut self.switcher {
            picker.busy = false;
            picker.error = Some(error);
        } else {
            tracing::warn!(%error, "session action failed");
        }
        cx.notify();
    }

    pub(super) fn switch_session(
        &mut self,
        target: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session_busy {
            return;
        }
        if target == self.session {
            self.dismiss_switcher(window, cx);
            return;
        }
        self.session_busy = true;
        if let Some(picker) = &mut self.switcher {
            picker.busy = true;
            picker.error = None;
        }
        self.show_session(target, window, cx);
    }

    fn show_session(&mut self, target: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        self.session_rpc(
            session::show_call(target),
            window,
            cx,
            move |view, result, window, cx| {
                if let Err(error) = result {
                    view.session_error(error, cx);
                    return;
                }
                // A newly created session's coalesced notification may not have
                // arrived yet. Fetch state after show succeeds before changing the
                // local id, so an older notification cannot close the new view.
                view.session_rpc(
                    Call::WorkspaceGet(Default::default()),
                    window,
                    cx,
                    move |view, result, window, cx| {
                        let state = result.and_then(|value| {
                            serde_json::from_value::<Workspace>(value["workspace"].clone())
                                .map_err(|e| e.to_string())
                        });
                        match state {
                            Ok(state) => {
                                if view.revisions.accept(state.rev) {
                                    view.workspace = state;
                                }
                                if view.workspace.sessions.iter().any(|s| s.id == target) {
                                    view.finish_switch(target, window, cx);
                                    return;
                                }
                                view.session_error("Session closed".into(), cx);
                            }
                            Err(error) => view.session_error(error, cx),
                        }
                        // The daemon switched but local state could not follow. Keep
                        // its window declaration consistent with the view we retained.
                        drop(view.client.send(session::show_call(view.session)));
                    },
                );
            },
        );
    }

    fn finish_switch(&mut self, target: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        self.session = target;
        self.session_busy = false;
        self.rename = None;
        self.drag = None;
        self.ratio_timer = None;
        self.size_timer = None;
        self.sizer.reset();
        self.selection = None;
        self.preedit = None;
        self.input_cursor = None;
        self.mouse_capture = None;
        self.wheel_pending.clear();
        self.shown = Default::default();
        self.dismiss_switcher(window, cx);
        self.sync_panes(cx);
        self.report_focus(window);
        tracing::debug!(session = %target, "switched session");
        cx.notify();
    }

    pub(super) fn create_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session_busy {
            return;
        }
        self.session_busy = true;
        if let Some(picker) = &mut self.switcher {
            picker.busy = true;
            picker.error = None;
        }
        self.session_rpc(
            Call::SessionCreate {
                name: None,
                cwd: None,
            },
            window,
            cx,
            |view, result, window, cx| {
                let target = result.and_then(|value| {
                    serde_json::from_value::<SessionId>(value["session"].clone())
                        .map_err(|e| e.to_string())
                });
                match target {
                    Ok(target) => view.show_session(target, window, cx),
                    Err(error) => view.session_error(error, cx),
                }
            },
        );
    }

    pub(super) fn rename_session(
        &mut self,
        session: SessionId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session_busy {
            return;
        }
        self.session_busy = true;
        if let Some(picker) = &mut self.switcher {
            picker.busy = true;
            picker.error = None;
        }
        self.session_rpc(
            Call::SessionRename { session, name },
            window,
            cx,
            |view, result, _, cx| {
                view.session_busy = false;
                match result {
                    Err(error) => view.session_error(error, cx),
                    Ok(_) => {
                        if let Some(picker) = &mut view.switcher {
                            picker.busy = false;
                            picker.mode = Mode::Browse;
                            picker.query = crate::field::TextField::new("");
                            picker.error = None;
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    pub(super) fn close_session(
        &mut self,
        session: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session_busy {
            return;
        }
        self.session_busy = true;
        if let Some(picker) = &mut self.switcher {
            picker.busy = true;
            picker.error = None;
        }
        self.session_rpc(
            Call::SessionClose { session },
            window,
            cx,
            |view, result, _, cx| {
                view.session_busy = false;
                match result {
                    Err(error) => view.session_error(error, cx),
                    Ok(_) => {
                        if let Some(picker) = &mut view.switcher {
                            picker.busy = false;
                            picker.mode = Mode::Browse;
                            picker.error = None;
                        }
                    }
                }
                cx.notify();
            },
        );
    }

    pub(super) fn new_window(&mut self, cx: &mut Context<Self>) {
        // The child performs smart open on its own GUI connection, including
        // daemon-side reservations. Use the focused pane's directory.
        let cwd = self
            .focused_pane()
            .and_then(|p| self.workspace.panes.get(&p))
            .and_then(|p| p.cwd.clone());
        let launch = cx.background_executor().spawn(async move {
            let exe = std::env::current_exe()?;
            let mut command = std::process::Command::new(exe);
            if let Some(cwd) = cwd {
                command.current_dir(cwd);
            }
            shoal_client::lifecycle::detach(&mut command)
                .spawn()
                .map(|mut child| {
                    // Reap the child after its window closes. Never block GPUI.
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                })
        });
        cx.spawn(async move |_, _| {
            if let Err(error) = launch.await {
                tracing::warn!(%error, "can't open a window");
            }
        })
        .detach();
    }
}
