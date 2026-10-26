//! Local viewport navigation and pointer selection. Only tracked references
//! survive mutations; borrowed grid references never leave a single call.

use libghostty_vt::{
    screen::{CellWide, Screen, TrackedGridRef},
    selection::{FormatOptions, Selection},
    terminal::{Point, PointCoordinate, PointSpace, ScrollViewport},
};

use crate::{TermError, Terminal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPoint {
    pub row: u16,
    pub col: u16,
}

/// A soft-wrapped logical line and the physical hovered row within it.
#[derive(Debug)]
pub struct TextLine {
    pub cells: Vec<String>,
    pub hovered: std::ops::Range<usize>,
}

#[derive(Debug, Clone, Copy)]
pub enum Scroll {
    Top,
    Bottom,
    Rows(i32),
    Prompts(i32),
}

#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    /// Physical row offset from the oldest retained row.
    pub offset: u64,
    pub history: u64,
    pub rows: u16,
    pub at_bottom: bool,
}

#[derive(Debug)]
pub(crate) struct Anchor {
    start: TrackedGridRef,
    end: TrackedGridRef,
    screen: Screen,
    clicks: usize,
    extent: Option<TrackedGridRef>,
}

impl Anchor {
    fn valid(&self) -> bool {
        self.start.has_value()
            && self.end.has_value()
            && self.extent.as_ref().is_none_or(TrackedGridRef::has_value)
    }
}

impl Terminal {
    /// The core clamps pins in a partially erased history page onto surviving
    /// text. Observe history changes at parser boundaries to clear that range
    /// before later output can hide the erase. The core parses each sequence;
    /// plain output remains a bulk write, with no parallel escape parser.
    pub(crate) fn feed_selected(&mut self, bytes: &[u8]) -> usize {
        let mut used = 0;
        while self.view.selection.is_some() && used < bytes.len() {
            let tail = &bytes[used..];
            let screen = self.inner.active_screen().ok();
            let before = self.inner.scrollback_rows().ok();
            let count = if self.inner.is_vt_ground().unwrap_or(true) {
                let count = tail
                    .iter()
                    .position(|b| *b == 0x1b)
                    .map_or(tail.len(), |i| i + 1);
                self.inner.vt_write(&tail[..count]);
                count
            } else {
                match self.inner.vt_write_until_ground(tail) {
                    Ok(Some(count)) if count > 0 => count,
                    Ok(None) => tail.len(),
                    _ => break,
                }
            };
            used += count;
            let erased = screen == self.inner.active_screen().ok()
                && before
                    .zip(self.inner.scrollback_rows().ok())
                    .is_some_and(|(before, after)| before > after);
            if erased {
                let _ = self.clear_selection();
            }
            let _ = self.validate_selection();
        }
        used
    }

    fn grid_point(&self, at: CellPoint) -> Result<Point, TermError> {
        let coordinate = PointCoordinate {
            x: at.col,
            y: u32::from(at.row),
        };
        Ok(if at.row < self.rows()? {
            Point::Viewport(coordinate)
        } else {
            Point::Screen(PointCoordinate {
                y: self.inner.scrollbar()?.offset as u32 + coordinate.y,
                ..coordinate
            })
        })
    }

    pub fn scroll(&mut self, scroll: Scroll) {
        self.inner.scroll_viewport(match scroll {
            Scroll::Top => ScrollViewport::Top,
            Scroll::Bottom => ScrollViewport::Bottom,
            Scroll::Rows(rows) => ScrollViewport::Delta(rows as isize),
            Scroll::Prompts(count) => ScrollViewport::DeltaPrompt(count as isize),
        });
    }

    pub fn viewport(&self) -> Result<Viewport, TermError> {
        let bar = self.inner.scrollbar()?;
        Ok(Viewport {
            offset: bar.offset,
            history: bar.total.saturating_sub(bar.len),
            rows: self.rows()?,
            at_bottom: self.inner.viewport_active()?,
        })
    }

    /// Begin a selection at a viewport cell. A single click has no selection
    /// until dragged; word and physical-row selection match §7.4's rules.
    pub fn begin_selection(&mut self, at: CellPoint, clicks: usize) -> Result<(), TermError> {
        self.clear_selection()?;
        let (start, end) = self.selection_edge(at, clicks)?;
        let anchor = Anchor {
            start: self
                .inner
                .track_grid_ref(self.grid_point(CellPoint { col: start, ..at })?)?,
            end: self
                .inner
                .track_grid_ref(self.grid_point(CellPoint { col: end, ..at })?)?,
            screen: self.inner.active_screen()?,
            clicks,
            extent: None,
        };
        self.view.selection = Some(anchor);
        if clicks >= 2 {
            self.extend_selection(at)?;
        }
        Ok(())
    }

    /// Extend from the original content anchor, even after scrolling/reflow.
    pub fn extend_selection(&mut self, at: CellPoint) -> Result<(), TermError> {
        let Some(anchor) = &self.view.selection else {
            return Ok(());
        };
        if !anchor.valid() {
            return self.clear_selection();
        }
        if anchor.screen != self.inner.active_screen()? {
            return Ok(());
        }
        let Some(origin) = anchor.start.point(PointSpace::Screen)? else {
            return self.clear_selection();
        };
        let (left, right) = self.selection_edge(at, anchor.clicks)?;
        let target = self.inner.grid_ref(self.grid_point(at)?)?;
        let Some(target) = self
            .inner
            .point_from_grid_ref(&target, PointSpace::Screen)?
        else {
            return Ok(());
        };
        let forward = (target.y, target.x) >= (origin.y, origin.x);
        let start = if forward { &anchor.start } else { &anchor.end };
        let Some(start) = start.snapshot(&self.inner)? else {
            return Ok(());
        };
        let end = self.inner.grid_ref(self.grid_point(CellPoint {
            col: if forward { right } else { left },
            ..at
        })?)?;
        self.inner
            .set_selection(Some(&Selection::new(start, end, false)))?;
        let extent = self.inner.track_grid_ref(self.grid_point(at)?)?;
        if let Some(anchor) = &mut self.view.selection {
            anchor.extent = Some(extent);
        }
        Ok(())
    }

