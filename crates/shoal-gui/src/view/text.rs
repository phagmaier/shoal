//! GPUI input-method contract. Preedit stays local until commit. The
//! inline rename field uses the same handler, with UTF-16 range conversion.

use super::ShoalWindow;
use crate::{chrome::TOP_BAR_HEIGHT, metrics, rename::byte_offset};
use gpui::{
    Bounds, ClipboardItem, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window,
    point, px, size,
};
use std::ops::Range;

impl EntityInputHandler for ShoalWindow {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = self
            .rename
            .as_ref()
            .map(|r| r.text())
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
        let range = if let Some(rename) = &self.rename {
            rename.selected_utf16()
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
        if let Some(rename) = &self.rename {
            rename.marked_utf16()
        } else {
            self.preedit.as_ref().map(|s| 0..s.encode_utf16().count())
        }
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(rename) = &mut self.rename {
            rename.unmark();
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
        if let Some(rename) = &mut self.rename {
            rename.replace(range, text, None, false);
        } else if let Some(pane) = self.focused_pane() {
            self.client.input(pane, text.as_bytes().to_vec());
            self.selection = None;
        }
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
        if let Some(rename) = &mut self.rename {
            rename.replace(range, text, selected, true);
        } else {
            self.preedit = (!text.is_empty()).then(|| text.to_owned());
        }
        cx.notify();
    }

    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = item.text() {
            if self.rename.is_some() {
                self.replace_text_in_range(None, &text, window, cx);
            } else if let Some(pane) = self.focused_pane() {
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
        if self.rename.is_some() {
            return Some(bounds);
        }
        let cell = self.metrics.as_ref()?.0;
        let pane = self.focused_pane()?;
        let rect = self.shown.panes.iter().find(|p| p.pane == pane)?.rect;
        let (x, y) = metrics::grid_origin(rect, &cell);
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
