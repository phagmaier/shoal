//! Pane boxes (M1 spec §7.2): each rounded box, its focus edge, and its grid
//! painted inside the content area, clipped to the grid's rows.

use gpui::{
    App, BorderStyle, Bounds, ContentMask, ElementInputHandler, Entity, FocusHandle, Window,
    canvas, fill, linear_color_stop, linear_gradient, point, prelude::*, px, quad, size,
};

use super::{ShoalWindow, hsla, rgba};
use crate::{
    chrome::{Color, PaneBox},
    metrics::{self, CellMetrics},
    terminal::{self, PaneFrame},
};

/// Paint every box and its grid. While `input` is set, its view takes text
/// input; `listen` installs the window-wide pointer listeners.
pub(super) fn paint(
    panes: Vec<PaneBox>,
    frames: Vec<Option<PaneFrame>>,
    cell: CellMetrics,
    input: Option<(FocusHandle, Entity<ShoalWindow>)>,
    listen: impl FnOnce(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, cx| {
            if let Some((focus, view)) = input {
                window.handle_input(&focus, ElementInputHandler::new(bounds, view), cx);
            }
            listen(window, cx);
            for (pane, frame) in panes.iter().zip(&frames) {
                let origin = bounds.origin + point(px(pane.rect.x as f32), px(pane.rect.y as f32));
                let extent = size(px(pane.rect.width as f32), px(pane.rect.height as f32));
                window.paint_quad(quad(
                    Bounds::new(origin, extent),
                    px(pane.radius),
                    rgba(pane.fill),
                    px(f32::from(pane.edge.is_some())),
                    pane.edge.map(hsla).unwrap_or_else(gpui::transparent_black),
                    BorderStyle::Solid,
                ));
                if let Some(color) = pane.focus {
                    // Two gradients meet at the middle, fading before the
                    // rounded corners, above the faint outline.
                    let inset = pane.radius.min(pane.rect.width as f32 / 2.0);
                    let half = (pane.rect.width as f32 / 2.0 - inset).max(0.0);
                    let clear = hsla(Color {
                        alpha: 0.0,
                        ..color
                    });
                    let bright = hsla(color);
                    for (x, from, to) in [(inset, clear, bright), (inset + half, bright, clear)] {
                        window.paint_quad(fill(
                            Bounds::new(origin + point(px(x), px(0.0)), size(px(half), px(1.5))),
                            linear_gradient(
                                90.0,
                                linear_color_stop(from, 0.0),
                                linear_color_stop(to, 1.0),
                            ),
                        ));
                    }
                }
                let Some(frame) = frame else {
                    continue;
                };
                let content = pane.content();
                let (x, y, width, height) =
                    metrics::grid_clip(content, &cell, frame.visible_rows());
                let clip = Bounds::new(
                    bounds.origin + point(px(x), px(y)),
                    size(px(width), px(height)),
                );
                let (x, y) = metrics::grid_origin(content, &cell);
                let grid = bounds.origin + point(px(x), px(y));
                window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                    window.paint_layer(clip, |window| terminal::paint(frame, grid, window));
                });
            }
        },
    )
    .size_full()
}
