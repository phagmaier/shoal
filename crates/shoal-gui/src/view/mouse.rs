//! Terminal mouse capture, local scrolling and content selection (§7.4–7.5).

use super::{ShoalWindow, layout::pane_point};
use crate::{chrome, input, metrics, selection::Selection};
use gpui::{
    ClipboardItem, Context, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, ScrollDelta, ScrollWheelEvent, Window,
};
use shoal_client::{model::PaneId, proto::Call};
use shoal_term::{CellPoint, CellSize, MouseAction, MouseButton as Button, MouseEvent};

impl ShoalWindow {
    pub(super) fn pointer_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.available
            || self.awaiting_workspace
            || self.close_confirmation.is_some()
            || self.switcher.is_some()
            || self.session_busy
        {
            return;
        }
        if event.button == MouseButton::Left {
            self.press(event.position, cx);
            if self.drag.is_some() {
                return;
            }
        }
        let at = pane_point(event.position);
        let Some(pane) = chrome::pane_at(&self.shown.panes, at) else {
            return;
        };
        window.focus(&self.focus, cx);
        if event.button == MouseButton::Left && self.focused_pane() != Some(pane) {
            drop(self.client.send(Call::PaneFocus { pane }));
        }
        self.hover_point = Some(event.position);
        if event.button == MouseButton::Left
            && event.modifiers.control
            && let Some(at) = self.cell_at(pane, event.position)
            && let Some(mirror) = self.mirror(pane)
            && let Ok(links) = mirror.links(at.row)
            && let Some(link) = links
                .into_iter()
                .find(|link| link.columns.contains(&at.col))
        {
            self.clear_selection();
            cx.background_executor()
                .spawn(async move {
                    let result = std::process::Command::new("xdg-open")
                        .arg(link.uri)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status();
                    if !result.is_ok_and(|status| status.success()) {
                        tracing::warn!("couldn't open terminal link");
                    }
                })
                .detach();
            cx.notify();
            return;
        }
        if !event.modifiers.shift && self.reports_mouse(pane) {
            self.clear_selection();
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
            self.clear_selection();
            if let Some(at) = self.cell_at(pane, event.position)
                && let Some(mirror) = self.mirror(pane)
            {
                match mirror.begin_selection(at, event.click_count) {
                    Ok(()) => {
                        self.selection = Some(Selection {
                            pane,
                            dragging: true,
                        });
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
        if self.hover_point != Some(event.position) {
            self.hover_point = Some(event.position);
            if event.modifiers.control {
                cx.notify();
            }
        }
        if self.close_confirmation.is_some() || self.switcher.is_some() || self.session_busy {
            return;
        }
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
                    && let Some(mirror) = self.mirror(pane)
                    && let Err(error) = mirror.extend_selection(at)
                {
                    tracing::warn!(%error, "can't extend terminal selection");
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
        if self.close_confirmation.is_some() || self.switcher.is_some() || self.session_busy {
            return;
        }
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
            let pane = selection.pane;
            let Some(text) = self.mirror(pane).and_then(|m| m.selection_text().ok()) else {
                return;
            };
            if !text.is_empty() {
                let item = ClipboardItem::new_string(text);
                cx.write_to_primary(item.clone());
                if self.config.selection.copy_on_select {
                    cx.write_to_clipboard(item);
                }
            }
        }
    }

    pub(super) fn clear_selection(&mut self) {
        if let Some(selection) = self.selection.take()
            && let Some(mirror) = self.mirror(selection.pane)
            && let Err(error) = mirror.clear_selection()
        {
            tracing::warn!(%error, "can't clear terminal selection");
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
        let (x, y) = metrics::grid_origin(pane.content(), &cell);
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

    pub(super) fn cell_at(&mut self, pane: PaneId, position: Point<Pixels>) -> Option<CellPoint> {
        let (x, y, cell) = self.mouse_position(pane, position)?;
        let size = self.workspace.panes.get(&pane)?.size;
        let fraction = self.mirror(pane).map_or(0.0, |m| m.fraction());
        Some(CellPoint {
            col: (x / cell.width as f32)
                .floor()
                .max(0.0)
                .min(f32::from(size.cols.saturating_sub(1))) as u16,
            row: (y / cell.height as f32 + fraction)
                .floor()
                .max(0.0)
                .min(f32::from(
                    size.rows.saturating_sub(u16::from(fraction == 0.0)),
                )) as u16,
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
        cx: &mut Context<Self>,
    ) {
        if self.close_confirmation.is_some() || self.switcher.is_some() || self.session_busy {
            return;
        }
        let Some(pane) = chrome::pane_at(&self.shown.panes, pane_point(event.position)) else {
            return;
        };
        let local = event.modifiers.shift || !self.reports_mouse(pane);
        let Some((cell, _, _)) = self.metrics.as_ref() else {
            return;
        };
        let (x, y) = match event.delta {
            ScrollDelta::Lines(delta) => (delta.x, delta.y * if local { 3.0 } else { 1.0 }),
            ScrollDelta::Pixels(delta) => (
                f32::from(delta.x) / cell.width,
                f32::from(delta.y) / cell.height,
            ),
        };
        if local {
            if let Some(mirror) = self.mirror(pane)
                && let Err(error) = mirror.scroll_rows(-y)
            {
                tracing::warn!(%error, "can't scroll terminal");
            }
            if let Some(selection) = &self.selection
                && selection.dragging
                && selection.pane == pane
                && let Some(at) = self.cell_at(pane, event.position)
                && let Some(mirror) = self.mirror(pane)
                && let Err(error) = mirror.extend_selection(at)
            {
                tracing::warn!(%error, "can't extend terminal selection");
            }
            cx.notify();
            return;
        }
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
