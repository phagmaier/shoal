//! The pane terminal's replies, modes, colors and events (M1 spec §4.2).

use std::{cell::RefCell, rc::Rc};

use shoal_term::{Mode, PaneSetup, PromptMark, Rgb, TermEvent, Terminal, Theme};

const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb { r, g, b }
}

const DARK: Theme = Theme {
    foreground: rgb(0xa9, 0xb1, 0xd6),
    background: rgb(0x12, 0x34, 0x56),
    cursor: rgb(0x7a, 0xa2, 0xf7),
    ansi: [
        rgb(0x12, 0x34, 0x56),
        rgb(0xf7, 0x76, 0x8e),
        rgb(2, 2, 2),
        rgb(3, 3, 3),
        rgb(4, 4, 4),
        rgb(5, 5, 5),
        rgb(6, 6, 6),
        rgb(7, 7, 7),
        rgb(8, 8, 8),
        rgb(9, 9, 9),
        rgb(10, 10, 10),
        rgb(11, 11, 11),
        rgb(12, 12, 12),
        rgb(13, 13, 13),
        rgb(14, 14, 14),
        rgb(0xc0, 0xca, 0xf5),
    ],
};

struct Pane {
    terminal: Terminal,
    replies: Rc<RefCell<Vec<u8>>>,
    events: Rc<RefCell<Vec<TermEvent>>>,
}

impl Pane {
    fn new(theme: Theme) -> Self {
        let replies = Rc::new(RefCell::new(Vec::new()));
        let events = Rc::new(RefCell::new(Vec::new()));
        let (reply_sink, event_sink) = (replies.clone(), events.clone());
        let setup = PaneSetup {
            cols: 80,
            rows: 24,
            scrollback: 100,
            theme,
            version: "shoal 0.1.0",
        };
        let terminal = Terminal::pane(
            setup,
            Box::new(move |bytes| reply_sink.borrow_mut().extend_from_slice(bytes)),
            Box::new(move |event| event_sink.borrow_mut().push(event)),
        )
        .unwrap();
        Self {
            terminal,
            replies,
            events,
        }
    }

    /// Feed `query` and take what the terminal answered.
    fn ask(&mut self, query: &[u8]) -> String {
        self.terminal.feed(query);
        String::from_utf8(self.replies.take()).unwrap()
    }
}

#[test]
fn device_attributes_match_ghosttys_answers() {
    let mut pane = Pane::new(DARK);
    assert_eq!(pane.ask(b"\x1b[c"), "\x1b[?62;22c");
    assert_eq!(pane.ask(b"\x1b[>c"), "\x1b[>1;0;0c");
    assert_eq!(pane.ask(b"\x1b[=c"), "\x1bP!|00000000\x1b\\");
}

#[test]
fn version_size_and_terminfo_queries_are_answered() {
    let mut pane = Pane::new(DARK);
    assert_eq!(pane.ask(b"\x1b[>q"), "\x1bP>|shoal 0.1.0\x1b\\");
    // XTWINOPS 18: the text area in characters.
    assert_eq!(pane.ask(b"\x1b[18t"), "\x1b[8;24;80t");
    pane.terminal.resize(100, 30).unwrap();
    assert_eq!(pane.ask(b"\x1b[18t"), "\x1b[8;30;100t");
    // XTGETTCAP for TN, hex-encoded: "xterm-256color".
    let name: String = b"xterm-256color"
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect();
    assert_eq!(
        pane.ask(b"\x1bP+q544E\x1b\\"),
        format!("\x1bP1+r544E={name}\x1b\\")
    );
}

