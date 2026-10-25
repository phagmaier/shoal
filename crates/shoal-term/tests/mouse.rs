//! Mouse encoding against the reports xterm specifies. Cells are 10 × 20
//! pixels; a report's columns and rows count from 1.

use shoal_term::{CellSize, Mods, MouseAction, MouseButton, MouseEvent, Terminal};

const CELL: CellSize = CellSize {
    width: 10,
    height: 20,
};

/// An event at the middle of cell (`col`, `row`), counted from zero.
fn at(action: MouseAction, button: Option<MouseButton>, col: u16, row: u16) -> MouseEvent {
    MouseEvent {
        action,
        button,
        mods: Mods::default(),
        x: f32::from(col) * 10.0 + 5.0,
        y: f32::from(row) * 20.0 + 10.0,
        buttons_held: button.is_some() && action != MouseAction::Release,
    }
}

fn press(button: MouseButton, col: u16, row: u16) -> MouseEvent {
    at(MouseAction::Press, Some(button), col, row)
}

fn release(button: MouseButton, col: u16, row: u16) -> MouseEvent {
    at(MouseAction::Release, Some(button), col, row)
}

fn motion(col: u16, row: u16) -> MouseEvent {
    at(MouseAction::Motion, None, col, row)
}

fn encode(terminal: &mut Terminal, event: MouseEvent) -> Vec<u8> {
    let mut out = Vec::new();
    terminal.encode_mouse(&event, CELL, &mut out).unwrap();
    out
}

fn terminal(setup: &[u8]) -> Terminal {
    let mut terminal = Terminal::new(80, 24, 100).unwrap();
    terminal.feed(setup);
    terminal
}

#[test]
fn nothing_is_reported_until_a_program_asks() {
    let mut t = terminal(b"");
    assert!(!t.mouse_reporting().unwrap());
    assert_eq!(encode(&mut t, press(MouseButton::Left, 3, 2)), b"");
}

#[test]
fn x10_reports_presses_only() {
    let mut t = terminal(b"\x1b[?9h");
    assert!(t.mouse_reporting().unwrap());
    // CSI M, then 32 + button, 32 + column, 32 + row.
    assert_eq!(encode(&mut t, press(MouseButton::Left, 0, 0)), b"\x1b[M !!");
    assert_eq!(encode(&mut t, press(MouseButton::Left, 3, 2)), b"\x1b[M $#");
    assert_eq!(
        encode(&mut t, press(MouseButton::Right, 3, 2)),
        b"\x1b[M\"$#"
    );
    assert_eq!(encode(&mut t, release(MouseButton::Left, 3, 2)), b"");
    assert_eq!(encode(&mut t, motion(4, 2)), b"");
}

#[test]
fn sgr_reports_presses_releases_wheel_and_modifiers() {
    let mut t = terminal(b"\x1b[?1000h\x1b[?1006h");
    assert_eq!(
        encode(&mut t, press(MouseButton::Left, 4, 2)),
        b"\x1b[<0;5;3M"
    );
    assert_eq!(
        encode(&mut t, release(MouseButton::Left, 4, 2)),
        b"\x1b[<0;5;3m"
    );
    assert_eq!(
        encode(&mut t, press(MouseButton::WheelUp, 4, 2)),
        b"\x1b[<64;5;3M"
    );
    assert_eq!(
        encode(&mut t, press(MouseButton::WheelDown, 4, 2)),
        b"\x1b[<65;5;3M"
    );
    let ctrl_click = MouseEvent {
        mods: Mods {
            ctrl: true,
            ..Mods::default()
        },
        ..press(MouseButton::Left, 4, 2)
    };
    assert_eq!(encode(&mut t, ctrl_click), b"\x1b[<16;5;3M");
    // Normal tracking reports no motion.
    assert_eq!(encode(&mut t, motion(5, 2)), b"");
}

#[test]
fn any_event_tracking_reports_motion_once_per_cell() {
    let mut t = terminal(b"\x1b[?1003h\x1b[?1006h");
    // Motion with no button is 32 + 3.
    assert_eq!(encode(&mut t, motion(9, 4)), b"\x1b[<35;10;5M");
    let same_cell = MouseEvent {
        x: 91.0,
        ..motion(9, 4)
    };
    assert_eq!(encode(&mut t, same_cell), b"");
    assert_eq!(encode(&mut t, motion(10, 4)), b"\x1b[<35;11;5M");
    // A drag is motion with the button: 32 + 0.
    let drag = at(MouseAction::Motion, Some(MouseButton::Left), 11, 4);
    assert_eq!(encode(&mut t, drag), b"\x1b[<32;12;5M");

    // In the pixel format each move within a cell is new information.
    t.feed(b"\x1b[?1016h");
    let first = encode(&mut t, same_cell);
    let nudged = encode(
        &mut t,
        MouseEvent {
            x: 92.0,
            ..same_cell
        },
    );
    assert!(first.starts_with(b"\x1b[<35;") && nudged.starts_with(b"\x1b[<35;"));
    assert_ne!(first, nudged);
}

#[test]
fn button_event_tracking_reports_drags_but_not_hovering() {
    let mut t = terminal(b"\x1b[?1002h\x1b[?1006h");
    assert_eq!(encode(&mut t, motion(2, 1)), b"");
    let drag = at(MouseAction::Motion, Some(MouseButton::Left), 3, 1);
    assert_eq!(encode(&mut t, drag), b"\x1b[<32;4;2M");
}

#[test]
fn a_zero_cell_size_is_an_error() {
    let mut t = terminal(b"\x1b[?1000h");
    let mut out = Vec::new();
    let zero = CellSize {
        width: 0,
        height: 20,
    };
    assert!(
        t.encode_mouse(&press(MouseButton::Left, 0, 0), zero, &mut out)
            .is_err()
    );
}
