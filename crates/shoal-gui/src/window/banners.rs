//! Native recovery controls and dismissable notices, above the pane area.
use gpui::{AnyElement, Context, MouseButton, Rgba, SharedString, canvas, div, prelude::*, px};
use shoal_config::schema::Rgb;

use super::ShoalRoot;
use crate::{chrome::TOP_BAR_HEIGHT, recovery::Status};

impl ShoalRoot {
    pub(super) fn color(&self, rgb: Rgb) -> Rgba {
        crate::view::rgba(crate::chrome::Color { rgb, alpha: 1.0 })
    }

    pub(super) fn overlay(&self) -> Option<AnyElement> {
        let text = self.recovery.overlay()?;
        Some(
            div()
                .absolute()
                .inset_0()
                .top(px(TOP_BAR_HEIGHT as f32))
                .flex()
                .items_center()
                .justify_center()
                .p(px(20.0))
                .bg(self.color(self.palette.background))
                .text_color(self.color(self.palette.foreground))
                .child(SharedString::from(text.to_owned()))
                .into_any_element(),
        )
    }

    pub(super) fn banners(&self, cx: &mut Context<Self>) -> AnyElement {
        let explanation = self.recovery.explanation().map(|text| {
            let button = div()
                .id("restart-daemon")
                .flex_none()
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(4.0))
                .bg(self.color(self.palette.unfocused_border))
                .child(if self.recovery.status == Status::Restarting {
                    "Restarting…"
                } else {
                    "Restart daemon"
                })
                .cursor_pointer()
                .on_click(cx.listener(|root, _, window, cx| root.restart_daemon(window, cx)));
            div()
                .flex()
                .items_center()
                .gap(px(12.0))
                .p(px(10.0))
                .border_b_1()
                .border_color(self.color(self.palette.accent))
                .child(div().flex_1().child(text))
                .child(button)
        });
        let notices = self
            .notices
            .visible()
            .map(|notice| {
                let id = notice.id;
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .h(px(30.0))
                    .px(px(10.0))
                    .border_b_1()
                    .border_color(self.color(self.palette.unfocused_border))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(SharedString::from(notice.text.clone())),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("dismiss-notice-{id}")))
                            .flex_none()
                            .px(px(6.0))
                            .cursor_pointer()
                            .child("Dismiss")
                            .on_click(cx.listener(move |root, _, _, cx| {
                                root.notices.dismiss(id);
                                cx.notify();
                            })),
                    )
            })
            .collect::<Vec<_>>();
        div()
            .id("banners")
            .absolute()
            .top(px(TOP_BAR_HEIGHT as f32))
            .left_0()
            .w_full()
            .flex()
            .flex_col()
            .bg(self.color(self.palette.top_bar))
            .text_size(px(13.0))
            .text_color(self.color(self.palette.foreground))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(explanation)
            .children(notices)
            .children(self.content.clone().map(|content| {
                // Measure native text layout, including wrapped mismatch
                // explanations, so panes and mouse coordinates stay below it.
                canvas(
                    |_, _, _| (),
                    move |bounds, _, _, cx| {
                        content.update(cx, |view, cx| {
                            view.set_banner_height(
                                f32::from(bounds.size.height).ceil().max(0.0) as u32,
                                cx,
                            );
                        });
                    },
                )
                .absolute()
                .size_full()
            }))
            .into_any_element()
    }
}
