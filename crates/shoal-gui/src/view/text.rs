//! GPUI input-method contract. Preedit stays local until commit. The
//! inline rename field uses the same handler, with UTF-16 range conversion.

use super::{ShoalWindow, TEXT_SIZE, rgba};
use crate::{
    chrome::{Color, TOP_BAR_HEIGHT},
    field::{TextField, byte_offset},
    metrics,
};
use gpui::{
    Bounds, ClipboardItem, Context, Div, EntityInputHandler, FocusHandle, Pixels, Point,
    SharedString, UTF16Selection, Window, div, point, prelude::*, px, size,
};
use std::ops::Range;

impl ShoalWindow {
    pub(super) fn editing(&self) -> Option<&TextField> {
        if let Some(picker) = &self.switcher {
            picker.field()
        } else {
            self.rename.as_ref().map(|r| &r.field)
        }
    }

    fn editing_mut(&mut self) -> Option<&mut TextField> {
        if let Some(picker) = &mut self.switcher {
            picker.field_mut()
        } else {
            self.rename.as_mut().map(|r| &mut r.field)
        }
    }

    pub(super) fn filter_changed(&mut self) {
        if let Some(picker) = &mut self.switcher {
            picker.filtered(&self.workspace.sessions, self.session);
            self.switcher_scroll.scroll_to_item(0);
        }
    }

    pub(super) fn text_field(
        &self,
        field: &TextField,
        focus: &FocusHandle,
        id: &'static str,
        placeholder: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let input = cx.entity();
        let focus_for_input = focus.clone();
        let (before, after) = field.parts();
        let color = |rgb| rgba(Color { rgb, alpha: 1.0 });
        let mut before_element = div()
            .flex_none()
            .whitespace_nowrap()
            .child(SharedString::from(before.to_owned()));
        if field.all_selected() {
            before_element = before_element.bg(rgba(Color {
                rgb: self.palette.accent,
                alpha: 0.35,
            }));
        }
        div()
            .id(id)
            .relative()
            .track_focus(focus)
            .min_w(px(48.0))
            .min_h(px(24.0))
            .flex()
            .flex_row()
            .items_center()
            .overflow_hidden()
            .px(px(4.0))
            .rounded(px(4.0))
            .bg(color(self.palette.unfocused_border))
            .text_color(color(self.palette.foreground))
            .child(
                gpui::canvas(
                    |_, _, _| (),
                    move |bounds, (), window, cx| {
                        window.handle_input(
                            &focus_for_input,
                            gpui::ElementInputHandler::new(bounds, input),
                            cx,
                        );
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(before_element)
            .child(
                div()
                    .flex_none()
                    .w(px(1.5))
                    .h(px(TEXT_SIZE + 3.0))
                    .bg(color(self.palette.cursor)),
            )
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .child(SharedString::from(after.to_owned())),
            )
            .when(field.text().is_empty(), |element| {
                element.child(
                    div()
                        .text_color(rgba(Color {
                            rgb: self.palette.foreground,
                            alpha: 0.9,
                        }))
                        .child(placeholder),
                )
            })
    }
}

impl EntityInputHandler for ShoalWindow {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = self
            .editing()
            .map(|field| field.text())
            .or(self.preedit.as_deref())
            .unwrap_or("");
        let start = byte_offset(text, range.start);
        let end = byte_offset(text, range.end).max(start);
        *adjusted = Some(text[..start].encode_utf16().count()..text[..end].encode_utf16().count());
        Some(text[start..end].into())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let range = if let Some(field) = self.editing() {
            field.selected_utf16()
        } else {
            let at = self.preedit.as_deref().unwrap_or("").encode_utf16().count();
            at..at
        };
        Some(UTF16Selection {
            range,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        if let Some(field) = self.editing() {
            field.marked_utf16()
        } else {
            self.preedit.as_ref().map(|s| 0..s.encode_utf16().count())
        }
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(field) = self.editing_mut() {
            field.unmark();
        }
        self.preedit = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(field) = self.editing_mut() {
            field.replace(range, text, None, false);
        } else if self.available
            && !self.awaiting_workspace
            && self.close_confirmation.is_none()
            && self.switcher.is_none()
            && !self.session_busy
            && let Some(pane) = self.focused_pane()
        {
            if let Some(mirror) = self.mirror(pane)
                && let Err(error) = mirror.scroll(shoal_term::Scroll::Bottom)
            {
                tracing::warn!(%error, "can't return to terminal input");
            }
            self.client.input(pane, text.as_bytes().to_vec());
            self.clear_selection();
        }
        self.filter_changed();
        self.preedit = None;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(field) = self.editing_mut() {
            field.replace(range, text, selected, true);
        } else if self.close_confirmation.is_none() && self.switcher.is_none() && !self.session_busy
        {
            if !text.is_empty()
                && let Some(pane) = self.focused_pane()
                && let Some(mirror) = self.mirror(pane)
                && let Err(error) = mirror.scroll(shoal_term::Scroll::Bottom)
            {
                tracing::warn!(%error, "can't return to terminal input");
            }
            self.preedit = (!text.is_empty()).then(|| text.to_owned());
        }
        self.filter_changed();
        cx.notify();
    }

    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            if self.rename.is_some() || self.switcher.is_some() {
                self.replace_text_in_range(None, &text, window, cx);
            } else if self.close_confirmation.is_none()
                && self.switcher.is_none()
                && !self.session_busy
                && let Some(pane) = self.focused_pane()
            {
                self.paste(pane, &text);
                cx.notify();
            }
        }
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        if self.rename.is_some() || self.switcher.is_some() {
            return Some(bounds);
        }
        let cell = self.metrics.as_ref()?.0;
        let pane = self.focused_pane()?;
        let content = self.shown.panes.iter().find(|p| p.pane == pane)?.content();
        let (x, y) = metrics::grid_origin(content, &cell);
        let cursor = self.input_cursor.unwrap_or((0, 0));
        Some(Bounds::new(
            point(
                px(x + f32::from(cursor.0) * cell.width),
                px(TOP_BAR_HEIGHT as f32 + y + f32::from(cursor.1) * cell.height),
            ),
            size(px(cell.width), px(cell.height)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
