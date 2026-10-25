//! Render state to a draw list: per row, merged background spans and text
//! runs that share a style. Painting happens elsewhere, so this part is
//! independent of GPUI (the M1 renderer keeps that split).

use libghostty_vt::render::{CellIterator, CursorVisualStyle, RenderState, RowIterator};
use libghostty_vt::screen::CellWide;
use libghostty_vt::style::{RgbColor, Underline};
use libghostty_vt::terminal::Terminal;

#[derive(Clone, Copy, PartialEq)]
pub struct RunStyle {
    pub fg: RgbColor,
    pub dim: bool,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

pub struct Run {
    pub col: u16,
    pub text: String,
    pub style: RunStyle,
    /// One cell holding a multi-codepoint grapheme; must be shaped.
    pub complex: bool,
}

pub struct RowDraw {
    pub row: u16,
    /// (first column, number of columns, color) for non-default backgrounds.
    pub bgs: Vec<(u16, u16, RgbColor)>,
    pub runs: Vec<Run>,
}

pub struct Cursor {
    pub col: u16,
    pub row: u16,
    pub wide: bool,
    pub style: CursorVisualStyle,
}

#[derive(Default)]
pub struct Frame {
    pub cols: u16,
    pub rows: Vec<RowDraw>,
    pub cursor: Option<Cursor>,
}

pub struct Builder {
    state: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    text: String,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            state: RenderState::new().expect("render state"),
            rows: RowIterator::new().expect("row iterator"),
            cells: CellIterator::new().expect("cell iterator"),
            text: String::with_capacity(16),
        }
    }

    /// Rebuild every row (the spike measures the full-redraw worst case).
    pub fn build(&mut self, term: &Terminal<'static, 'static>) -> Frame {
        let mut frame = Frame::default();
        let Ok(snapshot) = self.state.update(term) else {
            return frame;
        };
        let Ok(colors) = snapshot.colors() else {
            return frame;
        };
        frame.cols = snapshot.cols().unwrap_or(0);
        frame.cursor = snapshot.cursor_viewport().ok().flatten().and_then(|c| {
            snapshot.cursor_visible().ok().filter(|v| *v).map(|_| Cursor {
                col: c.x,
                row: c.y,
                wide: c.at_wide_tail,
                style: snapshot
                    .cursor_visual_style()
                    .unwrap_or(CursorVisualStyle::Block),
            })
        });
        let Ok(mut rows) = self.rows.update(&snapshot) else {
            return frame;
        };
        let mut y = 0u16;
        while let Some(row) = rows.next() {
            let mut out = RowDraw {
                row: y,
                bgs: Vec::new(),
                runs: Vec::new(),
            };
            if let Ok(mut cells) = self.cells.update(row) {
                let mut x = 0u16;
                while let Some(cell) = cells.next() {
                    let col = x;
                    x += 1;
                    let wide = cell
                        .raw_cell()
                        .and_then(|c| c.wide())
                        .unwrap_or(CellWide::Narrow);
                    if matches!(wide, CellWide::SpacerTail | CellWide::SpacerHead) {
                        continue;
                    }
                    let style = cell.style().unwrap_or_default();
                    let mut fg = cell.fg_color().ok().flatten().unwrap_or(colors.foreground);
                    let mut bg = cell.bg_color().ok().flatten();
                    if style.inverse {
                        let old_fg = fg;
                        fg = bg.unwrap_or(colors.background);
                        bg = Some(old_fg);
                    }
                    let span = if wide == CellWide::Wide { 2 } else { 1 };
                    if let Some(bg) = bg {
                        match out.bgs.last_mut() {
                            Some((start, len, c)) if *c == bg && *start + *len == col => {
                                *len += span
                            }
                            _ => out.bgs.push((col, span, bg)),
                        }
                    }
                    let has_text = cell.graphemes_len().unwrap_or(0) > 0;
                    if has_text {
                        let _ = cell.graphemes_utf8(&mut self.text);
                    } else {
                        self.text.clear();
                        self.text.push(' ');
                    }
                    let rs = RunStyle {
                        fg,
                        dim: style.faint,
                        bold: style.bold,
                        italic: style.italic,
                        underline: style.underline != Underline::None,
                        strike: style.strikethrough,
                    };
                    // A run continues while the style matches and every cell is
                    // narrow; wide glyphs get their own run so the grid stays aligned.
                    let complex = self.text.chars().nth(1).is_some();
                    let can_extend = span == 1
                        && !complex
                        && out.runs.last().is_some_and(|r| {
                            !r.complex
                                && r.style == rs
                                && r.col as usize + r.text.chars().count() == col as usize
                                && !r.text.chars().any(|c| c as u32 >= 0x1100)
                        });
                    if can_extend {
                        if let Some(r) = out.runs.last_mut() {
                            r.text.push_str(&self.text);
                        }
                    } else if has_text || bg.is_some() || rs.underline || rs.strike {
                        out.runs.push(Run {
                            col,
                            text: self.text.clone(),
                            style: rs,
                            complex,
                        });
                    }
                }
            }
            // Trailing spaces draw nothing.
            for r in &mut out.runs {
                if !r.style.underline && !r.style.strike {
                    let trimmed = r.text.trim_end_matches(' ').len();
                    r.text.truncate(trimmed);
                }
            }
            out.runs.retain(|r| !r.text.is_empty());
            frame.rows.push(out);
            y += 1;
        }
        frame
    }
}
