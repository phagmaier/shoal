//! Keyboard dispatch, clipboard input and terminal focus reports (§7.4).

use gpui::{ClipboardItem, Context, KeyDownEvent, KeyUpEvent, Window};
use shoal_client::{
    model::{PaneId, Rect},
    proto::Call,
};
use shoal_config::{Action, XdgPaths};
use shoal_term::{KeyAction, TermError};

use super::ShoalWindow;
use crate::{
    actions,
    input::InputKey,
    keys,
    mirror::{Mirror, State},
};

impl ShoalWindow {
    pub(super) fn mirror(&mut self, pane: PaneId) -> Option<&mut Mirror> {
        let State::Live(mirror) = &mut self.panes.get_mut(pane)?.state else {
            return None;
        };
        Some(mirror)
    }

    pub(super) fn send_input(&self, pane: PaneId, encoded: Result<Vec<u8>, TermError>) {
        match encoded {
            Ok(bytes) if !bytes.is_empty() => self.client.input(pane, bytes),
            Ok(_) => {}
            Err(error) => tracing::warn!(%pane, %error, "can't encode terminal input"),
        }
    }

    pub(super) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_confirmation.is_some() {
            self.close_key(event, window, cx);
            return;
        }
        if !self.available
            || self.awaiting_workspace
            || self.rename.is_some()
            || self.switcher.is_some()
            || self.session_busy
        {
            return;
        }
        if let Some(action) =
            keys::resolve(&event.keystroke, self.held_modifiers, &self.config.keybinds)
        {
            self.dispatch(action, window, cx);
            cx.stop_propagation();
            return;
        }
        // The platform owns composition. Only a committed string may enter
        // the PTY; let GPUI deliver its key_char through the input handler.
        if self.preedit.is_some() {
            return;
        }
        let Some(pane) = self.focused_pane() else {
            return;
        };
        let input = InputKey::new(
            &event.keystroke,
            self.held_modifiers,
            self.caps_lock,
            if event.is_held {
                KeyAction::Repeat
            } else {
                KeyAction::Press
            },
        );
        if let Some(mirror) = self.mirror(pane) {
            if let Err(error) = mirror.scroll(shoal_term::Scroll::Bottom) {
                tracing::warn!(%error, "can't return to terminal input");
            }
            let bytes = mirror.key(&input.event());
            self.send_input(pane, bytes);
            self.pressed
                .insert(keys::unshift(&event.keystroke.key).to_owned(), pane);
        }
        self.clear_selection();
        cx.notify();
        cx.stop_propagation();
    }

    pub(super) fn key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.pressed.remove(keys::unshift(&event.keystroke.key)) else {
            return;
        };
        let input = InputKey::new(
            &event.keystroke,
            self.held_modifiers,
            self.caps_lock,
            KeyAction::Release,
        );
        if let Some(mirror) = self.mirror(pane) {
            let bytes = mirror.key(&input.event());
            self.send_input(pane, bytes);
        }
        cx.stop_propagation();
    }

    fn dispatch(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Action::SessionSwitcher => self.toggle_switcher(window, cx),
            Action::SessionNext | Action::SessionPrev => {
                if let Some(target) = crate::switcher::adjacent(
                    &self.workspace.sessions,
                    self.session,
                    action == Action::SessionNext,
                ) {
                    self.switch_session(target, window, cx);
                }
            }
            Action::WindowNew => self.new_window(cx),
            Action::TabRename => {
                if let Some(tab) = self.active_tab() {
                    self.start_rename(tab.id, window, cx);
                }
            }
            Action::ClipboardCopy => {
                if let Some(pane) = self.selection.as_ref().map(|s| s.pane)
                    && let Some(mirror) = self.mirror(pane)
                    && let Ok(text) = mirror.selection_text()
                    && !text.is_empty()
                {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            Action::ClipboardPaste => {
                // Linux GPUI reads synchronously (including through its
                // async API). Send now so later typing cannot overtake it.
                if let Some(pane) = self.focused_pane()
                    && let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
                {
                    self.paste(pane, &text);
                    cx.notify();
                }
            }
            Action::ConfigEdit => self.edit_config(cx),
            Action::ScrollPageUp
            | Action::ScrollPageDown
            | Action::ScrollTop
            | Action::ScrollBottom
            | Action::ScrollPrevPrompt
            | Action::ScrollNextPrompt => {
                if let Some(pane) = self.focused_pane()
                    && let Some(mirror) = self.mirror(pane)
                {
                    let result = match action {
                        Action::ScrollPageUp => mirror.scroll_page(-1),
                        Action::ScrollPageDown => mirror.scroll_page(1),
                        Action::ScrollTop => mirror.scroll(shoal_term::Scroll::Top),
                        Action::ScrollBottom => mirror.scroll(shoal_term::Scroll::Bottom),
                        Action::ScrollPrevPrompt => mirror.scroll(shoal_term::Scroll::Prompts(-1)),
                        _ => mirror.scroll(shoal_term::Scroll::Prompts(1)),
                    };
                    if let Err(error) = result {
                        tracing::warn!(%error, "can't scroll terminal");
                    }
                    cx.notify();
                }
            }
            _ => {
                let viewport = window.viewport_size();
                let pad = self.config.window.padding;
                let width = f32::from(viewport.width).max(0.0) as u32;
                let height = (f32::from(viewport.height).max(0.0) as u32)
                    .saturating_sub(crate::chrome::TOP_BAR_HEIGHT)
                    .saturating_sub(self.close_bar_height())
                    .saturating_sub(self.banner_height);
                let area = Rect {
                    x: pad.min(width),
                    y: pad.min(height) + self.banner_height,
                    width: width.saturating_sub(pad.saturating_mul(2)),
                    height: height.saturating_sub(pad.saturating_mul(2)),
                };
                if let Some(session) = self
                    .workspace
                    .sessions
                    .iter()
                    .find(|s| s.id == self.session)
                    && let Some(call) = actions::call(action, session, area, self.config.window.gap)
                {
                    self.send_action(call, window, cx);
                }
            }
        }
    }

    pub(super) fn paste(&mut self, pane: PaneId, text: &str) {
        if let Some(mirror) = self.mirror(pane) {
            if let Err(error) = mirror.scroll(shoal_term::Scroll::Bottom) {
                tracing::warn!(%error, "can't return to terminal input");
            }
            let bytes = mirror.paste(text);
            self.send_input(pane, bytes);
        }
        self.clear_selection();
    }

    pub(crate) fn sync_terminal_focus(&mut self) {
        let desired = (self.window_active
            && self.rename.is_none()
            && self.switcher.is_none()
            && self.close_confirmation.is_none())
        .then(|| self.focused_pane())
        .flatten();
        if desired == self.terminal_focus {
            return;
        }
        if let Some(old) = self.terminal_focus.take()
            && let Some(mirror) = self.mirror(old)
        {
            let bytes = mirror.focus(false);
            self.send_input(old, bytes);
        }
        if let Some(pane) = desired
            && let Some(mirror) = self.mirror(pane)
        {
            let bytes = mirror.focus(true);
            self.send_input(pane, bytes);
            self.terminal_focus = Some(pane);
        }
        self.preedit = None;
    }

    fn edit_config(&self, cx: &mut Context<Self>) {
        let Ok(paths) = XdgPaths::from_env() else {
            return;
        };
        let file = paths.config_file();
        let directory = paths.config_dir();
        let session = self.session;
        let create = cx
            .background_executor()
            .spawn(async move { std::fs::create_dir_all(directory) });
        cx.spawn(async move |view, cx| {
            if let Err(error) = create.await {
                tracing::warn!(%error, "can't create config directory");
                return;
            }
            // The shell expands EDITOR and its options in the pane's
            // environment. The path is positional data, never shell source.
            view.update(cx, |view, _| {
                drop(view.client.send(Call::TabCreate {
                    session,
                    cwd: None,
                    index: None,
                    argv: Some(vec![
                        "/bin/sh".into(),
                        "-c".into(),
                        "exec ${EDITOR:-vi} \"$1\"".into(),
                        "shoal-config".into(),
                        file.to_string_lossy().into_owned(),
                    ]),
                }))
            })
            .ok();
        })
        .detach();
    }
}
