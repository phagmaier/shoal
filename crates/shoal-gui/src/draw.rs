//! A mirror's render state as a draw list (M1 spec §7.3): per row, merged
//! background spans, text runs, cells drawn as shapes, and underline and
//! strike lines; then the cursor. Pure and independent of GPUI, so it is
//! tested without a GPU.
//!
//! Rows are cached. The render state's dirty flags say which rows changed;
//! only those are rebuilt, and each rebuilt row gets a new `version` so the
//! painter can reuse its own work for the rest.

use std::rc::Rc;

use shoal_config::schema::Rgb;
use shoal_term::{
    CellStyle, CellWidth, Colors, CursorShape, Dirty, RowCells, Rows, TermError, Terminal,
    Underline,
};

use crate::{chrome::Color, shapes};

/// Dim (faint) text is drawn at this alpha.
const DIM_ALPHA: f32 = 0.6;

/// What the draw list takes from the window rather than the terminal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// `window.opacity`: explicit backgrounds are painted at it.
    pub opacity: f32,
    /// `cursor.color`, resolved through the palette.
    pub cursor: Rgb,
    /// Whether the pane has keyboard focus; an unfocused pane shows a
    /// hollow block.
    pub focused: bool,
}

/// Bold and italic pick the font face; everything else is color or lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Face {
    pub bold: bool,
    pub italic: bool,
}

/// Columns `col..col + cells` of a row in one color.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub col: u16,
    pub cells: u16,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunKind {
    /// One character per column; spaces draw nothing.
    Narrow,
    /// One character across two columns.
    Wide,
    /// One multi-codepoint grapheme cluster, shaped as a unit, across
    /// `cells` columns.
    Cluster { cells: u16 },
}

/// Text in one face and color, starting at `col`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub col: u16,
    pub kind: RunKind,
    pub face: Face,
    pub color: Color,
    pub text: String,
}

