use std::{cell::RefCell, rc::Rc};

use shoal_term::Terminal;

fn pane() -> (Terminal, Rc<RefCell<Vec<u8>>>) {
    let replies = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&replies);
    let terminal = Terminal::with_replies(
        80,
        24,
        100,
        Box::new(move |bytes| sink.borrow_mut().extend_from_slice(bytes)),
    )
    .unwrap();
    (terminal, replies)
}

#[test]
fn the_pane_terminal_answers_a_cursor_position_query() {
    let (mut terminal, replies) = pane();
    terminal.feed(b"\x1b[3;5H\x1b[6n");
    assert_eq!(*replies.borrow(), b"\x1b[3;5R");
}

#[test]
fn a_restored_mirror_answers_nothing_even_for_a_query_split_by_the_snapshot() {
    let (mut terminal, replies) = pane();
    terminal.feed(b"\x1b[3;5H\x1b[6");
    let bytes = terminal.snapshot().unwrap();
    let mut mirror = Terminal::restore(&bytes, 80, 24, 100).unwrap();
    // The restored parser completes the query on the very next byte. Neither
    // the pane's sink nor anything else hears an answer from the mirror.
    mirror.feed(b"n\x1b[5n\x1b[c");
    assert_eq!(mirror.cursor().unwrap(), (4, 2));
    assert!(replies.borrow().is_empty());
    // The pane terminal itself still answers the same completed query.
    terminal.feed(b"n");
    assert_eq!(*replies.borrow(), b"\x1b[3;5R");
}
