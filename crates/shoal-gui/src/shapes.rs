//! Box drawing (U+2500–257F), block elements (U+2580–259F) and powerline
//! separators (U+E0B0–E0B7) as shapes, not glyphs (M1 spec §7.3), so they
//! fill the cell exactly and join their neighbours in any font. Pure: the
//! painter turns each [`Shape`] into GPUI primitives.
//!
//! Geometry is in device pixels from the cell's top-left corner, so lines
//! land on whole pixels at any scale.

/// One piece of a character's drawing.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// A filled rectangle; `alpha` scales the text color (shades use it).
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        alpha: f32,
    },
    /// A filled polygon.
    Fill(Vec<(f32, f32)>),
    /// A polyline stroked `width` wide.
    Stroke { points: Vec<(f32, f32)>, width: f32 },
}

/// Whether `ch` is drawn by [`shapes`].
pub fn is_shape(ch: char) -> bool {
    matches!(ch, '\u{2500}'..='\u{259f}' | '\u{e0b0}'..='\u{e0b7}')
}

/// Whether `ch` is a powerline separator or cap, which ends with the
/// backgrounds it joins rather than filling the cell (§7.3).
pub fn is_powerline(ch: char) -> bool {
    matches!(ch, '\u{e0b0}'..='\u{e0b7}')
}

/// The light line thickness for a cell `h` device pixels tall.
fn light(h: f32) -> f32 {
    (h / 16.0).round().max(1.0)
}

/// The shapes that draw `ch` in a `w` × `h` cell (device pixels). Empty for
/// characters [`is_shape`] rejects.
pub fn shapes(ch: char, w: f32, h: f32) -> Vec<Shape> {
    let code = ch as u32;
    match code {
        0x2500..=0x257f => box_drawing(code, w, h),
        0x2580..=0x259f => block(code, w, h),
        0xe0b0..=0xe0b7 => powerline(code, w, h),
        _ => Vec::new(),
    }
}

/// How heavy one arm of a box-drawing character is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Weight {
    None,
    Light,
    Heavy,
    Double,
}

/// Up, right, down and left arms of U+2500–254B, U+2550–256C and
/// U+2574–257F: `0` none, `1` light, `2` heavy, `3` double. `-` marks
/// characters drawn another way (dashes, arcs, diagonals).
const ARMS: [&str; 128] = [
    // 2500
    "0101", "0202", "1010", "2020", "-", "-", "-", "-", //
    "-", "-", "-", "-", "0110", "0210", "0120", "0220", //
    // 2510
    "0011", "0012", "0021", "0022", "1100", "1200", "2100", "2200", //
    "1001", "1002", "2001", "2002", "1110", "1210", "2110", "1120", //
    // 2520
    "2120", "2210", "1220", "2220", "1011", "1012", "2011", "1021", //
    "2021", "2012", "1022", "2022", "0111", "0112", "0211", "0212", //
    // 2530
    "0121", "0122", "0221", "0222", "1101", "1102", "1201", "1202", //
    "2101", "2102", "2201", "2202", "1111", "1112", "1211", "1212", //
    // 2540
    "2111", "1121", "2121", "2112", "2211", "1122", "1221", "2212", //
    "1222", "2122", "2221", "2222", "-", "-", "-", "-", //
    // 2550
    "0303", "3030", "0310", "0130", "0330", "0013", "0031", "0033", //
    "1300", "3100", "3300", "1003", "3001", "3003", "1310", "3130", //
    // 2560
    "3330", "1013", "3031", "3033", "0313", "0131", "0333", "1303", //
    "3101", "3303", "1313", "3131", "3333", "-", "-", "-", //
    // 2570
    "-", "-", "-", "-", "0001", "1000", "0100", "0010", //
    "0002", "2000", "0200", "0020", "0201", "1020", "0102", "2010", //
];

fn weight(digit: u8) -> Weight {
    match digit {
        b'1' => Weight::Light,
        b'2' => Weight::Heavy,
        b'3' => Weight::Double,
        _ => Weight::None,
    }
}

fn box_drawing(code: u32, w: f32, h: f32) -> Vec<Shape> {
    let index = (code - 0x2500) as usize;
    let t = light(h);
    match code {
        // Triple and quadruple dashes: light or heavy, horizontal or vertical.
        0x2504..=0x250b => {
            let n = code - 0x2504;
            let dashes = if n < 4 { 3 } else { 4 };
            let heavy = n % 2 == 1;
            let vertical = n % 4 >= 2;
            dashed(dashes, heavy, vertical, w, h)
        }
        // Double dashes.
        0x254c..=0x254f => {
            let n = code - 0x254c;
            dashed(2, n % 2 == 1, n >= 2, w, h)
        }
        0x256d..=0x2570 => arc(code, w, h),
        0x2571..=0x2573 => {
            let mut out = Vec::new();
            if code != 0x2572 {
                out.push(Shape::Stroke {
                    points: vec![(w, 0.0), (0.0, h)],
                    width: t,
                });
            }
            if code != 0x2571 {
                out.push(Shape::Stroke {
                    points: vec![(0.0, 0.0), (w, h)],
                    width: t,
                });
            }
            out
        }
        _ => {
            let digits = ARMS[index].as_bytes();
            let [up, right, down, left] = [0, 1, 2, 3].map(|i| weight(digits[i]));
            lines([up, right, down, left], w, h)
        }
    }
}

