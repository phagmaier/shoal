//! Render-state reads against a fed terminal: dirty rows, cell text and
//! widths, styles, colors and the cursor.

use shoal_term::{
    CellStyle, CellWidth, CursorPosition, CursorShape, Dirty, Rgb, RowCells, Rows, Terminal,
    Underline,
};

#[derive(Debug, PartialEq)]
struct Row {
    y: u16,
    dirty: bool,
    /// Text and width of each cell.
    cells: Vec<(String, CellWidth)>,
    /// Each cell's style.
    styles: Vec<CellStyle>,
}

fn collect(row: &RowCells) -> Row {
    Row {
        y: row.y(),
        dirty: row.dirty(),
        cells: row
            .cells()
            .iter()
            .map(|cell| (row.text(cell).to_owned(), cell.width))
            .collect(),
        styles: row.cells().iter().map(|cell| *row.style(cell)).collect(),
    }
}

fn read(terminal: &mut Terminal, which: Rows) -> (Dirty, Vec<Row>) {
    let mut frame = terminal.frame().unwrap();
    let dirty = frame.dirty().unwrap();
    let mut rows = Vec::new();
    frame
        .read_rows(which, |row| rows.push(collect(row)))
        .unwrap();
    frame.mark_clean().unwrap();
    (dirty, rows)
}

const RED: Rgb = Rgb { r: 255, g: 0, b: 0 };
const BLUE: Rgb = Rgb { r: 0, g: 0, b: 255 };
/// What OSC 4 sets palette entry 1 to below.
const PALETTE_1: Rgb = Rgb {
    r: 0x12,
    g: 0x34,
    b: 0x56,
};

#[test]
fn cells_match_what_was_fed() {
    let mut t = Terminal::new(6, 3, 100).unwrap();
    // Row 0: bold red "ab", then a wide character.
    t.feed(b"\x1b[1;38;2;255;0;0mab\x1b[0m\xe4\xb8\xad\r\n");
    // Row 1: "e" with a combining acute accent, then a thumbs-up with a skin
    // tone under Unicode grapheme clustering (mode 2027), then palette color
    // 1 after OSC 4 changed it.
    t.feed("\x1b[?2027he\u{301}\u{1f44d}\u{1f3fd}".as_bytes());
    t.feed(b"\x1b]4;1;rgb:12/34/56\x1b\\\x1b[31mx\x1b[0m\r\n");
    // Row 2: a curly underline in palette color 1, then erase to the end with
    // a blue background.
    t.feed(b"\x1b[4:3;58;5;1m~\x1b[0m\x1b[48;2;0;0;255m\x1b[K\x1b[0m");

    let (dirty, rows) = read(&mut t, Rows::Dirty);
    assert_eq!(dirty, Dirty::Full);
    assert_eq!(rows.iter().map(|row| row.y).collect::<Vec<_>>(), [0, 1, 2]);

    let narrow = |text: &str| (text.to_owned(), CellWidth::Narrow);
    let blank = || narrow("");
    let bold_red = CellStyle {
        fg: Some(RED),
        bold: true,
        ..CellStyle::default()
    };
    let plain = CellStyle::default();
    assert_eq!(
        rows[0].cells,
        [
            narrow("a"),
            narrow("b"),
            ("\u{4e2d}".to_owned(), CellWidth::Wide),
            (String::new(), CellWidth::SpacerTail),
            blank(),
            blank(),
        ]
    );
    assert_eq!(rows[0].styles[..2], [bold_red, bold_red]);
    assert_eq!(rows[0].styles[2..], [plain; 4]);

    assert_eq!(
        rows[1].cells,
        [
            narrow("e\u{301}"),
            ("\u{1f44d}\u{1f3fd}".to_owned(), CellWidth::Wide),
            (String::new(), CellWidth::SpacerTail),
            narrow("x"),
            blank(),
            blank(),
        ]
    );
    let palette_fg = CellStyle {
        fg: Some(PALETTE_1),
        ..CellStyle::default()
    };
    assert_eq!(rows[1].styles[3], palette_fg);

    let curly = CellStyle {
        underline: Underline::Curly,
        underline_color: Some(PALETTE_1),
        ..CellStyle::default()
    };
    let blue_bg = CellStyle {
        bg: Some(BLUE),
        ..CellStyle::default()
    };
    assert_eq!(
        rows[2].cells,
        [narrow("~"), blank(), blank(), blank(), blank(), blank()]
    );
    assert_eq!(
        rows[2].styles,
        [curly, blue_bg, blue_bg, blue_bg, blue_bg, blue_bg]
    );
}

