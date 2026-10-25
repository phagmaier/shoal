//! Key, focus and paste encoding against the bytes xterm and the Kitty
//! keyboard protocol specify.

use shoal_term::{Key, KeyAction, KeyEvent, Mods, Terminal};

fn terminal(setup: &[u8]) -> Terminal {
    let mut terminal = Terminal::new(80, 24, 100).unwrap();
    terminal.feed(setup);
    terminal
}

fn key(terminal: &mut Terminal, event: KeyEvent<'_>) -> Vec<u8> {
    let mut out = Vec::new();
    terminal.encode_key(&event, &mut out).unwrap();
    out
}

fn named(key: Key) -> KeyEvent<'static> {
    KeyEvent {
        key,
        ..KeyEvent::default()
    }
}

fn text(key: Key, text: &'static str, unshifted: char) -> KeyEvent<'static> {
    KeyEvent {
        key,
        text: Some(text),
        unshifted: Some(unshifted),
        ..KeyEvent::default()
    }
}

const CTRL: Mods = Mods {
    shift: false,
    ctrl: true,
    alt: false,
    super_key: false,
    caps_lock: false,
    num_lock: false,
};

#[test]
fn keys_in_normal_cursor_mode() {
    let mut t = terminal(b"");
    assert_eq!(key(&mut t, named(Key::ArrowUp)), b"\x1b[A");
    assert_eq!(key(&mut t, named(Key::Home)), b"\x1b[H");
    assert_eq!(key(&mut t, named(Key::Enter)), b"\r");
    assert_eq!(key(&mut t, named(Key::Backspace)), b"\x7f");
    assert_eq!(key(&mut t, named(Key::F5)), b"\x1b[15~");
    assert_eq!(key(&mut t, text(Key::A, "a", 'a')), b"a");
    let ctrl_a = KeyEvent {
        mods: CTRL,
        ..text(Key::A, "a", 'a')
    };
    assert_eq!(key(&mut t, ctrl_a), b"\x01");
    let ctrl_up = KeyEvent {
        mods: CTRL,
        ..named(Key::ArrowUp)
    };
    assert_eq!(key(&mut t, ctrl_up), b"\x1b[1;5A");
    // Releases mean nothing to a legacy program.
    let release = KeyEvent {
        action: KeyAction::Release,
        ..text(Key::A, "a", 'a')
    };
    assert_eq!(key(&mut t, release), b"");
}

#[test]
fn keys_in_application_cursor_mode() {
    let mut t = terminal(b"\x1b[?1h");
    assert_eq!(key(&mut t, named(Key::ArrowUp)), b"\x1bOA");
    assert_eq!(key(&mut t, named(Key::ArrowLeft)), b"\x1bOD");
    assert_eq!(key(&mut t, named(Key::Home)), b"\x1bOH");
    // A modifier forces the CSI form.
    let ctrl_up = KeyEvent {
        mods: CTRL,
        ..named(Key::ArrowUp)
    };
    assert_eq!(key(&mut t, ctrl_up), b"\x1b[1;5A");
    // Leaving the mode goes back to the normal form.
    t.feed(b"\x1b[?1l");
    assert_eq!(key(&mut t, named(Key::ArrowUp)), b"\x1b[A");
}

#[test]
fn keys_under_kitty_keyboard_flags() {
    // Flag 1, disambiguate escape codes.
    let mut t = terminal(b"\x1b[>1u");
    assert_eq!(key(&mut t, named(Key::Escape)), b"\x1b[27u");
    let ctrl_a = KeyEvent {
        mods: CTRL,
        ..text(Key::A, "a", 'a')
    };
    assert_eq!(key(&mut t, ctrl_a), b"\x1b[97;5u");
    assert_eq!(key(&mut t, text(Key::A, "a", 'a')), b"a");
    assert_eq!(key(&mut t, named(Key::Enter)), b"\r");
    // Shift that only picks the character still sends plain text.
    let shift = Mods {
        shift: true,
        ..Mods::default()
    };
    let bang = KeyEvent {
        mods: shift,
        consumed: shift,
        ..text(Key::Digit1, "!", '1')
    };
    assert_eq!(key(&mut t, bang), b"!");

    // Flags 1 and 2 also report releases, as event type 3.
    t.feed(b"\x1b[>3u");
    let release = |key| KeyEvent {
        action: KeyAction::Release,
        ..named(key)
    };
    assert_eq!(key(&mut t, release(Key::Escape)), b"\x1b[27;1:3u");

    // Popping back to flag 1 stops release reports.
    t.feed(b"\x1b[<u");
    assert_eq!(key(&mut t, release(Key::Escape)), b"");

    // Flag 8 reports every key as an escape code, by its unshifted key.
    t.feed(b"\x1b[>8u");
    assert_eq!(key(&mut t, text(Key::A, "a", 'a')), b"\x1b[97u");
    let shift_a = KeyEvent {
        mods: Mods {
            shift: true,
            ..Mods::default()
        },
        ..text(Key::A, "A", 'a')
    };
    assert_eq!(key(&mut t, shift_a), b"\x1b[97;2u");
    assert_eq!(key(&mut t, named(Key::Enter)), b"\x1b[13u");

    // Popping every level returns to legacy encoding.
    t.feed(b"\x1b[<5u");
    assert_eq!(key(&mut t, named(Key::Escape)), b"\x1b");
}

#[test]
fn a_restored_mirror_encodes_with_the_programs_modes() {
    let mirror = |setup: &[u8]| {
        let snapshot = terminal(setup).snapshot().unwrap();
        Terminal::restore(&snapshot, 80, 24, 100).unwrap()
    };
    assert_eq!(
        key(&mut mirror(b"\x1b[?1h"), named(Key::ArrowUp)),
        b"\x1bOA"
    );
    assert_eq!(
        key(&mut mirror(b"\x1b[>1u"), named(Key::Escape)),
        b"\x1b[27u"
    );
}

fn focus(terminal: &Terminal, focused: bool) -> Vec<u8> {
    let mut out = Vec::new();
    terminal.encode_focus(focused, &mut out).unwrap();
    out
}

#[test]
fn focus_reports_only_when_asked_for() {
    let mut t = terminal(b"");
    assert_eq!(focus(&t, true), b"");
    t.feed(b"\x1b[?1004h");
    assert_eq!(focus(&t, true), b"\x1b[I");
    assert_eq!(focus(&t, false), b"\x1b[O");
    t.feed(b"\x1b[?1004l");
    assert_eq!(focus(&t, false), b"");
}

fn paste(terminal: &Terminal, text: &str) -> Vec<u8> {
    let mut out = b"before".to_vec();
    terminal.encode_paste(text, &mut out).unwrap();
    out.drain(..6);
    out
}

#[test]
fn pastes_without_bracketed_paste() {
    let t = terminal(b"");
    assert_eq!(paste(&t, "echo a\nls"), b"echo a\rls");
    // ESC could start a sequence, so it becomes a space.
    assert_eq!(paste(&t, "a\x1bb"), b"a b");
}

#[test]
fn pastes_with_bracketed_paste() {
    let t = terminal(b"\x1b[?2004h");
    assert_eq!(paste(&t, "echo a\nls"), b"\x1b[200~echo a\nls\x1b[201~");
    // Pasted text cannot end the paste early and run what follows.
    assert_eq!(paste(&t, "x\x1b[201~rm"), b"\x1b[200~x [201~rm\x1b[201~");
    let large = "word ".repeat(40_000);
    let encoded = paste(&t, &large);
    assert_eq!(encoded.len(), large.len() + 12);
    assert_eq!(&encoded[6..encoded.len() - 6], large.as_bytes());
}
