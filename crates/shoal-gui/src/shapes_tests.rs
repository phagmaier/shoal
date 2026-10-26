use super::*;

const W: f32 = 8.0;
const H: f32 = 16.0;

/// Which pixels' centres the shapes cover: `#` full, `+` partly (shades),
/// `.` empty. Polygons use an even-odd test, strokes the distance to each
/// segment.
fn raster(ch: char, w: f32, h: f32) -> Vec<String> {
    let shapes = shapes(ch, w, h);
    (0..h as u32)
        .map(|y| {
            (0..w as u32)
                .map(|x| {
                    let p = (x as f32 + 0.5, y as f32 + 0.5);
                    let cover = shapes
                        .iter()
                        .map(|shape| match shape {
                            Shape::Rect { x, y, w, h, alpha } => {
                                let hit = (*x..x + w).contains(&p.0) && (*y..y + h).contains(&p.1);
                                if hit { *alpha } else { 0.0 }
                            }
                            Shape::Fill(points) => f32::from(u8::from(inside(points, p))),
                            Shape::Stroke { points, width } => f32::from(u8::from(
                                points
                                    .windows(2)
                                    .any(|s| distance(s[0], s[1], p) <= width / 2.0),
                            )),
                        })
                        .fold(0.0, f32::max);
                    match cover {
                        1.0 => '#',
                        0.0 => '.',
                        _ => '+',
                    }
                })
                .collect()
        })
        .collect()
}

fn inside(points: &[(f32, f32)], (px, py): (f32, f32)) -> bool {
    let mut odd = false;
    for (i, &(x1, y1)) in points.iter().enumerate() {
        let (x2, y2) = points[(i + 1) % points.len()];
        if (y1 > py) != (y2 > py) && px < x1 + (py - y1) * (x2 - x1) / (y2 - y1) {
            odd = !odd;
        }
    }
    odd
}

fn distance((x1, y1): (f32, f32), (x2, y2): (f32, f32), (px, py): (f32, f32)) -> f32 {
    let (dx, dy) = (x2 - x1, y2 - y1);
    let len = dx * dx + dy * dy;
    let t = if len == 0.0 {
        0.0
    } else {
        (((px - x1) * dx + (py - y1) * dy) / len).clamp(0.0, 1.0)
    };
    ((px - x1 - t * dx).powi(2) + (py - y1 - t * dy).powi(2)).sqrt()
}

/// Rows of a picture, top to bottom; a row of `n` repeats it.
fn picture(rows: &[(&str, usize)]) -> Vec<String> {
    rows.iter()
        .flat_map(|(row, n)| std::iter::repeat_n(row.to_string(), *n))
        .collect()
}

#[track_caller]
fn assert_draws(ch: char, expected: &[(&str, usize)]) {
    assert_eq!(
        raster(ch, W, H),
        picture(expected),
        "{ch} (U+{:04X})",
        ch as u32
    );
}

#[test]
fn light_and_heavy_lines_cross_at_the_centre() {
    assert_draws('─', &[("........", 8), ("########", 1), ("........", 7)]);
    assert_draws('│', &[("....#...", 16)]);
    assert_draws('┼', &[("....#...", 8), ("########", 1), ("....#...", 7)]);
    // Heavy is twice as thick, from the centre row up one.
    assert_draws('┏', &[("........", 7), ("...#####", 2), ("...##...", 7)]);
    assert_draws('┘', &[("....#...", 8), ("#####...", 1), ("........", 7)]);
    // Half lines start at the centre.
    assert_draws('╶', &[("........", 8), ("....####", 1), ("........", 7)]);
    assert_draws('╻', &[("........", 8), ("...##...", 8)]);
}

