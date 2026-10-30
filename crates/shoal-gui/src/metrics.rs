//! Cell size and grid placement (M1 spec §7.2, §9.1), worked out from the
//! font's measurements so the GPUI code only measures. Sizes are logical
//! pixels; edges land on whole device pixels at the window's scale.

use shoal_model::{GridSize, Rect};

/// Space between a pane box's edge and its grid.
pub const GRID_INSET: f32 = 6.0;

/// `font.size` is in points, at 96 logical pixels per inch.
const PIXELS_PER_POINT: f32 = 96.0 / 72.0;

/// What the font measures at a size, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FontMeasure {
    /// The advance of a typical character (`M`).
    pub advance: f32,
    pub ascent: f32,
    /// Below the baseline, as a positive distance.
    pub descent: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellMetrics {
    /// The font size to shape and paint at.
    pub font_size: f32,
    pub width: f32,
    pub height: f32,
    /// From the cell's top to the text baseline.
    pub baseline: f32,
    /// From the cell's top to the underline's top.
    pub underline: f32,
    /// From the cell's top to the strike line's top.
    pub strike: f32,
    /// Underline and strike thickness.
    pub thickness: f32,
    /// A background with no background above or below it covers only this
    /// band: the text plus 1 px each side, centred in the row (§7.3).
    pub band_top: f32,
    pub band_height: f32,
    pub scale: f32,
}

impl CellMetrics {
    /// The top and height a background or powerline cell covers: it
    /// reaches the cell's top or bottom edge only where another background
    /// meets it there, so stacked rows join and a lone row is a pill.
    pub fn reach(&self, up: bool, down: bool) -> (f32, f32) {
        let top = if up { 0.0 } else { self.band_top };
        let bottom = if down {
            self.height
        } else {
            self.band_top + self.band_height
        };
        (top, bottom - top)
    }
}

/// The font size in logical pixels for `font.size` points.
pub fn font_size(points: f64) -> f32 {
    points as f32 * PIXELS_PER_POINT
}

/// Cell metrics for a font measured at [`font_size`], `font.line_height`
/// and the window's scale factor.
pub fn cell_metrics(
    font_size: f32,
    measure: FontMeasure,
    line_height: f64,
    scale: f32,
) -> CellMetrics {
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let snap = |value: f32| (value * scale).round().max(1.0) / scale;
    let FontMeasure {
        advance,
        ascent,
        descent,
    } = measure;
    let width = snap(advance);
    let height = snap((ascent + descent) * line_height as f32);
    // Extra line height is split above and below the text.
    let baseline = snap((height - ascent - descent) / 2.0 + ascent);
    let thickness = snap(font_size / 14.0);
    let underline = snap(baseline + descent / 2.0).min(height - thickness);
    let strike = snap(baseline - ascent * 0.3);
    let band_height = snap(ascent + descent + 2.0).min(height);
    // Rounded down, so the band stays inside the cell.
    let band_top = ((height - band_height) / 2.0 * scale).floor() / scale;
    CellMetrics {
        font_size,
        width,
        height,
        baseline,
        underline,
        strike,
        thickness,
        band_top,
        band_height,
        scale,
    }
}

/// The most columns or rows a pane may have (M1 spec §5.5).
const MAX_CELLS: u32 = 1000;

/// The grid a pane box holds (M1 spec §7.2): whole cells inside the inset,
/// at most 1000 × 1000. `None` if not even one cell fits, so a box squeezed
/// to nothing keeps its pane's size rather than reflowing it to 1 × 1.
pub fn grid_size(pane: Rect, cell: &CellMetrics) -> Option<GridSize> {
    let fit = |extent, size| match fit(extent, size, cell.scale) {
        0 => None,
        cells => Some(cells.min(MAX_CELLS) as u16),
    };
    Some(GridSize {
        cols: fit(pane.width, cell.width)?,
        rows: fit(pane.height, cell.height)?,
    })
}

/// Where a pane's grid starts, relative to the pane area: inside the box's
/// inset, with the pixels left over after whole cells split evenly.
pub fn grid_origin(pane: Rect, cell: &CellMetrics) -> (f32, f32) {
    let leftover = |extent, size| room(extent) - fit(extent, size, cell.scale) as f32 * size;
    let half = |value: f32| (value / 2.0 * cell.scale).floor() / cell.scale;
    (
        pane.x as f32 + GRID_INSET + half(leftover(pane.width, cell.width)),
        pane.y as f32 + GRID_INSET + half(leftover(pane.height, cell.height)),
    )
}

