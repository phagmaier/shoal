//! New-output indicators are driven by bytes, never an idle timer.
use super::{ShoalWindow, rgba};
use crate::{
    chrome::{Color, PaneBox},
    controls,
};
use gpui::{AnyElement, Context, MouseButton, SharedString, div, prelude::*, px};

/// Space between the pill and the grid's corner.
const PILL_MARGIN: f32 = 10.0;

impl ShoalWindow {
    /// A pill at the bottom right of each scrolled-up pane with new output;
    /// clicking it returns to the bottom.
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
                let content = pane.content();
                let text = Color::opaque(self.palette.background);
                let pill = div()
                    .id(SharedString::from(format!("new-output-{id}")))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .pl(px(8.0))
                    .pr(px(12.0))
                    .py(px(3.0))
                    .rounded_full()
                    .cursor_pointer()
                    .shadow_md()
                    .bg(rgba(Color::opaque(self.palette.accent)))
                    .text_color(rgba(text))
                    .child(controls::glyph(controls::DOWN_GLYPH, text))
                    .child("New output")
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
                    }));
                // A frame over the content area places it at the corner
                // without knowing its width; only the pill takes the mouse.
                Some(
                    div()
                        .absolute()
                        .left(px(content.x as f32))
                        .top(px(content.y as f32))
                        .w(px(content.width as f32))
                        .h(px(content.height as f32))
                        .flex()
                        .justify_end()
                        .items_end()
                        .p(px(PILL_MARGIN))
                        .child(pill)
                        .into_any_element(),
                )
            })
            .collect()
    }
}
