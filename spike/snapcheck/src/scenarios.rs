//! Synthetic scenarios, one per kind of terminal state a snapshot must carry.
//! `setup` builds the state; `after` is more output a mirror would receive,
//! chosen to expose state the restore missed.

pub struct Scenario {
    pub name: &'static str,
    pub cols: u16,
    pub rows: u16,
    pub setup: Vec<u8>,
    pub after: Vec<u8>,
}

fn s(name: &'static str, setup: impl Into<Vec<u8>>, after: impl Into<Vec<u8>>) -> Scenario {
    Scenario {
        name,
        cols: 80,
        rows: 24,
        setup: setup.into(),
        after: after.into(),
    }
}

fn styled_lines(n: usize) -> String {
    let mut out = String::new();
    for i in 0..n {
        out.push_str(&format!(
            "{i:04} \x1b[1mbold\x1b[0m \x1b[3mital\x1b[0m \x1b[4mul\x1b[0m \x1b[4:3mcurly\x1b[0m \
             \x1b[58;2;255;0;0m\x1b[4mredul\x1b[0m \x1b[2mdim\x1b[0m \x1b[7minv\x1b[0m \x1b[9mstrike\x1b[0m \
             \x1b[53mover\x1b[0m \x1b[31mred\x1b[0m \x1b[91mbred\x1b[0m \x1b[38;5;208m256\x1b[0m \
             \x1b[38;2;10;20;30;48;2;200;100;50mtrue\x1b[0m 漢字 🎉 e\u{301}\r\n"
        ));
    }
    out
}

fn many_lines(prefix: &str, n: usize) -> String {
    (0..n).map(|i| format!("{prefix} {i}\r\n")).collect()
}

pub fn all() -> Vec<Scenario> {
    let long_line = "x".repeat(200);
    vec![
        s(
            "text, styles, wide chars, 300 lines scrollback",
            format!(
                "{}{long_line}\r\n\x1b[44m\x1b[K\x1b[0m\r\nend",
                styled_lines(300)
            ),
            many_lines("after", 30),
        ),
        s(
            "cursor position, hidden, pen style",
            "abc\x1b[5;10H\x1b[?25l\x1b[1;31;48;5;22m",
            "XYZ\r\nnext",
        ),
        s("cursor shape (DECSCUSR bar)", "abc\x1b[5 q", "def"),
        s("pending wrap at last column", "x".repeat(80), "Q"),
        s(
            "input modes (DECCKM, keypad, paste, focus, mouse)",
            "\x1b[?1h\x1b=\x1b[?2004h\x1b[?1004h\x1b[?1002h\x1b[?1006h\x1b[?1007h\x1b[?1036h\x1b[?1039h",
            "text",
        ),
        s("X10 and any-event mouse with UTF-8 encoding", "\x1b[?1003h\x1b[?1005h", "text"),
        s(
            "screen modes (insert, no wrap, reverse video, LNM)",
            "\x1b[4h\x1b[?7l\x1b[?5h\x1b[20hhello\x1b[1;1H",
            format!("AB{}\nnext", "y".repeat(100)),
        ),
        s(
            "misc DEC modes (2026, 2027, 2031, 2048, 12, 45)",
            "\x1b[?2026h\x1b[?2027h\x1b[?2031h\x1b[?2048h\x1b[?12h\x1b[?45h",
            "text",
        ),
        s(
            "scroll region + origin mode",
            format!("{}\x1b[5;15r\x1b[?6h\x1b[1;1Hregion top", many_lines("base", 24)),
            format!("{}\x1b[1;1HTOP", many_lines("\r\nin region", 20)),
        ),
        s(
            "left/right margins (DECSLRM)",
            "\x1b[?69h\x1b[10;40s\x1b[1;10H",
            "y".repeat(100),
        ),
        s(
            "custom tab stops",
            "\x1b[3g\x1b[1;5H\x1bH\x1b[1;13H\x1bH\x1b[1;30H\x1bH\x1b[2;1H",
            "a\tb\tc\td\r\n",
        ),
        s("DEC special graphics in G0", "\x1b(0", "lqqk\r\nx  x\r\nmqqj"),
        s("G1 graphics shifted in (SO)", "\x1b)0\x0e", "lqqk\r\nmqqj"),
        s("kitty keyboard flags", "\x1b[>5u", "text"),
        s("modifyOtherKeys", "\x1b[>4;2m", "text"),
        s(
            "hyperlinks (closed and still open)",
            "\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ plain \x1b]8;id=x;https://a.b\x1b\\open",
            " more",
        ),
        s(
            "palette and dynamic colors (OSC 4/10/11/12)",
            "\x1b]4;1;rgb:12/34/56\x1b\\\x1b]10;rgb:aa/bb/cc\x1b\\\x1b]11;rgb:01/02/03\x1b\\\x1b]12;rgb:ff/00/ff\x1b\\\x1b[31mred",
            "\x1b[31m more red",
        ),
        s(
            "title and pwd (OSC 2, OSC 7)",
            "\x1b]2;my title\x07\x1b]7;file://host/home/user/code\x07$ ",
            "ls",
        ),
        s(
            "saved cursor (DECSC)",
            "\x1b[3;7H\x1b[1;32m\x1b7\x1b[10;1H\x1b[0mplain",
            "\x1b8X",
        ),
        s(
            "alternate screen 1049 over scrollback",
            format!(
                "{}primary prompt $ \x1b[?1049h\x1b[H\x1b[2J\x1b[1;34mALT SCREEN\x1b[0m\r\nline 2\x1b[5;5H",
                many_lines("primary", 100)
            ),
            "!\x1b[?1049lafter",
        ),
        s(
            "alternate screen 47",
            "primary text\x1b[?47h\x1b[2J\x1b[Halt text",
            "\x1b[?47lback",
        ),
        s(
            "shell prompt marks (OSC 133)",
            "\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07file1 file2\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ \x1b]133;B\x07",
            "echo hi",
        ),
        s(
            "protected characters (DECSCA)",
            "\x1b[1\"qPROT\x1b[0\"q unprot",
            "\x1b[1;1H\x1b[?2K",
        ),
        s("split mid-escape (dump taken inside CSI)", "hello \x1b[3", "1mRED\x1b[0m"),
        s("split mid-UTF-8", b"caf\xc3".to_vec(), b"\xa9 ok".to_vec()),
    ]
}
