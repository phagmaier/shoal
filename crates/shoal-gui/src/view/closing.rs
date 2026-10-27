//! Inline close confirmation and retained exited-pane status (§7.7, §4.2).
use super::{ShoalWindow, rgba};
use crate::{
    chrome::{Color, PaneBox},
    closing::{self, Confirmation, Target},
    controls::{self, Looks},
};
use gpui::{AnyElement, Context, KeyDownEvent, SharedString, Window, div, prelude::*, px};
use shoal_client::proto::Call;

impl ShoalWindow {
    pub(super) fn close_bar_height(&self) -> u32 {
        if self.close_confirmation.is_some() {
            crate::chrome::TOP_BAR_HEIGHT
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

    /// The bar at the window's bottom: the warning, then its keys.
    pub(super) fn close_bar(&self) -> Option<AnyElement> {
        let pending = self.close_confirmation.as_ref()?;
        let looks = Looks::new(&self.palette);
        Some(
            div()
                .flex_none()
                .h(px(self.close_bar_height() as f32))
                .w_full()
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(12.0))
                .border_t_1()
                .border_color(rgba(Color {
                    rgb: self.palette.accent,
                    alpha: 0.6,
                }))
                .bg(rgba(Color {
                    rgb: self.palette.top_bar,
                    alpha: self.config.window.opacity as f32,
                }))
                .text_color(rgba(looks.text))
                .child(controls::glyph(controls::WARNING_GLYPH, looks.accent))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(SharedString::from(pending.text.clone())),
                )
                .child(controls::hints([
                    controls::hint(&["Enter"], "Close", &looks),
                    controls::hint(&["Esc"], "Cancel", &looks),
                ]))
                .into_any_element(),
        )
    }

    /// A retained exited pane's footer, inside its box below the grid: what
    /// ended it, and a way to close it. The output above stays as it was.
    pub(super) fn exit_banners(
        &self,
        panes: &[PaneBox],
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let looks = Looks::new(&self.palette);
        panes
            .iter()
            .filter(|pane| pane.footer > 0)
            .filter_map(|pane| {
                let text = closing::exit_text(self.workspace.panes.get(&pane.pane)?.status)?;
                let id = pane.pane;
                let rect = pane.rect;
                // Follow the box's bottom corners.
                let height = pane.footer.min(rect.height) as f32;
                Some(
                    div()
                        .absolute()
                        .left(px(rect.x as f32))
                        .top(px((rect.y + rect.height) as f32 - height))
                        .w(px(rect.width as f32))
                        .h(px(height))
                        .overflow_hidden()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(9.0))
                        .rounded_b(px(pane.radius))
                        .border_t_1()
                        .border_color(rgba(Color {
                            rgb: self.palette.unfocused_border,
                            alpha: 0.6,
                        }))
                        .bg(rgba(Color {
                            rgb: self.palette.top_bar,
                            alpha: self.config.window.opacity as f32,
                        }))
                        .text_color(rgba(looks.text))
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
                            controls::button(
                                SharedString::from(format!("close-exited-{id}")),
                                "Close pane",
                                &looks,
                            )
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation()
                            })
                            .on_click(cx.listener(
                                move |view, _, window, cx| {
                                    view.request_close(Target::Pane(id), window, cx);
                                },
                            )),
                        )
                        .into_any_element(),
                )
            })
            .collect()
    }
}
