//! New-output indicators are driven by bytes, never an idle timer.
use super::{ShoalWindow, rgba};
use crate::chrome::{Color, PaneBox};
use gpui::{AnyElement, Context, MouseButton, SharedString, div, prelude::*, px};

impl ShoalWindow {
    pub(super) fn output_pills(
        &mut self,
        panes: &[PaneBox],
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        panes
            .iter()
            .filter_map(|pane| {
                if !self.mirror(pane.pane)?.has_new_output() {
                    return None;
                }
                let id = pane.pane;
                Some(
                    div()
                        .id(SharedString::from(format!("new-output-{id}")))
                        .absolute()
                        .left(px(
                            (pane.rect.x + pane.rect.width).saturating_sub(104) as f32
                        ))
                        .top(px(
                            (pane.rect.y + pane.rect.height).saturating_sub(34) as f32
                        ))
                        .px(px(10.0))
                        .py(px(4.0))
                        .rounded(px(12.0))
                        .cursor_pointer()
                        .bg(rgba(Color {
                            rgb: self.palette.accent,
                            alpha: 1.0,
                        }))
                        .text_color(rgba(Color {
                            rgb: self.palette.background,
                            alpha: 1.0,
                        }))
                        .child("new output")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |view, _, _, cx| {
                            if let Some(mirror) = view.mirror(id)
                                && let Err(error) = mirror.scroll(shoal_term::Scroll::Bottom)
                            {
                                tracing::warn!(%error, "can't return to new output");
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }))
                        .into_any_element(),
                )
            })
            .collect()
    }
}