/// The light-line rectangle for an arm from `from` to `to` along one axis,
/// centred on `at` across it.
fn bar(vertical: bool, at: f32, thickness: f32, from: f32, to: f32) -> Shape {
    let (start, len) = (from.min(to), (to - from).abs());
    let across = at - (thickness / 2.0).floor();
    if vertical {
        Shape::Rect {
            x: across,
            y: start,
            w: thickness,
            h: len,
            alpha: 1.0,
        }
    } else {
        Shape::Rect {
            x: start,
            y: across,
            w: len,
            h: thickness,
            alpha: 1.0,
        }
    }
}

/// Where a line running from the centre toward `sign` (+1 or -1) starts so
/// that it covers a line across it centred on `at`, `thickness` wide.
fn meet(sign: f32, at: f32, thickness: f32) -> f32 {
    let low = at - (thickness / 2.0).floor();
    if sign > 0.0 { low } else { low + thickness }
}

/// Lines from the centre to each edge with an arm, joined where they meet.
/// A double line is two light lines a light width either side of the
/// centre line.
fn lines(arms: [Weight; 4], w: f32, h: f32) -> Vec<Shape> {
    let [up, right, down, left] = arms;
    let t = light(h);
    let thick = |weight| match weight {
        Weight::None => 0.0,
        Weight::Heavy => 2.0 * t,
        Weight::Light | Weight::Double => t,
    };
    let (cx, cy) = ((w / 2.0).floor(), (h / 2.0).floor());
    let mut out = Vec::new();
    // Horizontal arms cross the vertical ones and the other way round:
    // (vertical, the arms along the axis with their direction, the arms
    // across it, the centre along and across, the far edge).
    for (vertical, along, [before, after], (c, c_across), edge) in [
        (false, [(left, -1.0), (right, 1.0)], [up, down], (cx, cy), w),
        (true, [(up, -1.0), (down, 1.0)], [left, right], (cy, cx), h),
    ] {
        let double_across = before == Weight::Double || after == Weight::Double;
        let single_across = thick(before).max(thick(after));
        for (arm, sign) in along {
            if arm == Weight::None {
                continue;
            }
            let end = if sign > 0.0 { edge } else { 0.0 };
            let offsets: &[f32] = if arm == Weight::Double {
                &[-t, t]
            } else {
                &[0.0]
            };
            for &offset in offsets {
                let from = if double_across {
                    // A line beside a double arm across stops at that arm's
                    // near line, forming the inner corner; any other line
                    // goes on to the far line, forming the outer one.
                    let beside = if offset < 0.0 { before } else { after };
                    let near = arm == Weight::Double && beside == Weight::Double;
                    let at = if near { c + sign * t } else { c - sign * t };
                    meet(sign, at, t)
                } else if single_across > 0.0 {
                    meet(sign, c, single_across)
                } else {
                    c
                };
                out.push(bar(vertical, c_across + offset, thick(arm), from, end));
            }
        }
    }
    out
}

/// `n` dashes along the cell, with gaps between them.
fn dashed(n: u32, heavy: bool, vertical: bool, w: f32, h: f32) -> Vec<Shape> {
    let t = light(h) * if heavy { 2.0 } else { 1.0 };
    let (len, at) = if vertical {
        (h, (w / 2.0).floor())
    } else {
        (w, (h / 2.0).floor())
    };
    let step = len / n as f32;
    let gap = (step / 3.0).round().max(1.0);
    (0..n)
        .map(|i| {
            let from = (i as f32 * step + gap / 2.0).round();
            let to = ((i + 1) as f32 * step - gap / 2.0).round();
            bar(vertical, at, t, from, to.max(from + 1.0))
        })
        .collect()
}

