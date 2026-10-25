//! Draw-list rows to GPUI primitives. Each row's resolved glyphs (or shaped
//! lines) and quads are cached by row version, so a clean row costs only
//! its `paint_glyph` calls.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    App, Bounds, Font, FontId, FontStyle, FontWeight, GlyphId, Hsla, Pixels, Rgba, ShapedLine,
    StrikethroughStyle, TextAlign, TextRun, UnderlineStyle, Window, fill, point, px, size,
};
use libghostty_vt::render::CursorVisualStyle;
use libghostty_vt::style::RgbColor;

use crate::draw::{Cursor, RowDraw, RunStyle};

pub const PAD: f32 = 6.0;

pub struct Metrics {
    pub font_size: Pixels,
    pub cell_w: Pixels,
    pub line_h: Pixels,
    /// Distance from the row's top to the text baseline.
    pub baseline: Pixels,
}

pub fn rgba(c: RgbColor, a: f32) -> Hsla {
    Rgba {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a,
    }
    .into()
}

struct Glyph {
    col: u16,
    font: FontId,
    id: GlyphId,
    emoji: bool,
    color: Hsla,
}

struct Line {
    col: u16,
    cells: u16,
    strike: bool,
    color: Hsla,
}

pub struct PaintedRow {
    version: u64,
    row: u16,
    bgs: Vec<(u16, u16, Hsla)>,
    glyphs: Vec<Glyph>,
    lines: Vec<Line>,
    shaped: Vec<(u16, ShapedLine)>,
}

type Found = Option<(FontId, GlyphId, bool)>;

/// (char, bold, italic) -> the font (after fallback) and glyph that draw it.
#[derive(Default)]
struct GlyphCache {
    ascii: Vec<Option<Found>>,
    other: HashMap<(char, bool, bool), Found>,
}

pub struct Painter {
    /// Place cached glyphs on the grid; otherwise shape each style run.
    pub glyph_mode: bool,
    glyphs: GlyphCache,
    rows: Vec<Rc<PaintedRow>>,
    /// Font size the cache was built for.
    font_size: Pixels,
}

fn font_for(font: &Font, s: RunStyle) -> Font {
    Font {
        weight: if s.bold { FontWeight::BOLD } else { FontWeight::NORMAL },
        style: if s.italic { FontStyle::Italic } else { FontStyle::Normal },
        ..font.clone()
    }
}

