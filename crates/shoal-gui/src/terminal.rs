//! Painting a mirror's draw list with GPUI (M1 spec §7.3). Each row's
//! glyphs, shapes and quads are resolved once per row version, so a clean
//! row costs only its paint calls.

use std::{collections::HashMap, rc::Rc, time::Instant};

use gpui::{
    BorderStyle, Bounds, Hsla, PathBuilder, Pixels, Point, UnderlineStyle, Window,
    WindowTextSystem, fill, outline, point, px, size,
};
use shoal_term::{CursorShape, Underline};

use crate::{
    draw::{CursorDraw, DrawList, LineKind, RowDraw, RunKind},
    glyphs::{GlyphCache, Placed},
    metrics::CellMetrics,
    shapes::{self, Shape},
    view::hsla,
};

/// A glyph to paint, `x` from the row's left edge.
#[derive(Debug, Clone, Copy)]
struct Glyph {
    col: u16,
    x: f32,
    placed: Placed,
    color: Hsla,
}

#[derive(Debug, Clone)]
enum Mark {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: Hsla,
    },
    Wavy {
        x: f32,
        y: f32,
        w: f32,
        color: Hsla,
    },
}

#[derive(Debug)]
struct PaintedRow {
    version: u64,
    backgrounds: Vec<(f32, f32, Hsla)>,
    glyphs: Vec<Glyph>,
    shapes: Vec<(u16, Rc<[Shape]>, Hsla)>,
    lines: Vec<Mark>,
}

/// One pane's resolved rows, reused while their versions match.
#[derive(Debug, Default)]
pub struct Painter {
    rows: Vec<Rc<PaintedRow>>,
    /// The cell metrics and glyph cache generation the rows were resolved
    /// with; a new font family, size or line height starts over.
    metrics: Option<(CellMetrics, u64)>,
    shapes: HashMap<char, Rc<[Shape]>>,
}

/// Everything one pane's paint needs, owned so the paint closure can take it.
pub struct PaneFrame {
    rows: Vec<Rc<PaintedRow>>,
    cursor: Option<CursorDraw>,
    metrics: CellMetrics,
    /// When drawing this frame started, and whether every row was
    /// rebuilt, for the full-redraw log.
    started: Instant,
    full: Option<(u16, u16)>,
}

impl Painter {
    /// Resolve the rows that changed since the last frame. `started` is
    /// when building the draw list began.
    pub fn prepare(
        &mut self,
        list: &DrawList,
        metrics: CellMetrics,
        glyphs: &mut GlyphCache,
        text: &WindowTextSystem,
        started: Instant,
    ) -> PaneFrame {
        let key = Some((metrics, glyphs.generation()));
        if self.metrics != key {
            self.metrics = key;
            self.rows.clear();
            self.shapes.clear();
        }
        self.rows.truncate(list.rows.len());
        for (y, row) in list.rows.iter().enumerate() {
            if self
                .rows
                .get(y)
                .is_some_and(|done| done.version == row.version)
            {
                continue;
            }
            let painted = Rc::new(self.resolve(row, &metrics, glyphs, text));
            match self.rows.get_mut(y) {
                Some(slot) => *slot = painted,
                None => self.rows.push(painted),
            }
        }
        let rows = list.rows.len() as u16;
        PaneFrame {
            rows: self.rows.clone(),
            cursor: list.cursor,
            metrics,
            started,
            full: (rows > 0 && list.rebuilt == rows).then_some((list.cols, rows)),
        }
    }

