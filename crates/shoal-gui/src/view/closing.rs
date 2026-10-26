//! Inline close confirmation and retained exited-pane status (§7.7, §4.2).
use super::{ShoalWindow, rgba};
use crate::{
    chrome::{Color, PaneBox},
    closing::{self, Confirmation, Target},
};
use gpui::{AnyElement, Context, KeyDownEvent, SharedString, Window, div, prelude::*, px};
use shoal_client::proto::Call;

impl ShoalWindow {
    pub(super) fn close_bar_height(&self) -> u32 {
        if self.close_confirmation.is_some() {
            36
        } else {
            0
        }
    }
    pub(super) fn send_action(&mut self, call: Call, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = Target::from_call(&call) {
            self.request_close(target, window, cx);
        } else {
            drop(self.client.send(call));
        }
    }

    pub(super) fn request_close(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.available
            || self.awaiting_workspace
            || self.session_busy
            || self.switcher.is_some()
            || self.close_confirmation.is_some()
        {
            return;
        }
        let Some(call) = target.call(&self.workspace) else {
            return;
        };
        if let Some(text) =
            target.prompt(&self.workspace, self.config.behavior.confirm_close_running)
        {
            self.finish_rename(None, window, cx);
            self.drag = None;
            self.ratio_timer = None;
            self.mouse_capture = None;
            self.clear_selection();
            self.preedit = None;
            self.close_confirmation = Some(Confirmation { target, text });
            window.focus(&self.focus, cx);
            self.sync_terminal_focus();
            cx.notify();
        } else {
            drop(self.client.send(call));
        }
    }

    pub(super) fn close_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => self.cancel_close(window, cx),
            "enter" if !event.is_held => {
                if let Some(pending) = self.close_confirmation.take()
                    && let Some(call) = pending.target.call(&self.workspace)
                {
                    drop(self.client.send(call));
                }
                self.cancel_close(window, cx);
            }
            _ => {}
        }
        cx.stop_propagation();
    }

    pub(super) fn cancel_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_confirmation = None;
        window.focus(&self.focus, cx);
        self.sync_terminal_focus();
        cx.notify();
    }

    pub(super) fn close_bar(&self) -> Option<AnyElement> {
        let pending = self.close_confirmation.as_ref()?;
        let color = |rgb| rgba(Color { rgb, alpha: 1.0 });
        Some(
            div()
                .flex_none()
                .h(px(self.close_bar_height() as f32))
                .w_full()
                .flex()
                .items_center()
                .gap(px(12.0))
                .px(px(12.0))
                .py(px(6.0))
                .bg(color(self.palette.top_bar))
                .text_color(color(self.palette.foreground))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(SharedString::from(pending.text.clone())),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(color(self.palette.inactive_text))
                        .child("Enter / Esc"),
                )
                .into_any_element(),
        )
    }

    /// Preserve the actual error output, adding an explanation and a way to close.
    pub(super) fn exit_banners(
        &self,
        panes: &[PaneBox],
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        panes
            .iter()
            .filter_map(|pane| {
                let text = closing::exit_text(self.workspace.panes.get(&pane.pane)?.status)?;
                let id = pane.pane;
                let rect = pane.rect;
                let color = |rgb| rgba(Color { rgb, alpha: 1.0 });
                Some(
                    div()
                        .absolute()
                        .left(px(rect.x as f32 + 1.0))
                        .top(px(
                            (rect.y + rect.height.saturating_sub(closing::EXIT_BAR_HEIGHT)) as f32,
                        ))
                        .w(px(rect.width.saturating_sub(2) as f32))
                        .h(px(rect.height.min(closing::EXIT_BAR_HEIGHT) as f32))
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(10.0))
                        .py(px(5.0))
                        .bg(color(self.palette.top_bar))
                        .text_color(color(self.palette.foreground))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(SharedString::from(text)),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("close-exited-{id}")))
                                .flex_none()
                                .px(px(8.0))
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .bg(color(self.palette.unfocused_border))
                                .child("Close pane")
                                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                    cx.stop_propagation()
                                })
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.request_close(Target::Pane(id), window, cx);
                                })),
                        )
                        .into_any_element(),
                )
            })
            .collect()
    }
}
