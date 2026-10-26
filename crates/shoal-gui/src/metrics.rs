//! Cell size and grid placement (M1 spec §7.2, §9.1), worked out from the
//! font's measurements so the GPUI code only measures. Sizes are logical
//! pixels; edges land on whole device pixels at the window's scale.

use shoal_model::Rect;

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
    pub scale: f32,
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
    CellMetrics {
        font_size,
        width,
        height,
        baseline,
        underline,
        strike,
        thickness,
        scale,
    }
}

/// Where a pane's grid starts, relative to the pane area: inside the box's
/// inset, with the pixels left over after whole cells split evenly.
pub fn grid_origin(pane: Rect, cell: &CellMetrics) -> (f32, f32) {
    let inner = |extent: u32| (extent as f32 - 2.0 * GRID_INSET).max(0.0);
    let leftover = |room: f32, size: f32| room - (room / size).floor() * size;
    let half = |value: f32| (value / 2.0 * cell.scale).floor() / cell.scale;
    (
        pane.x as f32 + GRID_INSET + half(leftover(inner(pane.width), cell.width)),
        pane.y as f32 + GRID_INSET + half(leftover(inner(pane.height), cell.height)),
    )
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
        // Line height scales the row, with the text centred in it.
        let tall = cell_metrics(size, measure, 1.5, 1.0);
        assert_eq!(tall.height, 29.0);
        assert_eq!(tall.baseline, 20.0, "(29 - 19.36) / 2 + 14.96 = 19.78");
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
}
