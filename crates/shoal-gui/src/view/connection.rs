//! Drop old connection state before any fresh snapshot can be displayed.
use super::{ShoalWindow, rgba};
use crate::{
    chrome::{Color, PaneBox},
    mirror::State,
};
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};
use shoal_config::Config;

impl ShoalWindow {
    pub fn current_session(&self) -> shoal_client::model::SessionId {
        self.session
    }

    pub fn set_banner_height(&mut self, height: u32, cx: &mut Context<Self>) {
        if self.banner_height != height {
            self.banner_height = height;
            self.sizer.reset();
            cx.notify();
        }
    }

    pub fn apply_input_config(&mut self, config: &Config) {
        self.config.keybinds = config.keybinds.clone();
        self.config.selection = config.selection.clone();
        self.config.behavior.confirm_close_running = config.behavior.confirm_close_running;
    }

    pub fn set_available(&mut self, available: bool, cx: &mut Context<Self>) {
        if self.available && !available {
            self.panes.clear();
            self.selection = None;
            self.preedit = None;
            self.input_cursor = None;
            self.terminal_focus = None;
            self.pressed.clear();
            self.mouse_capture = None;
            self.wheel_pending.clear();
            self.drag = None;
            self.ratio_timer = None;
            self.size_timer = None;
            self.rename = None;
            self.close_confirmation = None;
            self.switcher = None;
            self.session_busy = false;
            self.sizer.reset();
            cx.notify();
        }
        self.available = available;
    }
    /// A pane's banner, with Retry, laid over its box.
    pub(super) fn banner(
        &mut self,
        pane_box: &PaneBox,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let State::Stopped(banner) = &self.panes.get_mut(pane_box.pane)?.state else {
            return None;
        };
        let pane = pane_box.pane;
        let rect = pane_box.rect;
        let retry = div()
            .id(SharedString::from(format!("retry-{pane}")))
            .px(px(10.0))
            .py(px(3.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .bg(rgba(Color {
                rgb: self.palette.unfocused_border,
                alpha: 1.0,
            }))
            .child("Retry")
            .on_click(cx.listener(move |view, _, _, cx| {
                let ShoalWindow { panes, client, .. } = view;
                panes.retry(pane, client, cx);
                cx.notify();
            }));
        Some(
            div()
                .absolute()
                .left(px(rect.x as f32))
                .top(px(rect.y as f32))
                .w(px(rect.width as f32))
                .h(px(rect.height as f32))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.0))
                .p(px(12.0))
                .text_color(rgba(Color {
                    rgb: self.palette.foreground,
                    alpha: 1.0,
                }))
                .child(SharedString::from(banner.text()))
                .child(retry)
                .into_any_element(),
        )
    }
}