    fn resolve(
        &mut self,
        row: &RowDraw,
        m: &CellMetrics,
        glyphs: &mut GlyphCache,
        text: &WindowTextSystem,
    ) -> PaintedRow {
        let x = |col: u16| f32::from(col) * m.width;
        let backgrounds = row
            .backgrounds
            .iter()
            .map(|span| {
                (
                    x(span.col),
                    f32::from(span.cells) * m.width,
                    hsla(span.color),
                )
            })
            .collect();
        let mut out_glyphs = Vec::new();
        let mut place = |col: u16, cells: u16, text_of: &str, run_color, face, center: bool| {
            let shaped = glyphs.get(text_of, face, text);
            let room = f32::from(cells) * m.width;
            let offset = if center {
                ((room - f32::from(shaped.width)) / 2.0).max(0.0)
            } else {
                0.0
            };
            for placed in shaped.glyphs.iter() {
                out_glyphs.push(Glyph {
                    col,
                    x: x(col) + offset + f32::from(placed.x),
                    placed: *placed,
                    color: run_color,
                });
            }
        };
        let mut buffer = [0; 4];
        for run in &row.text {
            let color = hsla(run.color);
            match run.kind {
                RunKind::Narrow => {
                    for (i, ch) in run.text.chars().enumerate() {
                        if ch != ' ' {
                            let col = run.col + i as u16;
                            place(col, 1, ch.encode_utf8(&mut buffer), color, run.face, false);
                        }
                    }
                }
                RunKind::Wide => place(run.col, 2, &run.text, color, run.face, true),
                RunKind::Cluster { cells } => {
                    place(run.col, cells, &run.text, color, run.face, true);
                }
            }
        }
        let (device_w, device_h) = (m.width * m.scale, m.height * m.scale);
        let shapes = row
            .shapes
            .iter()
            .map(|cell| {
                let drawn = self
                    .shapes
                    .entry(cell.ch)
                    .or_insert_with(|| shapes::shapes(cell.ch, device_w, device_h).into())
                    .clone();
                (cell.col, drawn, hsla(cell.color))
            })
            .collect();
        let mut lines = Vec::new();
        for line in &row.lines {
            let (left, w, color) = (
                x(line.span.col),
                f32::from(line.span.cells) * m.width,
                hsla(line.span.color),
            );
            let t = m.thickness;
            let rect = |x, y, w| Mark::Rect {
                x,
                y,
                w,
                h: t,
                color,
            };
            match line.kind {
                LineKind::Strike => lines.push(rect(left, m.strike, w)),
                LineKind::Overline => lines.push(rect(left, 0.0, w)),
                LineKind::Underline(Underline::Curly) => lines.push(Mark::Wavy {
                    x: left,
                    y: m.underline,
                    w,
                    color,
                }),
                LineKind::Underline(Underline::Double) => {
                    let lower = (m.underline + 2.0 * t).min(m.height - t);
                    lines.push(rect(left, lower - 2.0 * t, w));
                    lines.push(rect(left, lower, w));
                }
                LineKind::Underline(kind @ (Underline::Dotted | Underline::Dashed)) => {
                    let (on, step) = if kind == Underline::Dotted {
                        (t, 2.0 * t)
                    } else {
                        (3.0 * t, 5.0 * t)
                    };
                    let mut at = 0.0;
                    while at < w {
                        lines.push(rect(left + at, m.underline, on.min(w - at)));
                        at += step;
                    }
                }
                LineKind::Underline(_) => lines.push(rect(left, m.underline, w)),
            }
        }
        PaintedRow {
            version: row.version,
            backgrounds,
            glyphs: out_glyphs,
            shapes,
            lines,
        }
    }
}

