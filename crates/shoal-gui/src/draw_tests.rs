//! Golden draw lists from fixture terminal states. Each fixture is fed into
//! a terminal as a program would write it; the snapshot is the draw list in
//! a readable form, reviewed by hand.

use std::fmt::Write as _;

use super::*;

const ACCENT: Rgb = Rgb([0x7a, 0xa2, 0xf7]);

fn settings(focused: bool) -> Settings {
    Settings {
        cursor: ACCENT,
        focused,
    }
}

fn color(color: Color) -> String {
    let [r, g, b] = color.rgb.0;
    if color.alpha == 1.0 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}@{}", color.alpha)
    }
}

fn span(span: &Span) -> String {
    format!("{}+{} {}", span.col, span.cells, color(span.color))
}

/// The draw list as text: rows that draw anything, then the cursor.
fn describe(list: &DrawList) -> String {
    let mut out = format!("{} cols, {} rows rebuilt\n", list.cols, list.rebuilt);
    for row in &list.rows {
        if row.backgrounds.is_empty()
            && row.text.is_empty()
            && row.shapes.is_empty()
            && row.lines.is_empty()
        {
            continue;
        }
        writeln!(out, "row {}", row.y).unwrap();
        for bg in &row.backgrounds {
            writeln!(out, "  bg {}", span(bg)).unwrap();
        }
        for run in &row.text {
            let kind = match run.kind {
                RunKind::Narrow => "narrow".to_owned(),
                RunKind::Wide => "wide".to_owned(),
                RunKind::Cluster { cells } => format!("cluster/{cells}"),
            };
            let face = match (run.face.bold, run.face.italic) {
                (false, false) => "",
                (true, false) => " bold",
                (false, true) => " italic",
                (true, true) => " bold italic",
            };
            writeln!(
                out,
                "  text {} {kind}{face} {} {:?}",
                run.col,
                color(run.color),
                run.text
            )
            .unwrap();
        }
        for shape in &row.shapes {
            writeln!(
                out,
                "  shape {} {:?} {}",
                shape.col,
                shape.ch,
                color(shape.color)
            )
            .unwrap();
        }
        for line in &row.lines {
            writeln!(out, "  line {:?} {}", line.kind, span(&line.span)).unwrap();
        }
    }
    match list.cursor {
        Some(cursor) => writeln!(
            out,
            "cursor {},{} {:?} {} cells {}{}",
            cursor.col,
            cursor.row,
            cursor.shape,
            cursor.cells,
            color(cursor.color),
            cursor
                .text
                .map(|text| format!(" text {}", color(text)))
                .unwrap_or_default()
        )
        .unwrap(),
        None => out.push_str("no cursor\n"),
    }
    out
}

fn draw(cols: u16, rows: u16, bytes: &str, focused: bool) -> String {
    let mut terminal = Terminal::new(cols, rows, 100).unwrap();
    // Programs see the defaults a pane has: clear colors set by OSC 10/11.
    terminal.feed(b"\x1b]10;rgb:c0/ca/f5\x07\x1b]11;rgb:1a/1b/26\x07");
    terminal.feed(bytes.as_bytes());
    let list = Builder::default()
        .build(&mut terminal, &settings(focused))
        .unwrap();
    describe(&list)
}

#[test]
fn styled_text() {
    insta::assert_snapshot!(draw(
        40,
        6,
        concat!(
            "plain words here\r\n",
            "\x1b[1mbold\x1b[0m \x1b[3mitalic\x1b[0m \x1b[1;3mboth\x1b[0m \x1b[2mdim\x1b[0m\r\n",
            "\x1b[31mred\x1b[0m \x1b[38;2;10;20;30mrgb\x1b[0m \x1b[41;37m on red \x1b[0m\x1b[44m  \x1b[0m\r\n",
            "\x1b[7minverse\x1b[0m \x1b[8msecret\x1b[0m \x1b[9mstrike\x1b[0m \x1b[53mover\x1b[0m\r\n",
            "\x1b[4msingle\x1b[0m \x1b[4:3;58:2::255:0:0mcurly\x1b[0m \x1b[21mdouble\x1b[0m\r\n",
            "\x1b[42m\x1b[K",
        ),
        true,
    ));
}