    pub fn clear_selection(&mut self) -> Result<(), TermError> {
        self.inner.set_selection(None)?;
        self.view.selection = None;
        Ok(())
    }

    pub fn selection_text(&self) -> Result<String, TermError> {
        if self.view.selection.as_ref().is_none_or(|anchor| {
            !anchor.valid() || Some(anchor.screen) != self.inner.active_screen().ok()
        }) {
            return Ok(String::new());
        }
        let bytes = self
            .inner
            .format_selection_alloc(None, FormatOptions::new().with_unwrap(true).with_trim(true))?;
        Ok(bytes
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default())
    }

    pub(crate) fn validate_selection(&mut self) -> Result<(), TermError> {
        if self
            .view
            .selection
            .as_ref()
            .is_some_and(|anchor| !anchor.valid())
        {
            self.clear_selection()?;
        }
        if self
            .view
            .selection
            .as_ref()
            .is_none_or(|anchor| Some(anchor.screen) != self.inner.active_screen().ok())
        {
            self.inner.set_selection(None)?;
        }
        Ok(())
    }

    fn selection_edge(&self, at: CellPoint, clicks: usize) -> Result<(u16, u16), TermError> {
        let cols = self.cols()?;
        if clicks >= 3 {
            return Ok((0, cols - 1));
        }
        let row = self.row_text(at.row)?;
        let mut col = usize::from(at.col).min(row.len() - 1);
        while col > 0 && row[col].is_empty() {
            col -= 1;
        }
        let mut start = col;
        let mut end = col + 1;
        while end < row.len() && row[end].is_empty() {
            end += 1;
        }
        if clicks == 2 {
            let class = |s: &str| match s.chars().next() {
                Some(c) if c.is_alphanumeric() || c == '_' => 1,
                Some(c) if c.is_whitespace() => 2,
                _ => 3,
            };
            let kind = class(&row[col]);
            while start > 0 && (row[start - 1].is_empty() || class(&row[start - 1]) == kind) {
                start -= 1;
            }
            while end < row.len() && (row[end].is_empty() || class(&row[end]) == kind) {
                end += 1;
            }
        }
        Ok((start as u16, (end - 1) as u16))
    }

    /// Viewport text for pointer hit testing, never the per-frame renderer.
    /// Empty strings denote wide-cell tails; blanks contain one space.
    pub fn row_text(&self, row: u16) -> Result<Vec<String>, TermError> {
        let point = self.grid_point(CellPoint { row, col: 0 })?;
        self.read_text_row(point)
    }

    /// Read across soft wraps, including portions outside the viewport, so
    /// a URL is opened in full when its label crosses a viewport edge.
    pub fn logical_line(&self, row: u16) -> Result<TextLine, TermError> {
        let center = self.inner.scrollbar()?.offset as u32 + u32::from(row);
        let point = |y| Point::Screen(PointCoordinate { x: 0, y });
        let mut start = center;
        while start > 0
            && self
                .inner
                .grid_ref(point(start))?
                .row()?
                .is_wrap_continuation()?
        {
            start -= 1;
        }
        let cols = usize::from(self.cols()?);
        let mut cells = Vec::new();
        let mut y = start;
        loop {
            cells.extend(self.read_text_row(point(y))?);
            if !self.inner.grid_ref(point(y))?.row()?.is_wrapped()? {
                break;
            }
            y += 1;
        }
        let hovered = (center - start) as usize * cols;
        Ok(TextLine {
            cells,
            hovered: hovered..hovered + cols,
        })
    }

    fn read_text_row(&self, point: Point) -> Result<Vec<String>, TermError> {
        let mut result = Vec::with_capacity(usize::from(self.cols()?));
        let mut chars = vec!['\0'; 8];
        for col in 0..self.cols()? {
            let point = match point {
                Point::Viewport(at) => Point::Viewport(PointCoordinate { x: col, ..at }),
                Point::Screen(at) => Point::Screen(PointCoordinate { x: col, ..at }),
                _ => unreachable!("invariant: text rows use viewport or screen coordinates"),
            };
            let cell = self.inner.grid_ref(point)?;
            if matches!(
                cell.cell()?.wide()?,
                CellWide::SpacerTail | CellWide::SpacerHead
            ) {
                result.push(String::new());
                continue;
            }
            let len = match cell.graphemes(&mut chars) {
                Err(libghostty_vt::Error::OutOfSpace { required }) => {
                    chars.resize(required, '\0');
                    cell.graphemes(&mut chars)?
                }
                other => other?,
            };
            result.push(if len == 0 {
                " ".into()
            } else {
                chars[..len].iter().collect()
            });
        }
        Ok(result)
    }

    /// The OSC 8 URI at a viewport cell, including retained history.
    pub fn hyperlink(&self, at: CellPoint) -> Result<Option<String>, TermError> {
        let cell = self.inner.grid_ref(self.grid_point(at)?)?;
        let mut bytes = vec![0; 256];
        let len = match cell.hyperlink_uri(&mut bytes) {
            Err(libghostty_vt::Error::OutOfSpace { required }) => {
                bytes.resize(required, 0);
                cell.hyperlink_uri(&mut bytes)?
            }
            other => other?,
        };
        Ok((len > 0).then(|| String::from_utf8_lossy(&bytes[..len]).into_owned()))
    }
}