#[test]
fn equal_styles_in_a_row_share_an_index() {
    let mut t = Terminal::new(6, 1, 100).unwrap();
    t.feed(b"\x1b[31ma\x1b[32mb\x1b[31mc\x1b[0md");
    let mut frame = t.frame().unwrap();
    let mut indices = Vec::new();
    frame
        .read_rows(Rows::All, |row| {
            assert_eq!(row.styles()[0], CellStyle::default());
            indices = row.cells().iter().map(|cell| cell.style).collect();
        })
        .unwrap();
    let [a, b, c, d, ref rest @ ..] = indices[..] else {
        panic!("six cells expected, got {indices:?}");
    };
    assert_eq!(a, c);
    assert_ne!(a, b);
    assert_eq!(d, 0);
    assert!(rest.iter().all(|index| *index == 0));
}

#[test]
fn the_cursor_matches_the_terminal() {
    let mut t = Terminal::new(10, 3, 100).unwrap();
    t.feed(b"ab\r\nc");
    let cursor = t.frame().unwrap().cursor().unwrap();
    assert_eq!(
        cursor.position,
        Some(CursorPosition {
            col: 1,
            row: 1,
            wide_tail: false
        })
    );
    assert!(cursor.visible);
    assert_eq!(cursor.shape, CursorShape::Block);

    // DECSCUSR 5 asks for a blinking bar; DECTCEM hides the cursor.
    t.feed(b"\x1b[5 q\x1b[?25l");
    let cursor = t.frame().unwrap().cursor().unwrap();
    assert_eq!(cursor.shape, CursorShape::Bar);
    assert!(cursor.blinking);
    assert!(!cursor.visible);

    // DECSCUSR 4 asks for a steady underline.
    t.feed(b"\x1b[4 q\x1b[?25h");
    let cursor = t.frame().unwrap().cursor().unwrap();
    assert_eq!(cursor.shape, CursorShape::Underline);
    assert!(!cursor.blinking);
    assert!(cursor.visible);

    // The right half of a wide character.
    t.feed("\x1b[3;1H\u{4e2d}\x1b[3;2H".as_bytes());
    let cursor = t.frame().unwrap().cursor().unwrap();
    assert_eq!(
        cursor.position,
        Some(CursorPosition {
            col: 1,
            row: 2,
            wide_tail: true
        })
    );
}

#[test]
fn the_default_cursor_gives_way_to_a_programs_choice() {
    let shape = |t: &mut Terminal| {
        let cursor = t.frame().unwrap().cursor().unwrap();
        (cursor.shape, cursor.blinking)
    };
    let mut t = Terminal::new(10, 3, 100).unwrap();
    t.set_default_cursor(CursorShape::Bar, false).unwrap();
    assert_eq!(shape(&mut t), (CursorShape::Bar, false));
    // A program's choice wins, even over a later default.
    t.feed(b"\x1b[3 q");
    t.set_default_cursor(CursorShape::Bar, false).unwrap();
    assert_eq!(shape(&mut t), (CursorShape::Underline, true));
    // Asking for the default brings it back.
    t.feed(b"\x1b[0 q");
    assert_eq!(shape(&mut t), (CursorShape::Bar, false));

    // A snapshot keeps whether the program chose: a mirror's default
    // replaces only a cursor nobody chose.
    let mut chose = Terminal::new(10, 3, 100).unwrap();
    chose.feed(b"\x1b[2 q");
    let mut never = Terminal::new(10, 3, 100).unwrap();
    for (pane, expected) in [
        (&mut chose, CursorShape::Block),
        (&mut never, CursorShape::Underline),
    ] {
        let bytes = pane.snapshot().unwrap();
        let mut mirror = Terminal::restore(&bytes, 10, 3, 100).unwrap();
        mirror
            .set_default_cursor(CursorShape::Underline, false)
            .unwrap();
        assert_eq!(shape(&mut mirror).0, expected);
    }
}

