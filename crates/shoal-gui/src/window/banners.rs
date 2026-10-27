//! Native recovery controls and dismissable notices, above the pane area.
use gpui::{AnyElement, Context, MouseButton, Rgba, SharedString, canvas, div, prelude::*, px};
use shoal_config::schema::Rgb;

use super::ShoalRoot;
use crate::{
    chrome::{Color, TOP_BAR_HEIGHT},
    controls::{self, Looks},
    recovery::Status,
    view::rgba,
};

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

    /// Recovery explanations and notices above the pane area, one line
    /// each except an explanation, which wraps.
    pub(super) fn banners(&self, cx: &mut Context<Self>) -> AnyElement {
        let looks = Looks::new(&self.palette);
        let rule = Color {
            rgb: self.palette.unfocused_border,
            alpha: 0.6,
        };
        let row = || {
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(12.0))
                .border_b_1()
                .border_color(rgba(rule))
        };
        let explanation = self.recovery.explanation().map(|text| {
            let button = controls::button(
                "restart-daemon",
                if self.recovery.status == Status::Restarting {
                    "Restarting…"
                } else {
                    "Restart daemon"
                },
                &looks,
            )
            .on_click(cx.listener(|root, _, window, cx| root.restart_daemon(window, cx)));
            row()
                .py(px(8.0))
                .border_color(rgba(Color {
                    rgb: self.palette.accent,
                    alpha: 0.6,
                }))
                .child(controls::glyph(controls::WARNING_GLYPH, looks.accent))
                .child(div().flex_1().child(text))
                .child(button)
        });
        let notices = self
            .notices
            .visible()
            .map(|notice| {
                let id = notice.id;
                row()
                    .h(px(30.0))
                    .child(controls::glyph(controls::WARNING_GLYPH, looks.accent))
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
                        controls::button(
                            SharedString::from(format!("dismiss-notice-{id}")),
                            "Dismiss",
                            &looks,
                        )
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
            .bg(rgba(Color {
                rgb: self.palette.top_bar,
                alpha: self.config.window.opacity as f32,
            }))
            .text_color(rgba(looks.text))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(explanation)
            .children(notices)
            .children(self.content.clone().map(|content| {
                // Measure native text layout, including wrapped mismatch
                // explanations, so panes and mouse coordinates stay below it.
                // GPUI ignores a notify made while drawing, so the new height
                // goes in after this frame, and its notify draws the next.
                canvas(
                    |_, _, _| (),
                    move |bounds, _, _, cx| {
                        let height = f32::from(bounds.size.height).ceil().max(0.0) as u32;
                        cx.defer(move |cx| {
                            content.update(cx, |view, cx| view.set_banner_height(height, cx));
                        });
                    },
                )
                .absolute()
                .size_full()
            }))
            .into_any_element()
    }
}
