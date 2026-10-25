//! The round-1 (measured) full-rebuild row loop, kept verbatim apart from
//! its output types. `SPIKE_VERIFY_ROWS=1` compares the cached builder in
//! `draw.rs` against it every frame, on the same render-state snapshot: a
//! second `RenderState` on the same terminal would go stale, because each
//! update consumes the terminal's own dirty rows. Never used for timing.

use libghostty_vt::render::{CellIterator, RowIterator, Snapshot};
use libghostty_vt::screen::CellWide;
use libghostty_vt::style::Underline;

use crate::draw::{RowDraw, Run, RunStyle};

pub struct RefBuilder {
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    text: String,
}

impl RefBuilder {
    pub fn new() -> Self {
        Self {
            rows: RowIterator::new().expect("row iterator"),
            cells: CellIterator::new().expect("cell iterator"),
            text: String::with_capacity(16),
        }
    }

    /// Rebuild every row of the snapshot.
    pub fn build(&mut self, snapshot: &Snapshot<'static, '_>) -> Vec<RowDraw> {
        let mut frame_rows = Vec::new();
        let Ok(colors) = snapshot.colors() else {
            return frame_rows;
        };
        let Ok(mut rows) = self.rows.update(snapshot) else {
            return frame_rows;
        };
        let mut y = 0u16;
        while let Some(row) = rows.next() {
            let mut out = RowDraw {
                row: y,
                version: 0,
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
            frame_rows.push(out);
            y += 1;
        }
        frame_rows
    }
}