/// ╭ ╮ ╯ ╰: a light line that turns through a quarter curve at the centre.
fn arc(code: u32, w: f32, h: f32) -> Vec<Shape> {
    let t = light(h);
    let (cx, cy) = (
        (w / 2.0).floor() + 0.5 * (t % 2.0),
        (h / 2.0).floor() + 0.5 * (t % 2.0),
    );
    let r = cx.min(cy).min(w - cx);
    // The vertical end (top or bottom edge) and horizontal end (left or
    // right edge) the arc joins.
    let (y_end, x_end) = match code {
        0x256d => (h, w),   // down and right
        0x256e => (h, 0.0), // down and left
        0x256f => (0.0, 0.0),
        _ => (0.0, w),
    };
    let ys = if y_end > cy { 1.0 } else { -1.0 };
    let xs = if x_end > cx { 1.0 } else { -1.0 };
    let mut points = vec![(cx, y_end), (cx, cy + ys * r)];
    // A quarter circle, from the vertical arm to the horizontal one.
    const STEPS: u32 = 8;
    for i in 1..STEPS {
        let a = std::f32::consts::FRAC_PI_2 * i as f32 / STEPS as f32;
        points.push((cx + xs * r * (1.0 - a.cos()), cy + ys * r * (1.0 - a.sin())));
    }
    points.push((cx + xs * r, cy));
    points.push((x_end, cy));
    vec![Shape::Stroke { points, width: t }]
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Shape {
    Shape::Rect {
        x,
        y,
        w,
        h,
        alpha: 1.0,
    }
}

fn block(code: u32, w: f32, h: f32) -> Vec<Shape> {
    let eighth_h = |n: f32| (h * n / 8.0).round();
    let eighth_w = |n: f32| (w * n / 8.0).round();
    let (hw, hh) = ((w / 2.0).round(), (h / 2.0).round());
    let quadrants = |ul: bool, ur: bool, ll: bool, lr: bool| {
        [
            (ul, rect(0.0, 0.0, hw, hh)),
            (ur, rect(hw, 0.0, w - hw, hh)),
            (ll, rect(0.0, hh, hw, h - hh)),
            (lr, rect(hw, hh, w - hw, h - hh)),
        ]
        .into_iter()
        .filter_map(|(on, shape)| on.then_some(shape))
        .collect()
    };
    let shade = |alpha| {
        vec![Shape::Rect {
            x: 0.0,
            y: 0.0,
            w,
            h,
            alpha,
        }]
    };
    match code {
        0x2580 => vec![rect(0.0, 0.0, w, hh)],
        // Lower one eighth to full block.
        0x2581..=0x2588 => {
            let top = h - eighth_h((code - 0x2580) as f32);
            vec![rect(0.0, top, w, h - top)]
        }
        // Left seven eighths to left one eighth.
        0x2589..=0x258f => vec![rect(0.0, 0.0, eighth_w((0x2590 - code) as f32), h)],
        0x2590 => vec![rect(hw, 0.0, w - hw, h)],
        0x2591 => shade(0.25),
        0x2592 => shade(0.5),
        0x2593 => shade(0.75),
        0x2594 => vec![rect(0.0, 0.0, w, eighth_h(1.0))],
        0x2595 => {
            let left = w - eighth_w(1.0);
            vec![rect(left, 0.0, w - left, h)]
        }
        0x2596 => quadrants(false, false, true, false),
        0x2597 => quadrants(false, false, false, true),
        0x2598 => quadrants(true, false, false, false),
        0x2599 => quadrants(true, false, true, true),
        0x259a => quadrants(true, false, false, true),
        0x259b => quadrants(true, true, true, false),
        0x259c => quadrants(true, true, false, true),
        0x259d => quadrants(false, true, false, false),
        0x259e => quadrants(false, true, true, false),
        _ => quadrants(false, true, true, true),
    }
}

/// Points along half an ellipse centred on `(cx, h / 2)`, bulging toward
/// `toward` (+1 right, -1 left), from the top edge to the bottom.
fn half_ellipse(cx: f32, toward: f32, w: f32, h: f32) -> Vec<(f32, f32)> {
    const STEPS: u32 = 16;
    (0..=STEPS)
        .map(|i| {
            let a = std::f32::consts::PI * i as f32 / STEPS as f32;
            (cx + toward * w * a.sin(), h / 2.0 - h / 2.0 * a.cos())
        })
        .collect()
}

fn powerline(code: u32, w: f32, h: f32) -> Vec<Shape> {
    let t = light(h);
    let mid = h / 2.0;
    match code {
        0xe0b0 => vec![Shape::Fill(vec![(0.0, 0.0), (w, mid), (0.0, h)])],
        0xe0b1 => vec![Shape::Stroke {
            points: vec![(0.0, 0.0), (w - t / 2.0, mid), (0.0, h)],
            width: t,
        }],
        0xe0b2 => vec![Shape::Fill(vec![(w, 0.0), (0.0, mid), (w, h)])],
        0xe0b3 => vec![Shape::Stroke {
            points: vec![(w, 0.0), (t / 2.0, mid), (w, h)],
            width: t,
        }],
        0xe0b4 => vec![Shape::Fill(half_ellipse(0.0, 1.0, w, h))],
        0xe0b5 => vec![Shape::Stroke {
            points: half_ellipse(0.0, 1.0, w - t / 2.0, h),
            width: t,
        }],
        0xe0b6 => vec![Shape::Fill(half_ellipse(w, -1.0, w, h))],
        _ => vec![Shape::Stroke {
            points: half_ellipse(w, -1.0, w - t / 2.0, h),
            width: t,
        }],
    }
}

#[cfg(test)]
#[path = "shapes_tests.rs"]
mod tests;