#[test]
fn wide_characters_and_emoji() {
    insta::assert_snapshot!(draw(
        20,
        3,
        concat!(
            // Grapheme clustering, as pane terminals have it on.
            "\x1b[?2027h",
            "a\u{4e2d}\u{6587}b\r\n",
            "\u{1f600}x\u{1f44d}\u{1f3fd}y\r\n",
            // A family: one cluster joined by zero-width joiners.
            "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}z e\u{301}\x1b[3;1H",
        ),
        true,
    ));
}

#[test]
fn box_drawing_blocks_and_powerline() {
    insta::assert_snapshot!(draw(
        12,
        4,
        concat!(
            "\u{250c}\u{2500}\u{252c}\u{2500}\u{2510} \u{2591}\u{2588}\r\n",
            "\u{2502}a\u{2502}b\u{2502} \x1b[2m\u{2584}\x1b[0m\r\n",
            "\u{2570}\u{2500}\u{2534}\u{2500}\u{256f}\r\n",
            "\x1b[44;30m main \x1b[0;34m\u{e0b0}\x1b[0m\u{e0b5}",
        ),
        true,
    ));
}

#[test]
fn cursor_shapes() {
    let cases = [
        ("block", "ab\x1b[1;2H", true),
        ("bar", "ab\x1b[6 q\x1b[1;2H", true),
        ("underline", "ab\x1b[4 q\x1b[1;2H", true),
        ("unfocused", "ab\x1b[6 q\x1b[1;2H", false),
        ("hidden", "ab\x1b[?25l", true),
        ("wide head", "\u{4e2d}\x1b[1;1H", true),
        ("wide tail", "\u{4e2d}\x1b[1;2H", true),
    ];
    let mut out = String::new();
    for (name, bytes, focused) in cases {
        let list = draw(6, 1, bytes, focused);
        let cursor = list.lines().last().unwrap();
        writeln!(out, "{name}: {cursor}").unwrap();
    }
    insta::assert_snapshot!(out);
}

#[test]
fn clean_rows_are_reused_and_changed_ones_rebuilt() {
    let mut terminal = Terminal::new(10, 3, 100).unwrap();
    terminal.feed(b"one\r\ntwo\r\nthree");
    let mut builder = Builder::default();
    let first = builder.build(&mut terminal, &settings(true)).unwrap();
    assert_eq!(first.rebuilt, 3, "the first frame builds every row");

    let again = builder.build(&mut terminal, &settings(true)).unwrap();
    assert_eq!(again.rebuilt, 0);
    for (a, b) in first.rows.iter().zip(&again.rows) {
        assert!(Rc::ptr_eq(a, b), "clean rows are the same entries");
    }

    // Row 1 changes; row 2 is redrawn too because the cursor left it.
    terminal.feed(b"\x1b[2;1Hthe");
    let changed = builder.build(&mut terminal, &settings(true)).unwrap();
    assert_eq!(changed.rebuilt, 2);
    assert!(Rc::ptr_eq(&first.rows[0], &changed.rows[0]));
    assert_ne!(changed.rows[1].version, first.rows[1].version);
    assert_eq!(changed.rows[1].text[0].text, "the");

    // Focus changes only the cursor; it reuses the rows.
    let unfocused = builder.build(&mut terminal, &settings(false)).unwrap();
    assert_eq!(unfocused.rebuilt, 0);

    // A resize rebuilds everything at the new size.
    terminal.resize(12, 2).unwrap();
    let resized = builder.build(&mut terminal, &settings(true)).unwrap();
    assert_eq!(
        (resized.cols, resized.rows.len(), resized.rebuilt),
        (12, 2, 2)
    );
}

#[test]
fn explicit_and_inverse_backgrounds_are_solid_while_default_cells_show_the_pane() {
    let mut terminal = Terminal::new(8, 2, 100).unwrap();
    terminal.feed(b"a\x1b[48;2;10;20;30mb\x1b[0m\x1b[7mc\x1b[0m");
    let list = Builder::default()
        .build(&mut terminal, &settings(true))
        .unwrap();
    let backgrounds = &list.rows[0].backgrounds;
    assert_eq!(backgrounds.len(), 2);
    assert_eq!((backgrounds[0].col, backgrounds[0].cells), (1, 1));
    assert_eq!(backgrounds[0].color, Color::opaque(Rgb([10, 20, 30])));
    assert_eq!((backgrounds[1].col, backgrounds[1].cells), (2, 1));
    assert_eq!(backgrounds[1].color.alpha, 1.0);
    assert!(list.rows[1].backgrounds.is_empty());
}