#[test]
fn double_lines_form_inner_and_outer_corners() {
    assert_draws(
        '═',
        &[
            ("........", 7),
            ("########", 1),
            ("........", 1),
            ("########", 1),
            ("........", 6),
        ],
    );
    assert_draws(
        '╔',
        &[
            ("........", 7),
            ("...#####", 1),
            ("...#....", 1),
            ("...#.###", 1),
            ("...#.#..", 6),
        ],
    );
    assert_draws(
        '╬',
        &[
            ("...#.#..", 7),
            ("####.###", 1),
            ("........", 1),
            ("####.###", 1),
            ("...#.#..", 6),
        ],
    );
    // A single line across a double one reaches its far line.
    assert_draws('╓', &[("........", 8), ("...#####", 1), ("...#.#..", 7)]);
    assert_draws(
        '╞',
        &[
            ("....#...", 7),
            ("....####", 1),
            ("....#...", 1),
            ("....####", 1),
            ("....#...", 6),
        ],
    );
}

#[test]
fn dashes_leave_gaps() {
    let row = &raster('┄', W, H)[8];
    assert_eq!(row.matches('#').count() + row.matches('.').count(), 8);
    // Three dashes: three runs of ink.
    assert_eq!(
        row.split('.').filter(|run| !run.is_empty()).count(),
        3,
        "{row}"
    );
    let column: String = raster('┆', W, H).iter().map(|row| &row[4..5]).collect();
    assert_eq!(
        column.split('.').filter(|run| !run.is_empty()).count(),
        3,
        "{column}"
    );
}

#[test]
fn blocks_fill_their_fraction_of_the_cell() {
    assert_draws('█', &[("########", 16)]);
    assert_draws('▀', &[("########", 8), ("........", 8)]);
    assert_draws('▄', &[("........", 8), ("########", 8)]);
    assert_draws('▁', &[("........", 14), ("########", 2)]);
    assert_draws('▏', &[("#.......", 16)]);
    assert_draws('▊', &[("######..", 16)]);
    assert_draws('▕', &[(".......#", 16)]);
    assert_draws('▚', &[("####....", 8), ("....####", 8)]);
    assert_draws('▟', &[("....####", 8), ("########", 8)]);
    // Shades are the full cell at partial alpha.
    assert_draws('▒', &[("++++++++", 16)]);
}

#[test]
fn arcs_and_diagonals_reach_the_edges_they_join() {
    let arc = raster('╭', W, H);
    assert_eq!(&arc[15][4..5], "#", "down");
    assert_eq!(&arc[8][7..8], "#", "right");
    assert!(arc[0].chars().all(|c| c == '.'), "nothing above");
    assert!(
        arc.iter().all(|row| row.starts_with('.')),
        "nothing at the left"
    );
    let cross = raster('╳', W, H);
    assert_eq!(&cross[0][0..1], "#");
    assert_eq!(&cross[0][7..8], "#");
    assert_eq!(&cross[15][0..1], "#");
    assert_eq!(&cross[15][7..8], "#");
}

#[test]
fn powerline_separators_span_the_cell_height() {
    // The flat side covers one edge (the corner pixels are half covered);
    // the point is mid-height at the other.
    let solid = raster('\u{e0b0}', W, H);
    assert!(solid[1..15].iter().all(|row| row.starts_with('#')));
    assert_eq!(&solid[7][6..7], "#");
    assert_eq!(&solid[0][7..8], ".");
    let left = raster('\u{e0b2}', W, H);
    assert!(left[1..15].iter().all(|row| row.ends_with('#')));
    assert_eq!(&left[7][1..2], "#");
    assert_eq!(&left[0][0..1], ".");
    let round = raster('\u{e0b4}', W, H);
    assert!(round.iter().all(|row| row.starts_with('#')));
    assert_eq!(&round[8][7..8], "#");
    // Thin separators leave the inside empty.
    let thin = raster('\u{e0b1}', W, H);
    assert_eq!(&thin[8][0..3], "...");
}

#[test]
fn every_character_in_the_ranges_draws_something() {
    for code in (0x2500..=0x259f).chain(0xe0b0..=0xe0b7) {
        let ch = char::from_u32(code).unwrap();
        assert!(is_shape(ch));
        assert!(!shapes(ch, W, H).is_empty(), "U+{code:04X}");
        // At twice the size too, where lines are two pixels thick.
        assert!(
            raster(ch, 16.0, 32.0)
                .iter()
                .any(|row| row.contains('#') || row.contains('+'))
        );
    }
    assert!(!is_shape('a'));
    assert!(!is_shape('\u{e0b8}'));
}