#[test]
fn color_queries_get_the_theme() {
    // xterm's answer form: 16 bits per channel.
    let mut pane = Pane::new(DARK);
    assert_eq!(
        pane.ask(b"\x1b]11;?\x1b\\"),
        "\x1b]11;rgb:1212/3434/5656\x1b\\"
    );
    assert_eq!(
        pane.ask(b"\x1b]10;?\x1b\\"),
        "\x1b]10;rgb:a9a9/b1b1/d6d6\x1b\\"
    );
    assert_eq!(
        pane.ask(b"\x1b]4;1;?\x1b\\"),
        "\x1b]4;1;rgb:f7f7/7676/8e8e\x1b\\"
    );
    assert_eq!(
        pane.ask(b"\x1b]4;15;?\x1b\\"),
        "\x1b]4;15;rgb:c0c0/caca/f5f5\x1b\\"
    );
    // Indexes past 15 keep the standard 256-color cube: 16 is black.
    assert_eq!(
        pane.ask(b"\x1b]4;16;?\x1b\\"),
        "\x1b]4;16;rgb:0000/0000/0000\x1b\\"
    );
}

#[test]
fn a_new_theme_changes_answers_but_not_a_programs_own_colors() {
    let mut pane = Pane::new(DARK);
    pane.terminal.feed(b"\x1b]10;rgb:0101/0202/0303\x1b\\");
    let light = Theme {
        background: rgb(0xfa, 0xfa, 0xfa),
        ..DARK
    };
    pane.terminal.set_theme(&light).unwrap();
    assert_eq!(
        pane.ask(b"\x1b]11;?\x1b\\"),
        "\x1b]11;rgb:fafa/fafa/fafa\x1b\\"
    );
    assert_eq!(
        pane.ask(b"\x1b]10;?\x1b\\"),
        "\x1b]10;rgb:0101/0202/0303\x1b\\"
    );
    // OSC 110 resets the program's foreground to the theme's.
    pane.terminal.feed(b"\x1b]110\x1b\\");
    assert_eq!(
        pane.ask(b"\x1b]10;?\x1b\\"),
        "\x1b]10;rgb:a9a9/b1b1/d6d6\x1b\\"
    );
}

#[test]
fn the_color_scheme_follows_the_background() {
    let mut pane = Pane::new(DARK);
    assert_eq!(pane.ask(b"\x1b[?996n"), "\x1b[?997;1n");
    let light = Theme {
        background: rgb(0xee, 0xee, 0xee),
        ..DARK
    };
    let mut pane = Pane::new(light);
    assert_eq!(pane.ask(b"\x1b[?996n"), "\x1b[?997;2n");
}

#[test]
fn grapheme_clustering_is_on_and_survives_a_full_reset() {
    let mut pane = Pane::new(DARK);
    let clustering = Mode::Dec(2027);
    assert!(pane.terminal.mode(clustering).unwrap());
    assert_eq!(pane.ask(b"\x1b[?2027$p"), "\x1b[?2027;1$y");
    pane.terminal.feed(b"\x1b[?2027l");
    assert!(!pane.terminal.mode(clustering).unwrap());
    pane.terminal.feed(b"\x1bc");
    assert!(pane.terminal.mode(clustering).unwrap());
    // The mode travels in a snapshot, so a mirror draws the same widths.
    let bytes = pane.terminal.snapshot().unwrap();
    let mirror = Terminal::restore(&bytes, 80, 24, 100).unwrap();
    assert!(mirror.mode(clustering).unwrap());
}

#[test]
fn metadata_events_are_reported() {
    let mut pane = Pane::new(DARK);
    pane.terminal
        .feed(b"\x07\x1b]2;vim notes\x07\x1b]7;file://host/tmp/x\x07");
    pane.terminal
        .feed(b"\x1b]133;A\x07\x1b]133;B\x07\x1b]133;C\x07\x1b]133;D;3\x07\x1b]133;D\x07");
    assert_eq!(
        *pane.events.borrow(),
        [
            TermEvent::Bell,
            TermEvent::Title(b"vim notes".to_vec()),
            TermEvent::Directory(b"file://host/tmp/x".to_vec()),
            TermEvent::Prompt(PromptMark::PromptStart),
            TermEvent::Prompt(PromptMark::InputStart),
            TermEvent::Prompt(PromptMark::OutputStart),
            TermEvent::Prompt(PromptMark::CommandEnd { exit_code: Some(3) }),
            TermEvent::Prompt(PromptMark::CommandEnd { exit_code: None }),
        ]
    );
    assert!(pane.replies.borrow().is_empty());
}
