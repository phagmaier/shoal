use shoal_term::{CaptureFormat, Mode, Terminal};

fn screen(terminal: &Terminal) -> String {
    String::from_utf8(terminal.capture(CaptureFormat::Text, false).unwrap()).unwrap()
}

#[test]
fn writes_land_where_the_cursor_is() {
    let mut terminal = Terminal::new(10, 3, 100).unwrap();
    terminal.feed(b"hello\r\nworld");
    assert_eq!(screen(&terminal), "hello\nworld");
    assert_eq!(terminal.cursor().unwrap(), (5, 1));
    // Cursor addressing and erase overwrite in place.
    terminal.feed(b"\x1b[1;2HE\x1b[2;3H\x1b[K");
    assert_eq!(screen(&terminal), "hEllo\nwo");
    // Output past the bottom row scrolls the top row into history.
    terminal.feed(b"\r\nthree\r\nfour");
    assert_eq!(screen(&terminal), "wo\nthree\nfour");
    assert_eq!(terminal.scrollback_rows().unwrap(), 1);
    let all = terminal.capture(CaptureFormat::Text, true).unwrap();
    assert_eq!(String::from_utf8(all).unwrap(), "hEllo\nwo\nthree\nfour");
}

#[test]
fn resize_reflows_soft_wrapped_lines() {
    let mut terminal = Terminal::new(10, 3, 100).unwrap();
    terminal.feed(b"abcdefghijKLM");
    assert_eq!(screen(&terminal), "abcdefghij\nKLM");
    terminal.resize(20, 3).unwrap();
    assert_eq!(
        (terminal.cols().unwrap(), terminal.rows().unwrap()),
        (20, 3)
    );
    assert_eq!(screen(&terminal), "abcdefghijKLM");
    assert_eq!(terminal.cursor().unwrap(), (13, 0));
    terminal.resize(5, 3).unwrap();
    assert_eq!(screen(&terminal), "abcde\nfghij\nKLM");
    assert_eq!(terminal.cursor().unwrap(), (3, 2));
    // A hard line break never reflows into its neighbour.
    let mut hard = Terminal::new(10, 3, 100).unwrap();
    hard.feed(b"abc\r\ndef");
    hard.resize(20, 3).unwrap();
    assert_eq!(screen(&hard), "abc\ndef");
}

#[test]
fn the_alternate_screen_hides_and_restores_the_primary_one() {
    let mut terminal = Terminal::new(10, 3, 100).unwrap();
    terminal.feed(b"main");
    terminal.feed(b"\x1b[?1049h");
    assert!(terminal.alternate_screen().unwrap());
    assert!(terminal.mode(Mode::Dec(1049)).unwrap());
    assert_eq!(screen(&terminal), "");
    // 1049 saves the cursor and clears the screen but does not home it.
    terminal.feed(b"alt");
    assert_eq!(screen(&terminal), "    alt");
    terminal.feed(b"\x1b[?1049l");
    assert!(!terminal.alternate_screen().unwrap());
    assert_eq!(screen(&terminal), "main");
    assert_eq!(terminal.cursor().unwrap(), (4, 0));
}

#[test]
fn modes_title_and_directory_are_queryable() {
    let mut terminal = Terminal::new(10, 3, 100).unwrap();
    assert!(!terminal.mode(Mode::Dec(2004)).unwrap());
    assert!(!terminal.mode(Mode::Ansi(4)).unwrap());
    terminal.feed(b"\x1b[?2004h\x1b[4h\x1b]2;my title\x07\x1b]7;file:///tmp\x07");
    assert!(terminal.mode(Mode::Dec(2004)).unwrap());
    assert!(terminal.mode(Mode::Ansi(4)).unwrap());
    assert_eq!(terminal.title().unwrap(), b"my title");
    assert_eq!(terminal.pwd().unwrap(), b"file:///tmp");
}
