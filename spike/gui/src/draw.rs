//! Render state to a draw list: per row, merged background spans and text
//! runs that share a style. Painting happens elsewhere, so this part is
//! independent of GPUI (the M1 renderer keeps that split).
//!
//! Rows are cached. libghostty's render-state dirty flags say which rows
//! changed; only those are rebuilt, and each rebuilt row gets a new
//! `version` so the painter can reuse its own per-row work for the rest.

use std::rc::Rc;
use std::time::{Duration, Instant};

use libghostty_vt::render::{
    CellIterator, Colors, CursorVisualStyle, Dirty, RenderState, RowIteration, RowIterator,
};
use libghostty_vt::screen::{CellContentTag, CellWide};
use libghostty_vt::style::{self, RgbColor, Style, Underline};
use libghostty_vt::terminal::Terminal;

use crate::draw_ref::RefBuilder;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RunStyle {
    pub fg: RgbColor,
    pub dim: bool,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

#[derive(PartialEq, Debug)]
pub struct Run {
    pub col: u16,
    pub text: String,
    pub style: RunStyle,
    /// One cell holding a multi-codepoint grapheme; must be shaped.
    pub complex: bool,
}

#[derive(Debug)]
pub struct RowDraw {
    pub row: u16,
    /// Changes whenever the row is rebuilt; 0 for a placeholder.
    pub version: u64,
    /// (first column, number of columns, color) for non-default backgrounds.
    pub bgs: Vec<(u16, u16, RgbColor)>,
    pub runs: Vec<Run>,
}

/// Content equality; the version is bookkeeping.
impl PartialEq for RowDraw {
    fn eq(&self, other: &Self) -> bool {
        self.row == other.row && self.bgs == other.bgs && self.runs == other.runs
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Cursor {
    pub col: u16,
    pub row: u16,
    pub wide: bool,
    pub style: CursorVisualStyle,
}

#[derive(Default)]
pub struct Frame {
    pub cols: u16,
    pub rows: Vec<Rc<RowDraw>>,
    pub cursor: Option<Cursor>,
    /// Rows rebuilt for this frame.
    pub rebuilt: u16,
    /// Time in `RenderState::update` (part of the build time).
    pub update: Duration,
}

/// `SPIKE_VERIFY_ROWS=1`: the round-1 full rebuild of the same snapshot,
/// compared with the cached rows every frame.
struct Verify {
    reference: RefBuilder,
    frames: u64,
    rows: u64,
    bad_rows: u64,
}

pub struct Builder {
    state: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    text: String,
    cache: Vec<Rc<RowDraw>>,
    cols: u16,
    version: u64,
    verify: Option<Verify>,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            state: RenderState::new().expect("render state"),
            rows: RowIterator::new().expect("row iterator"),
            cells: CellIterator::new().expect("cell iterator"),
            text: String::with_capacity(16),
            cache: Vec::new(),
            cols: 0,
            version: 0,
            verify: std::env::var_os("SPIKE_VERIFY_ROWS").map(|_| Verify {
                reference: RefBuilder::new(),
                frames: 0,
                rows: 0,
                bad_rows: 0,
            }),
        }
    }

    /// Rebuild the rows libghostty marks dirty and reuse the rest.
    pub fn build(&mut self, term: &Terminal<'static, 'static>) -> Frame {
        let mut frame = Frame::default();
        let t = Instant::now();
        let Ok(snapshot) = self.state.update(term) else {
            return frame;
        };
        frame.update = t.elapsed();
        let Ok(colors) = snapshot.colors() else {
            return frame;
        };
        let cols = snapshot.cols().unwrap_or(0);
        let rows = snapshot.rows().unwrap_or(0) as usize;
        if self.cache.len() != rows || self.cols != cols {
            self.cols = cols;
            self.cache = (0..rows as u16)
                .map(|row| {
                    Rc::new(RowDraw {
                        row,
                        version: 0,
                        bgs: Vec::new(),
                        runs: Vec::new(),
                    })
                })
                .collect();
            let _ = snapshot.set_dirty(Dirty::Full);
        }
        frame.cols = cols;
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
        if let Ok(mut iter) = self.rows.update(&snapshot) {
            while let Some((y, row)) = iter.next_dirty() {
                let Some(slot) = self.cache.get_mut(y as usize) else {
                    break;
                };
                self.version += 1;
                let mut draw = build_row(&mut self.cells, &mut self.text, row, &colors, y);
                draw.version = self.version;
                *slot = Rc::new(draw);
                frame.rebuilt += 1;
            }
            // Only consume the dirty state once every dirty row is cached.
            let _ = snapshot.clean();
        }
        frame.rows = self.cache.clone();
        if let Some(v) = self.verify.as_mut() {
            let reference = v.reference.build(&snapshot);
            v.frames += 1;
            v.rows += reference.len() as u64;
            let bad: Vec<usize> = (0..reference.len().max(frame.rows.len()))
                .filter(|&i| reference.get(i) != frame.rows.get(i).map(|r| &**r))
                .collect();
            if !bad.is_empty() && v.bad_rows < 20 {
                eprintln!("SPIKE_VERIFY frame={} mismatched rows {bad:?}", v.frames);
                if v.bad_rows == 0 {
                    let i = bad[0];
                    eprintln!("SPIKE_VERIFY reference: {:?}", reference.get(i));
                    eprintln!("SPIKE_VERIFY cached:    {:?}", frame.rows.get(i));
                }
            }
            v.bad_rows += bad.len() as u64;
        }
        frame
    }

    pub fn verify_summary(&self) -> Option<String> {
        self.verify.as_ref().map(|v| {
            format!(
                "SPIKE_VERIFY frames={} rows={} mismatched_rows={}",
                v.frames, v.rows, v.bad_rows
            )
        })
    }
}

/// One row, with the same output as the round-1 builder (`draw_ref.rs`).
/// Each libghostty accessor is an FFI call (~10 ns), so this reads raw cells
/// in bulk, skips accessors that row flags make redundant, and queries
/// style and colors only when the style id changes.
fn build_row(
    cells: &mut CellIterator<'static>,
    text: &mut String,
    row: &RowIteration<'static, '_>,
    colors: &Colors,
    y: u16,
) -> RowDraw {
    let mut out = RowDraw {
        row: y,
        version: 0,
        bgs: Vec::new(),
        runs: Vec::new(),
    };
    let (Ok(raws), Ok(mut iter)) = (row.cells_raw(), cells.update(row)) else {
        return out;
    };
    let flags = row.raw_row().ok();
    let graphemes = flags.map_or(true, |r| r.has_grapheme_cluster().unwrap_or(true));
    // May report false positives, never false negatives.
    let styled = flags.map_or(true, |r| r.is_styled().unwrap_or(true));
    let plain = RunStyle {
        fg: colors.foreground,
        dim: false,
        bold: false,
        italic: false,
        underline: false,
        strike: false,
    };
    // Style ids are only comparable within one page, so within one row.
    // Cached: the run style and background after inverse.
    let mut cached: Option<(style::Id, RunStyle, Option<RgbColor>)> = None;
    // End column and "holds a char >= U+1100" for the last run.
    let mut tail = (0usize, false);
    for (x, raw) in raws.enumerate() {
        let col = x as u16;
        let wide = raw.wide().unwrap_or(CellWide::Narrow);
        if matches!(wide, CellWide::SpacerTail | CellWide::SpacerHead) {
            continue;
        }
        let cp = raw.codepoint().unwrap_or(0);
        let tag = if cp != 0 && !graphemes {
            CellContentTag::Codepoint
        } else {
            raw.content_tag().unwrap_or(CellContentTag::Codepoint)
        };
        let text_cell = matches!(
            tag,
            CellContentTag::Codepoint | CellContentTag::CodepointGrapheme
        );
        let id = if styled || !text_cell { raw.style_id().ok() } else { None };
        let (rs, bg) = match (cached, id) {
            _ if !styled && text_cell => (plain, None),
            (Some((cid, rs, bg)), Some(id)) if text_cell && cid == id => (rs, bg),
            _ => {
                let _ = iter.select(col);
                let style: Style = iter.style().unwrap_or_default();
                let mut fg = iter.fg_color().ok().flatten().unwrap_or(colors.foreground);
                // A background-only cell's color is its content, not its style.
                let mut bg = iter.bg_color().ok().flatten();
                if style.inverse {
                    let old_fg = fg;
                    fg = bg.unwrap_or(colors.background);
                    bg = Some(old_fg);
                }
                let rs = RunStyle {
                    fg,
                    dim: style.faint,
                    bold: style.bold,
                    italic: style.italic,
                    underline: style.underline != Underline::None,
                    strike: style.strikethrough,
                };
                if let (true, Some(id)) = (text_cell, id) {
                    cached = Some((id, rs, bg));
                }
                (rs, bg)
            }
        };
        let span = if wide == CellWide::Wide { 2 } else { 1 };
        if let Some(bg) = bg {
            match out.bgs.last_mut() {
                Some((start, len, c)) if *c == bg && *start + *len == col => *len += span,
                _ => out.bgs.push((col, span, bg)),
            }
        }
        // The cell's text: one char, or a whole grapheme cluster.
        let mut ch = char::from_u32(cp).filter(|_| cp != 0);
        let mut complex = false;
        if tag == CellContentTag::CodepointGrapheme {
            let _ = iter.select(col);
            let _ = iter.graphemes_utf8(text);
            let mut chars = text.chars();
            ch = chars.next();
            complex = chars.next().is_some();
        }
        let has_text = ch.is_some();
        let first = ch.unwrap_or(' ');
        // A run continues while the style matches and every cell is narrow;
        // wide glyphs get their own run so the grid stays aligned.
        let can_extend = span == 1
            && !complex
            && !tail.1
            && tail.0 == x
            && out.runs.last().is_some_and(|r| !r.complex && r.style == rs);
        if can_extend {
            if let Some(r) = out.runs.last_mut() {
                r.text.push(first);
                tail = (x + 1, first as u32 >= 0x1100);
            }
        } else if has_text || bg.is_some() || rs.underline || rs.strike {
            let run_text = if complex { text.clone() } else { first.to_string() };
            tail = (x + run_text.chars().count(), run_text.chars().any(|c| c as u32 >= 0x1100));
            out.runs.push(Run {
                col,
                text: run_text,
                style: rs,
                complex,
            });
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
    out
}