/// A box-drawing, block or powerline character, drawn as shapes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeCell {
    pub col: u16,
    pub ch: char,
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Underline(Underline),
    Strike,
    Overline,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line {
    pub kind: LineKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowDraw {
    /// The row's position on the viewport.
    pub y: u16,
    /// New whenever the row is rebuilt.
    pub version: u64,
    pub backgrounds: Vec<Span>,
    pub text: Vec<TextRun>,
    pub shapes: Vec<ShapeCell>,
    pub lines: Vec<Line>,
    /// Columns where a two-column character starts, for the cursor.
    pub wide: Vec<u16>,
}

impl RowDraw {
    fn empty(y: u16) -> Self {
        Self {
            y,
            version: 0,
            backgrounds: Vec::new(),
            text: Vec::new(),
            shapes: Vec::new(),
            lines: Vec::new(),
            wide: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorDraw {
    pub col: u16,
    pub row: u16,
    /// Two over a wide character.
    pub cells: u16,
    pub shape: CursorShape,
    pub color: Color,
    /// The color of text under a filled block.
    pub text: Option<Color>,
}

/// One frame of a mirror.
#[derive(Debug, Clone)]
pub struct DrawList {
    pub cols: u16,
    pub rows: Vec<Rc<RowDraw>>,
    pub cursor: Option<CursorDraw>,
    /// Rows rebuilt for this frame; equal to the row count on a full redraw.
    pub rebuilt: u16,
    pub selection: Vec<(u16, std::ops::Range<u16>)>,
    pub links: Vec<(u16, std::ops::Range<u16>)>,
    pub fraction: f32,
}

/// Builds a mirror's draw lists, reusing clean rows.
#[derive(Debug, Default)]
pub struct Builder {
    rows: Vec<Rc<RowDraw>>,
    cols: u16,
    /// The opacity the cached rows were built with.
    opacity: Option<f32>,
    version: u64,
}

impl Builder {
    pub fn invalidate(&mut self) {
        self.rows.clear();
    }
    /// Rebuild the rows the render state marks dirty, every row after a
    /// size or settings change, and reuse the rest.
    pub fn build(
        &mut self,
        terminal: &mut Terminal,
        settings: &Settings,
    ) -> Result<DrawList, TermError> {
        let mut frame = terminal.frame()?;
        let (cols, _) = frame.size()?;
        let rows = frame.captured_rows()?;
        let colors = frame.colors()?;
        let stale = self.cols != cols
            || self.rows.len() != usize::from(rows)
            || self.opacity != Some(settings.opacity);
        if stale {
            self.cols = cols;
            self.opacity = Some(settings.opacity);
            self.rows = (0..rows).map(|y| Rc::new(RowDraw::empty(y))).collect();
        }
        let which = if stale { Rows::All } else { Rows::Dirty };
        let mut rebuilt = 0;
        if stale || frame.dirty()? != Dirty::Clean {
            let (cache, version) = (&mut self.rows, &mut self.version);
            frame.read_rows(which, |row| {
                let Some(slot) = cache.get_mut(usize::from(row.y())) else {
                    return;
                };
                *version += 1;
                let mut draw = build_row(row, &colors, settings.opacity);
                draw.version = *version;
                *slot = Rc::new(draw);
                rebuilt += 1;
            })?;
        }
        let cursor = frame.cursor()?;
        let selection = frame.selection()?;
        // Every dirty row is cached now, so the flags can go.
        frame.mark_clean()?;
        let cursor = cursor.position.filter(|_| cursor.visible).and_then(|at| {
            let col = if at.wide_tail {
                at.col.checked_sub(1)?
            } else {
                at.col
            };
            let row = self.rows.get(usize::from(at.row))?;
            let wide = row.wide.binary_search(&col).is_ok();
            let shape = if settings.focused {
                cursor.shape
            } else {
                CursorShape::HollowBlock
            };
            Some(CursorDraw {
                col,
                row: at.row,
                cells: if wide { 2 } else { 1 },
                shape,
                color: Color {
                    rgb: settings.cursor,
                    alpha: 1.0,
                },
                text: (shape == CursorShape::Block).then_some(Color {
                    rgb: rgb(colors.background),
                    alpha: 1.0,
                }),
            })
        });
        Ok(DrawList {
            cols,
            rows: self.rows.clone(),
            cursor,
            rebuilt,
            selection,
            links: Vec::new(),
            fraction: 0.0,
        })
    }
}

fn rgb(color: shoal_term::Rgb) -> Rgb {
    Rgb([color.r, color.g, color.b])
}

/// A cell's text and background colors after inverse; `None` is the
/// default background, which is not painted.
fn resolve(style: &CellStyle, colors: &Colors) -> (Rgb, Option<Rgb>) {
    let fg = style.fg.unwrap_or(colors.foreground);
    if style.inverse {
        (rgb(style.bg.unwrap_or(colors.background)), Some(rgb(fg)))
    } else {
        (rgb(fg), style.bg.map(rgb))
    }
}

/// Add `span` to `spans`, merging it into the last one when they touch and
/// match.
fn push_span(spans: &mut Vec<Span>, span: Span) {
    match spans.last_mut() {
        Some(last) if last.color == span.color && last.col + last.cells == span.col => {
            last.cells += span.cells;
        }
        _ => spans.push(span),
    }
}

fn push_line(lines: &mut Vec<Line>, line: Line) {
    match lines.last_mut() {
        Some(last)
            if last.kind == line.kind
                && last.span.color == line.span.color
                && last.span.col + last.span.cells == line.span.col =>
        {
            last.span.cells += line.span.cells;
        }
        _ => lines.push(line),
    }
}

fn build_row(row: &RowCells, colors: &Colors, opacity: f32) -> RowDraw {
    let mut out = RowDraw::empty(row.y());
    // Where the last run ends, while it is a narrow run that can grow.
    let mut narrow_end = None;
    for (x, cell) in row.cells().iter().enumerate() {
        let cells = match cell.width {
            CellWidth::Narrow => 1,
            CellWidth::Wide => 2,
            CellWidth::SpacerTail | CellWidth::SpacerHead => continue,
        };
        let col = x as u16;
        if cells == 2 {
            out.wide.push(col);
        }
        let style = row.style(cell);
        let (fg, bg) = resolve(style, colors);
        if let Some(bg) = bg {
            push_span(
                &mut out.backgrounds,
                Span {
                    col,
                    cells,
                    color: Color {
                        rgb: bg,
                        alpha: opacity,
                    },
                },
            );
        }
        if style.invisible {
            continue;
        }
        let color = Color {
            rgb: fg,
            alpha: if style.faint { DIM_ALPHA } else { 1.0 },
        };
        let span = Span { col, cells, color };
        if style.underline != Underline::None {
            let color = match style.underline_color {
                Some(underline) => Color {
                    rgb: rgb(underline),
                    alpha: color.alpha,
                },
                None => color,
            };
            push_line(
                &mut out.lines,
                Line {
                    kind: LineKind::Underline(style.underline),
                    span: Span { color, ..span },
                },
            );
        }
        if style.strikethrough {
            push_line(
                &mut out.lines,
                Line {
                    kind: LineKind::Strike,
                    span,
                },
            );
        }
        if style.overline {
            push_line(
                &mut out.lines,
                Line {
                    kind: LineKind::Overline,
                    span,
                },
            );
        }
        let text = row.text(cell);
        let mut chars = text.chars();
        let (first, more) = (chars.next(), chars.next().is_some());
        let face = Face {
            bold: style.bold,
            italic: style.italic,
        };
        match first {
            None | Some(' ') => {
                // A blank keeps an open run going, so a line of words in one
                // style stays one run; trailing blanks are trimmed below.
                if narrow_end == Some(col)
                    && let Some(run) = out.text.last_mut()
                {
                    run.text.push(' ');
                    narrow_end = Some(col + 1);
                }
            }
            Some(ch) if !more && shapes::is_shape(ch) => {
                out.shapes.push(ShapeCell { col, ch, color });
            }
            Some(ch) => {
                let kind = match (more, cells) {
                    (true, _) => RunKind::Cluster { cells },
                    (false, 2) => RunKind::Wide,
                    (false, _) => RunKind::Narrow,
                };
                let touches = narrow_end == Some(col);
                narrow_end = (kind == RunKind::Narrow).then_some(col + 1);
                match out.text.last_mut() {
                    Some(run)
                        if kind == RunKind::Narrow
                            && touches
                            && run.face == face
                            && run.color == color =>
                    {
                        run.text.push(ch);
                    }
                    _ => {
                        trim(&mut out.text);
                        out.text.push(TextRun {
                            col,
                            kind,
                            face,
                            color,
                            text: text.to_owned(),
                        });
                    }
                }
            }
        }
    }
    trim(&mut out.text);
    out
}

/// Trailing blanks of the last run draw nothing.
fn trim(runs: &mut [TextRun]) {
    if let Some(run) = runs.last_mut() {
        let kept = run.text.trim_end_matches(' ').len();
        run.text.truncate(kept);
    }
}

#[cfg(test)]
#[path = "draw_tests.rs"]
mod tests;