/// Where a grid of `rows` paints, relative to the pane area, as `(x, y,
/// width, height)`: inside the inset, and only over its rows, so the row
/// below a scrolled view shows only as it scrolls in, and a mirror larger
/// than its box is cut at the inset.
pub fn grid_clip(content: Rect, cell: &CellMetrics, rows: u16) -> (f32, f32, f32, f32) {
    let (_, top) = grid_origin(content, cell);
    let inner_bottom = content.y as f32 + GRID_INSET + room(content.height);
    let bottom = (top + f32::from(rows) * cell.height).min(inner_bottom);
    (
        content.x as f32 + GRID_INSET,
        top,
        room(content.width),
        (bottom - top).max(0.0),
    )
}

/// A box's extent less the inset on both sides.
fn room(extent: u32) -> f32 {
    (extent as f32 - 2.0 * GRID_INSET).max(0.0)
}

/// How many whole cells of `size` fit in a box's room, counted in device
/// pixels, where cells are whole: at a fractional scale the logical sizes
/// aren't exact, and 88 / 8.8 would floor to 9.
fn fit(extent: u32, size: f32, scale: f32) -> u32 {
    let device = |value: f32| (value * scale + 1e-3).floor();
    let cell = (size * scale).round().max(1.0);
    (device(room(extent)) / cell).floor() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// JetBrains Mono's proportions at 11 pt: advance 0.6 em, ascent 1.02
    /// em, descent 0.3 em.
    fn jetbrains() -> (f32, FontMeasure) {
        let size = font_size(11.0);
        (
            size,
            FontMeasure {
                advance: 0.6 * size,
                ascent: 1.02 * size,
                descent: 0.3 * size,
            },
        )
    }

    #[test]
    fn eleven_points_is_about_fifteen_pixels() {
        assert!((font_size(11.0) - 14.666_667).abs() < 1e-4);
    }

    #[test]
    fn cells_snap_to_whole_device_pixels() {
        let (size, measure) = jetbrains();
        // 8.8 × 19.36 at scale 1.
        let one = cell_metrics(size, measure, 1.0, 1.0);
        assert_eq!((one.width, one.height), (9.0, 19.0));
        // (19 - 19.36) / 2 + 14.96 = 14.78.
        assert_eq!(one.baseline, 15.0);
        assert_eq!(one.thickness, 1.0);
        assert_eq!(one.underline, 17.0, "15 + 2.2");
        assert_eq!(one.strike, 11.0, "15 - 4.49");
        // At scale 2: 17.6 × 38.72 device pixels round to 18 × 39.
        let two = cell_metrics(size, measure, 1.0, 2.0);
        assert_eq!((two.width, two.height), (9.0, 19.5));
        // The airy default keeps the same advance, with text centred in
        // the 27 px row: (27 - 19.36) / 2 + 14.96 = 18.78.
        let airy = cell_metrics(size, measure, 1.4, 1.0);
        assert_eq!((airy.width, airy.height, airy.baseline), (9.0, 27.0, 19.0));
        // A lone background covers the 19.36 px text plus 1 px each side,
        // 21 px, centred in the 27 px row; a neighbour above or below
        // extends it to that edge.
        assert_eq!((airy.band_top, airy.band_height), (3.0, 21.0));
        assert_eq!(airy.reach(false, false), (3.0, 21.0));
        assert_eq!(airy.reach(true, false), (0.0, 24.0));
        assert_eq!(airy.reach(false, true), (3.0, 24.0));
        assert_eq!(airy.reach(true, true), (0.0, 27.0));
        // Without extra line height the band is the whole 19 px cell.
        assert_eq!(one.reach(false, false), (0.0, 19.0));
        // Line height scales the row, with the text centred in it.
        let tall = cell_metrics(size, measure, 1.5, 1.0);
        assert_eq!(tall.height, 29.0);
        assert_eq!(tall.baseline, 20.0, "(29 - 19.36) / 2 + 14.96 = 19.78");
    }

    #[test]
    fn the_grid_holds_whole_cells_inside_the_inset() {
        let (size, measure) = jetbrains();
        let cell = cell_metrics(size, measure, 1.0, 1.0);
        let grid = |width, height| {
            let size = grid_size(
                Rect {
                    x: 8,
                    y: 8,
                    width,
                    height,
                },
                &cell,
            )?;
            Some((size.cols, size.rows))
        };
        // 412 - 12 = 400 holds 44 cells of 9 (396); 212 - 12 = 200 holds
        // 10 rows of 19 (190).
        assert_eq!(grid(412, 212), Some((44, 10)));
        // Exactly 44 cells and 10 rows fit; a pixel less loses one of each.
        assert_eq!(grid(408, 202), Some((44, 10)));
        assert_eq!(grid(407, 201), Some((43, 9)));
        // One cell fits in 21 × 31; a pixel less in either way, none.
        assert_eq!(grid(21, 31), Some((1, 1)));
        assert_eq!(grid(20, 31), None);
        assert_eq!(grid(21, 30), None);
        assert_eq!(grid(0, 0), None);
        // A pane never exceeds 1000 × 1000.
        assert_eq!(grid(20_000, 30_000), Some((1000, 1000)));
    }

    #[test]
    fn a_fullscreen_window_fills_with_cells() {
        // The user's 1920 × 1080 screen: the pane area is 1920 × 1046 below
        // the 34 px bar, its one box 1904 × 1030 inside the 8 px padding,
        // and 1892 × 1018 inside the inset: 210 columns of 9 (1890) and 53
        // rows of 19 (1007).
        let (size, measure) = jetbrains();
        let cell = cell_metrics(size, measure, 1.0, 1.0);
        let pane = Rect {
            x: 8,
            y: 8,
            width: 1904,
            height: 1030,
        };
        assert_eq!(
            grid_size(pane, &cell),
            Some(GridSize {
                cols: 210,
                rows: 53
            })
        );
        // The 2 and 11 pixels left over split evenly around the grid.
        assert_eq!(grid_origin(pane, &cell), (15.0, 19.0));
    }

    #[test]
    fn cells_are_counted_in_device_pixels_at_fractional_scales() {
        // At 1.5 an 8.8 px advance is 13 device pixels, so the cell is
        // 8.667 logical pixels, which f32 holds as slightly more. A 142 px
        // box has 130 px of room, 195 device pixels: exactly 15 cells, none
        // left over (130 / 8.667 in f32 floors to 14).
        let (size, measure) = jetbrains();
        let cell = cell_metrics(size, measure, 1.0, 1.5);
        assert_eq!(cell.width * 1.5, 13.0);
        let pane = Rect {
            x: 0,
            y: 0,
            width: 142,
            height: 100,
        };
        assert_eq!(grid_size(pane, &cell).map(|size| size.cols), Some(15));
        assert_eq!(grid_origin(pane, &cell).0, 6.0);
    }

    #[test]
    fn the_grid_is_centred_in_the_leftover_pixels() {
        let (size, measure) = jetbrains();
        let cell = cell_metrics(size, measure, 1.0, 1.0);
        // 412 - 12 = 400 wide holds 44 cells of 9 with 4 left: 2 each side.
        // 212 - 12 = 200 tall holds 10 rows of 19 with 10 left: 5 each side.
        let pane = Rect {
            x: 8,
            y: 8,
            width: 412,
            height: 212,
        };
        assert_eq!(grid_origin(pane, &cell), (16.0, 19.0));
        // An odd leftover keeps whole pixels: 3 left gives 1.
        let narrow = Rect { width: 411, ..pane };
        assert_eq!(grid_origin(narrow, &cell).0, 15.0);
        // A box smaller than its inset still starts inside it.
        let tiny = Rect {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        };
        assert_eq!(grid_origin(tiny, &cell), (6.0, 6.0));
    }

    #[test]
    fn the_grid_clips_to_its_rows() {
        let (size, measure) = jetbrains();
        let cell = cell_metrics(size, measure, 1.0, 1.0);
        let pane = Rect {
            x: 8,
            y: 8,
            width: 412,
            height: 212,
        };
        // Ten rows of 19 start at 19 (5 left over above): the clip ends at
        // 209, hiding the scrolled view's extra row in the 5 px below.
        assert_eq!(grid_clip(pane, &cell, 10), (14.0, 19.0, 400.0, 190.0));
        // A smaller mirror is clipped to its own rows.
        assert_eq!(grid_clip(pane, &cell, 4), (14.0, 19.0, 400.0, 76.0));
        // A larger one is cut where the inset starts, at 8 + 212 - 6.
        assert_eq!(grid_clip(pane, &cell, 30), (14.0, 19.0, 400.0, 195.0));
        let empty = Rect { height: 0, ..pane };
        assert_eq!(grid_clip(empty, &cell, 10).3, 0.0);
    }
}