impl GlyphCache {
    fn get(&mut self, ch: char, s: RunStyle, font: &Font, m: &Metrics, window: &Window) -> Found {
        let slot = (ch as u32) < 128;
        let i = (ch as usize) * 4 + (s.bold as usize) * 2 + s.italic as usize;
        if slot {
            if self.ascii.is_empty() {
                self.ascii = vec![None; 128 * 4];
            }
            if let Some(found) = self.ascii[i] {
                return found;
            }
        } else if let Some(found) = self.other.get(&(ch, s.bold, s.italic)) {
            return *found;
        }
        let text = ch.to_string();
        let layout = window.text_system().layout_line(
            &text,
            m.font_size,
            &[TextRun {
                len: text.len(),
                font: font_for(font, s),
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        let found = layout
            .runs
            .first()
            .and_then(|run| run.glyphs.first().map(|g| (run.font_id, g.id, g.is_emoji)));
        if slot {
            self.ascii[i] = Some(found);
        } else {
            self.other.insert((ch, s.bold, s.italic), found);
        }
        found
    }
}

impl Painter {
    pub fn new(glyph_mode: bool) -> Self {
        Self {
            glyph_mode,
            glyphs: GlyphCache::default(),
            rows: Vec::new(),
            font_size: px(0.0),
        }
    }

    /// Bring the per-row cache up to date with the draw list. Returns the
    /// rows for this frame and how many were re-resolved.
    pub fn prepare(
        &mut self,
        rows: &[Rc<RowDraw>],
        font: &Font,
        m: &Metrics,
        opacity: f32,
        window: &Window,
    ) -> (Vec<Rc<PaintedRow>>, u16) {
        if self.font_size != m.font_size {
            self.font_size = m.font_size;
            self.glyphs = GlyphCache::default();
            self.rows.clear();
        }
        self.rows.truncate(rows.len());
        let mut redone = 0;
        for (i, r) in rows.iter().enumerate() {
            if self.rows.get(i).is_some_and(|p| p.version == r.version && r.version != 0) {
                continue;
            }
            let painted = Rc::new(self.resolve(r, font, m, opacity, window));
            redone += 1;
            if i < self.rows.len() {
                self.rows[i] = painted;
            } else {
                self.rows.push(painted);
            }
        }
        (self.rows.clone(), redone)
    }

    fn resolve(&mut self, r: &RowDraw, font: &Font, m: &Metrics, opacity: f32, window: &Window) -> PaintedRow {
        let mut out = PaintedRow {
            version: r.version,
            row: r.row,
            bgs: r.bgs.iter().map(|&(c, n, bg)| (c, n, rgba(bg, opacity))).collect(),
            glyphs: Vec::new(),
            lines: Vec::new(),
            shaped: Vec::new(),
        };
        for run in &r.runs {
            let s = run.style;
            let color = rgba(s.fg, if s.dim { 0.6 } else { 1.0 });
            if !self.glyph_mode || run.complex {
                let text_run = TextRun {
                    len: run.text.len(),
                    font: font_for(font, s),
                    color,
                    background_color: None,
                    underline: s.underline.then(|| UnderlineStyle {
                        thickness: px(1.0),
                        color: Some(color),
                        wavy: false,
                    }),
                    strikethrough: s.strike.then(|| StrikethroughStyle {
                        thickness: px(1.0),
                        color: Some(color),
                    }),
                };
                let line = window
                    .text_system()
                    .shape_line(run.text.clone().into(), m.font_size, &[text_run], None);
                out.shaped.push((run.col, line));
                continue;
            }
            let mut cells = 0u16;
            for ch in run.text.chars() {
                let col = run.col + cells;
                cells += 1;
                if ch == ' ' {
                    continue;
                }
                if let Some((font, id, emoji)) = self.glyphs.get(ch, s, font, m, window) {
                    out.glyphs.push(Glyph { col, font, id, emoji, color });
                }
            }
            for (on, strike) in [(s.underline, false), (s.strike, true)] {
                if on {
                    out.lines.push(Line { col: run.col, cells, strike, color });
                }
            }
        }
        out
    }
}

/// Paint one frame. Returns the number of glyphs submitted.
#[allow(clippy::too_many_arguments)]
pub fn paint(
    rows: &[Rc<PaintedRow>],
    cursor: Option<Cursor>,
    bounds: Bounds<Pixels>,
    m: &Metrics,
    accent: RgbColor,
    focused: bool,
    window: &mut Window,
    cx: &mut App,
) -> usize {
    let origin = bounds.origin + point(px(PAD), px(PAD));
    let cell = |col: u16, row: u16| origin + point(m.cell_w * col as f32, m.line_h * row as f32);
    for r in rows {
        for &(col, len, color) in &r.bgs {
            window.paint_quad(fill(
                Bounds::new(cell(col, r.row), size(m.cell_w * len as f32, m.line_h)),
                color,
            ));
        }
    }
    if let Some(c) = cursor {
        let w = if c.wide { m.cell_w * 2.0 } else { m.cell_w };
        let at = cell(c.col, c.row);
        let color = rgba(accent, 0.85);
        let quad = match (focused, c.style) {
            (false, _) | (_, CursorVisualStyle::BlockHollow) => {
                gpui::outline(Bounds::new(at, size(w, m.line_h)), color, gpui::BorderStyle::Solid)
            }
            (true, CursorVisualStyle::Bar) => fill(Bounds::new(at, size(px(2.0), m.line_h)), color),
            (true, CursorVisualStyle::Underline) => fill(
                Bounds::new(at + point(px(0.0), m.line_h - px(2.0)), size(w, px(2.0))),
                color,
            ),
            (true, _) => fill(Bounds::new(at, size(w, m.line_h)), color),
        };
        window.paint_quad(quad);
    }
    let mut glyphs = 0;
    for r in rows {
        let top = cell(0, r.row);
        for g in &r.glyphs {
            let at = top + point(m.cell_w * g.col as f32, m.baseline);
            let _ = if g.emoji {
                window.paint_emoji(at, g.font, g.id, m.font_size)
            } else {
                window.paint_glyph(at, g.font, g.id, m.font_size, g.color)
            };
        }
        glyphs += r.glyphs.len();
        for l in &r.lines {
            let y = if l.strike { m.baseline * 0.65 } else { m.baseline + px(1.5) };
            window.paint_quad(fill(
                Bounds::new(cell(l.col, r.row) + point(px(0.0), y), size(m.cell_w * l.cells as f32, px(1.0))),
                l.color,
            ));
        }
        for (col, line) in &r.shaped {
            let _ = line.paint(cell(*col, r.row), m.line_h, TextAlign::Left, None, window, cx);
        }
    }
    glyphs
}
