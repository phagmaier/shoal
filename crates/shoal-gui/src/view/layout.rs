//! The pane area's mouse and sizes (M1 spec §7.2): click to focus a pane,
//! drag a divider, and ask for each shown pane's grid size.

use std::time::Instant;

use gpui::{
    AnyElement, App, Context, CursorStyle, DispatchPhase, MouseButton, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Window, div, prelude::*, px,
};
use shoal_client::{
    model::{Axis, Divider},
    proto::Call,
};

use super::ShoalWindow;
use crate::{
    chrome::{PaneBox, TOP_BAR_HEIGHT},
    drag::{self, Drag, Grab},
    metrics::{self, CellMetrics},
};

/// What the last frame drew, in pane-area pixels.
#[derive(Debug, Default)]
pub struct Shown {
    pub panes: Vec<PaneBox>,
    pub dividers: Vec<Divider>,
}

impl ShoalWindow {
    /// Ask for the grid each shown pane's box holds, and wake when a size
    /// held back during a drag may go. Nothing waits while idle.
    pub(super) fn request_sizes(
        &mut self,
        panes: &[PaneBox],
        cell: &CellMetrics,
        cx: &mut Context<Self>,
    ) {
        let wanted: Vec<_> = panes
            .iter()
            .filter_map(|pane| Some((pane.pane, metrics::grid_size(pane.content(), cell)?)))
            .collect();
        let now = Instant::now();
        // A drag's sizes keep waiting after the release until they hold
        // still: the daemon may still echo a ratio sent before the last.
        let debounce = self.drag.is_some() || self.sizer.deadline().is_some();
        let workspace = &self.workspace;
        let ask = self.sizer.update(
            &wanted,
            |pane| workspace.panes.get(&pane).map(|pane| pane.size),
            now,
            debounce,
        );
        for (pane, size) in ask {
            tracing::debug!(%pane, cols = size.cols, rows = size.rows, "asking for a size");
            drop(self.client.send(Call::PaneSetSize {
                pane,
                cols: size.cols,
                rows: size.rows,
            }));
        }
        match self.sizer.deadline() {
            None => self.size_timer = None,
            Some(due) if self.size_timer.as_ref().is_some_and(|(at, _)| *at == due) => {}
            Some(due) => {
                let wait = due.saturating_duration_since(now);
                let task = cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    this.update(cx, |_, cx| cx.notify()).ok();
                });
                self.size_timer = Some((due, task));
            }
        }
    }

    /// The resize cursor over each divider's grab area.
    pub(super) fn divider_handles(&self, dividers: &[Divider]) -> Vec<AnyElement> {
        dividers
            .iter()
            .map(|divider| {
                let Grab {
                    left,
                    top,
                    right,
                    bottom,
                } = drag::grab_area(divider);
                div()
                    .absolute()
                    .left(px(left))
                    .top(px(top))
                    .w(px(right - left))
                    .h(px(bottom - top))
                    .cursor(resize_cursor(divider.axis))
                    .into_any_element()
            })
            .collect()
    }

    /// Window-wide pointer listeners, including releases outside a pane.
    /// During a divider drag, keep its cursor and follow it to release.
    pub(super) fn drag_listeners(
        &self,
        dragging: Option<Axis>,
        cx: &mut Context<Self>,
    ) -> impl FnOnce(&mut Window, &mut App) + 'static {
        let view = cx.entity();
        move |window, _| {
            if let Some(axis) = dragging {
                window.set_window_cursor_style(resize_cursor(axis));
            }
            let moved = view.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble {
                    moved.update(cx, |view, cx| {
                        if view.drag.is_some() {
                            view.drag_to(event, cx);
                        } else {
                            view.pointer_move(event, cx);
                        }
                    });
                }
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble {
                    view.update(cx, |view, cx| {
                        if event.button == MouseButton::Left && view.drag.is_some() {
                            view.end_drag(cx);
                        } else {
                            view.pointer_up(event, cx);
                        }
                    });
                }
            });
        }
    }

    /// A left press in the pane area: grab a divider.
    pub(super) fn press(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let point = pane_point(position);
        if let Some(divider) = self.shown.dividers.iter().find(|d| drag::grabs(d, point)) {
            self.drag = Some(Drag::new(divider, point));
            cx.notify();
        }
    }

    fn drag_to(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        // A release the window never saw ends the drag too.
        if event.pressed_button != Some(MouseButton::Left) {
            self.end_drag(cx);
            return;
        }
        let Some(drag) = &mut self.drag else {
            return;
        };
        if drag.move_to(pane_point(event.position)) {
            self.follow_drag();
            cx.notify();
        }
        self.send_ratio(cx);
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        // The final value always goes.
        drop(self.client.send(Call::SplitSetRatio {
            split: drag.split,
            ratio: drag.ratio,
        }));
        self.ratio_timer = None;
        cx.notify();
    }

    /// Show the dragged ratio over whatever the daemon last sent, before it
    /// confirms it. A drag whose split is gone ends.
    pub(super) fn follow_drag(&mut self) {
        let Some(drag) = &self.drag else {
            return;
        };
        let found = self
            .workspace
            .sessions
            .iter_mut()
            .filter(|session| session.id == self.session)
            .flat_map(|session| session.tabs.iter_mut())
            .any(|tab| shoal_model::set_ratio(&mut tab.layout, drag.split, drag.ratio).is_ok());
        if !found {
            self.drag = None;
            self.ratio_timer = None;
        }
    }

    /// Send the dragged ratio if the throttle allows, otherwise once it
    /// does.
    fn send_ratio(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = &mut self.drag else {
            return;
        };
        let now = Instant::now();
        match drag.due(now) {
            Ok(None) => {}
            Ok(Some(ratio)) => {
                drop(self.client.send(Call::SplitSetRatio {
                    split: drag.split,
                    ratio,
                }));
                drag.sent(ratio, now);
            }
            Err(wait) if self.ratio_timer.is_none() => {
                self.ratio_timer = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    this.update(cx, |view, cx| {
                        view.ratio_timer = None;
                        view.send_ratio(cx);
                    })
                    .ok();
                }));
            }
            Err(_) => {}
        }
    }
}

/// A window position in pane-area pixels: the pane area is the window
/// below the top bar.
pub(super) fn pane_point(position: Point<Pixels>) -> (f32, f32) {
    (
        f32::from(position.x),
        f32::from(position.y) - TOP_BAR_HEIGHT as f32,
    )
}

fn resize_cursor(axis: Axis) -> CursorStyle {
    match axis {
        Axis::Row => CursorStyle::ResizeLeftRight,
        Axis::Column => CursorStyle::ResizeUpDown,
    }
}
