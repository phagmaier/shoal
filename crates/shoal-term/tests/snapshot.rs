use shoal_term::{CaptureFormat, Mode, SnapshotError, Terminal};

// The mode inventory the T00 snapshot harness compared.
const DEC_MODES: &[u16] = &[
    1, 3, 4, 5, 6, 7, 8, 9, 12, 25, 40, 45, 47, 66, 69, 1000, 1002, 1003, 1004, 1005, 1006, 1007,
    1015, 1016, 1035, 1036, 1039, 1045, 1047, 1048, 1049, 2004, 2026, 2027, 2031, 2033, 2048, 5522,
];
const ANSI_MODES: &[u16] = &[2, 4, 12, 20];

/// Everything observable through the wrapper: every cell with its style (the
/// VT capture), cursor, screens, history, title, directory, modes and parser.
#[derive(Debug, PartialEq)]
struct State {
    vt: String,
    vt_screen: String,
    text: String,
    cursor: (u16, u16),
    alternate: bool,
    history: usize,
    title: Vec<u8>,
    pwd: Vec<u8>,
    modes: Vec<(Mode, bool)>,
    ground: bool,
}

fn state(terminal: &Terminal) -> State {
    let capture = |format, scrollback| {
        String::from_utf8_lossy(&terminal.capture(format, scrollback).unwrap()).into_owned()
    };
    let modes = DEC_MODES
        .iter()
        .map(|&n| Mode::Dec(n))
        .chain(ANSI_MODES.iter().map(|&n| Mode::Ansi(n)))
        .map(|mode| (mode, terminal.mode(mode).unwrap()))
        .collect();
    State {
        vt: capture(CaptureFormat::Vt, true),
        vt_screen: capture(CaptureFormat::Vt, false),
        text: capture(CaptureFormat::Text, true),
        cursor: terminal.cursor().unwrap(),
        alternate: terminal.alternate_screen().unwrap(),
        history: terminal.scrollback_rows().unwrap(),
        title: terminal.title().unwrap(),
        pwd: terminal.pwd().unwrap(),
        modes,
        ground: terminal.is_ground().unwrap(),
    }
}

fn lines(count: usize) -> Vec<u8> {
    (0..count)
        .flat_map(|i| format!("\x1b[3{}mline {i}\x1b[0m wide 漢字\r\n", i % 8).into_bytes())
        .collect()
}

/// A restored mirror equals the original, and stays equal after the same
/// follow-up output, which exposes state a snapshot could lose silently.
fn round_trip(name: &str, prefix: &[u8], follow: &[u8]) {
    let mut original = Terminal::new(80, 24, 10_000).unwrap();
    original.feed(prefix);
    let bytes = original.snapshot().unwrap();
    let mut mirror = Terminal::restore(&bytes, 80, 24, 10_000).unwrap();
    assert_eq!(state(&mirror), state(&original), "{name}: at restore");
    original.feed(follow);
    mirror.feed(follow);
    assert_eq!(state(&mirror), state(&original), "{name}: after follow-up");
}

#[test]
fn snapshots_round_trip_cells_and_modes() {
    let history = lines(300);
    let cases: &[(&str, &[u8], &[u8])] = &[
        (
            "text, styles, wide chars, 300 lines",
            &history,
            b"\x1b[1;4mnext\x1b[0m",
        ),
        (
            "cursor hidden with a pen",
            b"\x1b[5;7H\x1b[1;32m\x1b[?25l",
            b"pen",
        ),
        ("pending wrap", &[b'x'; 80], b"y"),
        (
            "input modes",
            b"\x1b[?1h\x1b=\x1b[?2004h\x1b[?1004h\x1b[?1006h\x1b[?1002h",
            b"",
        ),
        (
            "scroll region and origin",
            b"\x1b[5;10r\x1b[?6h\x1b[2;3H",
            b"in\r\n\n\n\n\n\n\nregion",
        ),
        (
            "tab stops",
            b"\x1b[3g\x1b[1;5H\x1bH\x1b[1;13H\x1bH\r",
            b"\ta\tb",
        ),
        ("DEC graphics", b"\x1b(0", b"lqqk"),
        (
            "hyperlink",
            b"\x1b]8;;https://example.com\x1b\\link",
            b" more\x1b]8;;\x1b\\",
        ),
        (
            "title and directory",
            b"\x1b]2;title\x07\x1b]7;file:///tmp\x07",
            b"",
        ),
        (
            "saved cursor",
            b"\x1b[5;10H\x1b[1m\x1b7\x1b[H\x1b[0m",
            b"\x1b8X",
        ),
        (
            "alternate 1049 over history",
            &[&history[..], b"\x1b[?1049halt"].concat(),
            b"\x1b[?1049lafter",
        ),
        ("alternate 47", b"main\x1b[?47halt", b"\x1b[?47lafter"),
        (
            "prompt marks",
            b"\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07",
            b"out\r\n\x1b]133;D;0\x07",
        ),
        ("split inside CSI", b"abc\x1b[3", b"1mred\x1b[0m"),
        ("split inside UTF-8", b"caf\xc3", b"\xa9!"),
    ];
    for (name, prefix, follow) in cases {
        round_trip(name, prefix, follow);
    }
}

#[test]
fn a_long_unfinished_osc_is_mid_sequence_until_its_terminator() {
    let mut terminal = Terminal::new(80, 24, 1000).unwrap();
    terminal.feed(b"before\x1b]2;");
    terminal.feed(&[b'y'; 5000]);
    assert!(matches!(
        terminal.snapshot(),
        Err(SnapshotError::MidSequence)
    ));
    assert!(!terminal.is_ground().unwrap());

    // An attach is served at the first ground point of the next read.
    let chunk = b"yy\x07after";
    assert_eq!(terminal.feed_until_ground(chunk).unwrap(), Some(3));
    let bytes = terminal.snapshot().unwrap();
    let mut mirror = Terminal::restore(&bytes, 80, 24, 1000).unwrap();
    terminal.feed(&chunk[3..]);
    mirror.feed(&chunk[3..]);
    assert_eq!(state(&mirror), state(&terminal));
    // The whole OSC was consumed: the text around it is adjacent.
    let text = mirror.capture(CaptureFormat::Text, false).unwrap();
    assert_eq!(text, b"beforeafter");
    // Output that never reaches ground is consumed whole; at ground nothing is.
    terminal.feed(b"\x1b]2;");
    assert_eq!(terminal.feed_until_ground(b"zz").unwrap(), None);
    assert_eq!(terminal.feed_until_ground(b"\x07x").unwrap(), Some(1));
    assert_eq!(terminal.feed_until_ground(b"x").unwrap(), Some(0));
    assert_eq!(terminal.title().unwrap(), b"zz");
    assert_eq!(
        terminal.capture(CaptureFormat::Text, false).unwrap(),
        b"beforeafter"
    );
}

#[test]
fn an_unfinished_osc_within_the_cap_snapshots_and_completes_on_the_mirror() {
    let mut terminal = Terminal::new(80, 24, 1000).unwrap();
    terminal.feed(b"\x1b]2;");
    terminal.feed(&[b'y'; 200]);
    let bytes = terminal.snapshot().unwrap();
    let mut mirror = Terminal::restore(&bytes, 80, 24, 1000).unwrap();
    assert!(!mirror.is_ground().unwrap());
    mirror.feed(b"\x07");
    assert_eq!(mirror.title().unwrap(), [b'y'; 200]);
}