#[test]
fn colors_report_the_defaults_a_program_set() {
    let mut t = Terminal::new(4, 1, 100).unwrap();
    // OSC 10, 11 and 12 set the foreground, background and cursor colors.
    t.feed(b"\x1b]10;rgb:ff/00/00\x07\x1b]11;rgb:00/00/ff\x07\x1b]12;rgb:12/34/56\x07");
    let colors = t.frame().unwrap().colors().unwrap();
    assert_eq!(colors.foreground, RED);
    assert_eq!(colors.background, BLUE);
    assert_eq!(colors.cursor, Some(PALETTE_1));
}

#[test]
fn rows_are_clean_after_a_frame_until_they_change() {
    let mut t = Terminal::new(8, 3, 100).unwrap();
    // Park the cursor on the row written below: moving the cursor also
    // dirties the row it leaves.
    t.feed(b"one\r\ntwo\r\nthree\x1b[2;4H");
    assert_eq!(read(&mut t, Rows::Dirty).0, Dirty::Full);

    let (dirty, rows) = read(&mut t, Rows::Dirty);
    assert_eq!(dirty, Dirty::Clean);
    assert!(rows.is_empty());

    t.feed(b"!");
    let (dirty, rows) = read(&mut t, Rows::Dirty);
    assert_eq!(dirty, Dirty::Partial);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].y, rows[0].dirty), (1, true));
    assert_eq!(rows[0].cells[3].0, "!");

    // Reading every row of a clean frame reports each as clean.
    let (dirty, rows) = read(&mut t, Rows::All);
    assert_eq!(dirty, Dirty::Clean);
    assert_eq!(
        rows.iter()
            .map(|row| (row.y, row.dirty))
            .collect::<Vec<_>>(),
        [(0, false), (1, false), (2, false)]
    );
}

#[test]
fn a_frame_not_marked_clean_is_drawn_again() {
    let mut t = Terminal::new(8, 2, 100).unwrap();
    t.feed(b"hi");
    // A frame read but never marked clean, as when drawing fails.
    let _ = t.frame().unwrap();
    let (dirty, rows) = read(&mut t, Rows::Dirty);
    assert_eq!(dirty, Dirty::Full);
    assert_eq!(rows.len(), 2);
}

#[test]
fn row_ids_follow_rows_as_the_screen_scrolls() {
    let mut t = Terminal::new(8, 3, 100).unwrap();
    t.feed(b"a\r\nb\r\nc");
    let ids = |t: &mut Terminal| {
        let mut frame = t.frame().unwrap();
        let mut ids = Vec::new();
        frame
            .read_rows(Rows::All, |row| ids.push(row.id()))
            .unwrap();
        frame.mark_clean().unwrap();
        ids
    };
    let before = ids(&mut t);
    assert_eq!(ids(&mut t), before);
    // A new line scrolls every row up one.
    t.feed(b"\r\nd");
    let after = ids(&mut t);
    assert_eq!(after[..2], before[1..]);
    assert!(!before.contains(&after[2]));
}

#[test]
fn a_restored_mirror_draws_the_panes_content() {
    let mut pane = Terminal::new(6, 2, 100).unwrap();
    pane.feed(b"\x1b[38;2;255;0;0mok\x1b[0m");
    let snapshot = pane.snapshot().unwrap();
    let mut mirror = Terminal::restore(&snapshot, 6, 2, 100).unwrap();
    let (_, rows) = read(&mut mirror, Rows::All);
    assert_eq!(rows[0].cells[0].0, "o");
    assert_eq!(rows[0].styles[1].fg, Some(RED));
}
