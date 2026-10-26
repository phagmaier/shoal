//! Terminal mouse capture and visible-grid selection (§7.4).

use super::{ShoalWindow, layout::pane_point};
use crate::{
    chrome, input, metrics,
    selection::{CellPoint, Selection},
};
use gpui::{
    ClipboardItem, Context, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, ScrollDelta, ScrollWheelEvent, Window,
};
use shoal_client::{model::PaneId, proto::Call};
use shoal_term::{CellSize, MouseAction, MouseButton as Button, MouseEvent};

impl ShoalWindow {
    pub(super) fn pointer_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button == MouseButton::Left {
            self.press(event.position, cx);
            if self.drag.is_some() {
                return;
            }
        }
        let Some(pane) = chrome::pane_at(&self.shown.panes, pane_point(event.position)) else {
            return;
        };
        window.focus(&self.focus, cx);
        if event.button == MouseButton::Left && self.focused_pane() != Some(pane) {
            drop(self.client.send(Call::PaneFocus { pane }));
        }
        if !event.modifiers.shift && self.reports_mouse(pane) {
            self.selection = None;
            self.mouse_capture = Some(pane);
            self.report_mouse(
                pane,
                event.position,
                MouseAction::Press,
                button(event.button),
                event.modifiers,
                true,
            );
        } else if event.button == MouseButton::Left {
            if let Some(at) = self.cell_at(pane, event.position)
                && let Some(mirror) = self.mirror(pane)
            {
                match mirror.screen() {
                    Ok(screen) => {
                        self.selection = Some(Selection::new(pane, screen, at, event.click_count))
                    }
                    Err(error) => tracing::warn!(%error, "can't select terminal text"),
                }
            }
        } else if event.button == MouseButton::Middle
            && let Some(text) = cx.read_from_primary().and_then(|item| item.text())
        {
            self.paste(pane, &text);
        }
        cx.notify();
    }

    pub(super) fn pointer_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.drag.is_some() {
            return;
        }
        if let Some(pane) = self
            .selection
            .as_ref()
            .filter(|s| s.dragging)
            .map(|s| s.pane)
        {
            if event.pressed_button == Some(MouseButton::Left) {
                if let Some(at) = self.cell_at(pane, event.position)
                    && let Some(selection) = &mut self.selection
                {
                    selection.move_to(at);
                }
            } else {
                self.finish_selection(cx);
            }
            cx.notify();
            return;
        }
        let pane = self
            .mouse_capture
            .or_else(|| chrome::pane_at(&self.shown.panes, pane_point(event.position)));
        if let Some(pane) = pane
            && (self.mouse_capture.is_some() || !event.modifiers.shift)
            && self.reports_mouse(pane)
        {
            self.report_mouse(
                pane,
                event.position,
                MouseAction::Motion,
                event.pressed_button.and_then(button),
                event.modifiers,
                event.pressed_button.is_some(),
            );
        }
    }

    pub(super) fn pointer_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if let Some(pane) = self.mouse_capture.take() {
            self.report_mouse(
                pane,
                event.position,
                MouseAction::Release,
                button(event.button),
                event.modifiers,
                false,
            );
        }
        if event.button == MouseButton::Left {
            self.finish_selection(cx);
        }
        cx.notify();
    }

    fn finish_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(selection) = &mut self.selection {
            selection.dragging = false;
            let text = selection.text();
            if !text.is_empty() {
                let item = ClipboardItem::new_string(text);
                cx.write_to_primary(item.clone());
                if self.config.selection.copy_on_select {
                    cx.write_to_clipboard(item);
                }
            }
        }
    }

    fn reports_mouse(&mut self, pane: PaneId) -> bool {
        self.mirror(pane)
            .and_then(|m| m.mouse_reporting().ok())
            .unwrap_or(false)
    }

    /// Grid-relative device pixels, matching the encoder's integer cell size.
    fn mouse_position(
        &self,
        pane: PaneId,
        position: Point<Pixels>,
    ) -> Option<(f32, f32, CellSize)> {
        let cell = self.metrics.as_ref()?.0;
        let pane = self.shown.panes.iter().find(|p| p.pane == pane)?;
        let (x, y) = metrics::grid_origin(pane.rect, &cell);
        let at = pane_point(position);
        Some((
            (at.0 - x) * cell.scale,
            (at.1 - y) * cell.scale,
            CellSize {
                width: (cell.width * cell.scale).round().max(1.0) as u32,
                height: (cell.height * cell.scale).round().max(1.0) as u32,
            },
        ))
    }

    fn cell_at(&self, pane: PaneId, position: Point<Pixels>) -> Option<CellPoint> {
        let (x, y, cell) = self.mouse_position(pane, position)?;
        let size = self.workspace.panes.get(&pane)?.size;
        Some(CellPoint {
            col: (x / cell.width as f32)
                .floor()
                .max(0.0)
                .min(f32::from(size.cols.saturating_sub(1))) as u16,
            row: (y / cell.height as f32)
                .floor()
                .max(0.0)
                .min(f32::from(size.rows.saturating_sub(1))) as u16,
        })
    }

    fn report_mouse(
        &mut self,
        pane: PaneId,
        position: Point<Pixels>,
        action: MouseAction,
        button: Option<Button>,
        modifiers: Modifiers,
        held: bool,
    ) {
        let Some((x, y, cell)) = self.mouse_position(pane, position) else {
            return;
        };
        let event = MouseEvent {
            x,
            y,
            action,
            button,
            mods: input::mods(modifiers),
            buttons_held: held,
        };
        if let Some(mirror) = self.mirror(pane) {
            let bytes = mirror.mouse(&event, cell);
            self.send_input(pane, bytes);
        }
    }

    pub(super) fn wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        let Some(pane) = chrome::pane_at(&self.shown.panes, pane_point(event.position)) else {
            return;
        };
        // Local scrolling is T45. Shift always bypasses program capture.
        if event.modifiers.shift || !self.reports_mouse(pane) {
            return;
        }
        let Some((cell, _, _)) = self.metrics.as_ref() else {
            return;
        };
        let (x, y) = match event.delta {
            ScrollDelta::Lines(delta) => (delta.x, delta.y),
            ScrollDelta::Pixels(delta) => (
                f32::from(delta.x) / cell.width,
                f32::from(delta.y) / cell.height,
            ),
        };
        let pending = self.wheel_pending.entry(pane).or_default();
        pending.0 += x;
        pending.1 += y;
        let (x, y) = (pending.0.trunc() as i32, pending.1.trunc() as i32);
        pending.0 -= x as f32;
        pending.1 -= y as f32;
        for (n, positive, negative) in [
            (x, Button::WheelLeft, Button::WheelRight),
            (y, Button::WheelUp, Button::WheelDown),
        ] {
            for _ in 0..n.unsigned_abs().min(100) {
                self.report_mouse(
                    pane,
                    event.position,
                    MouseAction::Press,
                    Some(if n > 0 { positive } else { negative }),
                    event.modifiers,
                    false,
                );
            }
        }
    }
}

fn button(button: MouseButton) -> Option<Button> {
    match button {
        MouseButton::Left => Some(Button::Left),
        MouseButton::Middle => Some(Button::Middle),
        MouseButton::Right => Some(Button::Right),
        MouseButton::Navigate(gpui::NavigationDirection::Back) => Some(Button::Back),
        MouseButton::Navigate(gpui::NavigationDirection::Forward) => Some(Button::Forward),
    }
}