/// Paint one pane's grid with its top-left cell at `origin`.
pub fn paint(frame: &PaneFrame, origin: Point<Pixels>, window: &mut Window) {
    let m = &frame.metrics;
    let top = |y: usize| origin.y + px(y as f32 * m.height);
    let cell = |x: f32, y: usize| point(origin.x + px(x), top(y));
    for (y, row) in frame.rows.iter().enumerate() {
        for &(x, w, color) in &row.backgrounds {
            window.paint_quad(fill(
                Bounds::new(cell(x, y), size(px(w), px(m.height))),
                color,
            ));
        }
    }
    let cursor = frame.cursor;
    let cursor_bounds = cursor.map(|c| {
        Bounds::new(
            cell(f32::from(c.col) * m.width, usize::from(c.row)),
            size(px(f32::from(c.cells) * m.width), px(m.height)),
        )
    });
    if let (Some(c), Some(bounds)) = (cursor, cursor_bounds) {
        let color = hsla(c.color);
        let bar = (2.0 * m.scale).round().max(1.0) / m.scale;
        match c.shape {
            CursorShape::Block => window.paint_quad(fill(bounds, color)),
            CursorShape::Bar => {
                window.paint_quad(fill(
                    Bounds::new(bounds.origin, size(px(bar), bounds.size.height)),
                    color,
                ));
            }
            CursorShape::Underline => window.paint_quad(fill(
                Bounds::new(
                    bounds.origin + point(px(0.0), bounds.size.height - px(bar)),
                    size(bounds.size.width, px(bar)),
                ),
                color,
            )),
            CursorShape::HollowBlock => {}
        }
    }
    // Text under a filled block takes the cursor's text color.
    let under = |y: usize, col: u16, color: Hsla| match cursor {
        Some(CursorDraw {
            row,
            col: at,
            cells,
            text: Some(text),
            ..
        }) if usize::from(row) == y && (at..at + cells).contains(&col) => hsla(text),
        _ => color,
    };
    for (y, row) in frame.rows.iter().enumerate() {
        for &(col, ref drawn, color) in &row.shapes {
            let color = under(y, col, color);
            let at = cell(f32::from(col) * m.width, y);
            paint_shapes(drawn, at, m.scale, color, window);
        }
        let baseline = top(y) + px(m.baseline);
        for glyph in &row.glyphs {
            let at = point(origin.x + px(glyph.x), baseline);
            let Placed {
                font,
                glyph: id,
                emoji,
                ..
            } = glyph.placed;
            let _ = if emoji {
                window.paint_emoji(at, font, id, px(m.font_size))
            } else {
                window.paint_glyph(
                    at,
                    font,
                    id,
                    px(m.font_size),
                    under(y, glyph.col, glyph.color),
                )
            };
        }
        for mark in &row.lines {
            match *mark {
                Mark::Rect {
                    x,
                    y: dy,
                    w,
                    h,
                    color,
                } => window.paint_quad(fill(
                    Bounds::new(cell(x, y) + point(px(0.0), px(dy)), size(px(w), px(h))),
                    color,
                )),
                Mark::Wavy { x, y: dy, w, color } => window.paint_underline(
                    cell(x, y) + point(px(0.0), px(dy)),
                    px(w),
                    &UnderlineStyle {
                        thickness: px(m.thickness),
                        color: Some(color),
                        wavy: true,
                    },
                ),
            }
        }
    }
    if let (Some(c), Some(bounds)) = (cursor, cursor_bounds)
        && c.shape == CursorShape::HollowBlock
    {
        window.paint_quad(outline(bounds, hsla(c.color), BorderStyle::Solid));
    }
    if let Some((cols, rows)) = frame.full {
        tracing::debug!(
            cols,
            rows,
            ms = frame.started.elapsed().as_secs_f64() * 1000.0,
            "full redraw"
        );
    }
}

fn paint_shapes(drawn: &[Shape], at: Point<Pixels>, scale: f32, color: Hsla, window: &mut Window) {
    let to = |(x, y): (f32, f32)| at + point(px(x / scale), px(y / scale));
    for shape in drawn {
        match shape {
            Shape::Rect { x, y, w, h, alpha } => window.paint_quad(fill(
                Bounds::new(to((*x, *y)), size(px(w / scale), px(h / scale))),
                Hsla {
                    a: color.a * alpha,
                    ..color
                },
            )),
            Shape::Fill(points) | Shape::Stroke { points, .. } => {
                let mut path = match shape {
                    Shape::Stroke { width, .. } => PathBuilder::stroke(px(width / scale)),
                    _ => PathBuilder::fill(),
                };
                let mut points = points.iter().copied().map(to);
                if let Some(first) = points.next() {
                    path.move_to(first);
                }
                for next in points {
                    path.line_to(next);
                }
                if matches!(shape, Shape::Fill(_)) {
                    path.close();
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, color);
                }
            }
        }
    }
}
