use shoal_term::{CellPoint, Rows, Scroll, Terminal};

fn at(row: u16, col: u16) -> CellPoint {
    CellPoint { row, col }
}

fn screen(t: &mut Terminal) -> Vec<String> {
    let mut rows = Vec::new();
    t.frame()
        .unwrap()
        .read_rows(Rows::All, |row| {
            rows.push(
                row.cells()
                    .iter()
                    .map(|c| {
                        let text = row.text(c);
                        if text.is_empty() { " " } else { text }
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_owned(),
            );
        })
        .unwrap();
    rows
}

#[test]
fn selection_and_scrolled_view_follow_content_after_output_and_reflow() {
    let mut t = Terminal::new(12, 3, 100).unwrap();
    t.feed("one\r\na界b\r\ncafé".as_bytes());
    t.begin_selection(at(1, 2), 1).unwrap(); // wide tail
    t.extend_selection(at(2, 2)).unwrap();
    assert_eq!(t.selection_text().unwrap(), "界b\ncaf");
    t.feed(b"\r\nnext\r\nlast");
    assert_eq!(t.selection_text().unwrap(), "界b\ncaf");
    t.scroll(Scroll::Top);
    let before = screen(&mut t);
    assert_eq!(before, ["one", "a界 b", "café"]);
    t.feed(b"\r\nmore\r\noutput");
    assert_eq!(screen(&mut t), before);
    t.resize(6, 3).unwrap();
    assert_eq!(t.selection_text().unwrap(), "界b\ncaf");
    t.clear_selection().unwrap();
    assert_eq!(t.selection_text().unwrap(), "");
}

#[test]
fn reverse_drag_word_line_and_soft_wrapped_selection_keep_unicode() {
    let mut t = Terminal::new(6, 3, 100).unwrap();
    t.feed("a界b\r\ncafé".as_bytes());
    t.begin_selection(at(1, 2), 1).unwrap();
    assert_eq!(t.selection_text().unwrap(), "");
    t.extend_selection(at(0, 2)).unwrap();
    assert_eq!(t.selection_text().unwrap(), "界b\ncaf");
    t.begin_selection(at(1, 3), 2).unwrap();
    assert_eq!(t.selection_text().unwrap(), "café");
    t.begin_selection(at(0, 2), 3).unwrap();
    assert_eq!(t.selection_text().unwrap(), "a界b");
    t.feed(b"\x1bcabcdefghij");
    t.begin_selection(at(0, 0), 1).unwrap();
    t.extend_selection(at(1, 3)).unwrap();
    assert_eq!(t.selection_text().unwrap(), "abcdefghij");
}

#[test]
fn prompt_navigation_and_overscan_use_retained_terminal_state() {
    let mut t = Terminal::new(20, 3, 100).unwrap();
    t.feed(b"\x1b]133;A\x07first\x1b]133;B\x07\r\n\x1b]133;C\x07a\r\nb\r\n\x1b]133;D;0\x07\x1b]133;A\x07second\x1b]133;B\x07\r\n\x1b]133;C\x07c\r\nd\r\n\x1b]133;D;0\x07\x1b]133;A\x07third\x1b]133;B\x07");
    t.scroll(Scroll::Prompts(-1));
    assert_eq!(screen(&mut t)[0], "second");
    t.scroll(Scroll::Prompts(-1));
    assert_eq!(screen(&mut t)[0], "first");
    t.scroll(Scroll::Prompts(1));
    assert_eq!(screen(&mut t)[0], "second");
    t.set_overscan(1).unwrap();
    assert_eq!(screen(&mut t), ["second", "c", "d", "third"]);
    t.scroll(Scroll::Bottom);
    assert!(t.viewport().unwrap().at_bottom);
    assert_eq!(screen(&mut t), ["c", "d", "third"]);
}

#[test]
fn osc8_links_are_read_from_history_and_survive_a_snapshot() {
    let mut t = Terminal::new(20, 2, 100).unwrap();
    t.feed(b"\x1b]8;;https://example.org/a\x1b\\label\x1b]8;;\x1b\\\r\nnext\r\nlast");
    let mut mirror = Terminal::restore(&t.snapshot().unwrap(), 20, 2, 100).unwrap();
    mirror.scroll(Scroll::Top);
    assert_eq!(
        mirror.hyperlink(at(0, 1)).unwrap(),
        Some("https://example.org/a".into())
    );
    assert_eq!(mirror.hyperlink(at(1, 1)).unwrap(), None);
}

#[test]
fn discarded_selection_does_not_retarget_unrelated_content() {
    let mut t = Terminal::new(12, 2, 1).unwrap();
    t.feed(b"selected\r\nsecond");
    t.begin_selection(at(0, 1), 2).unwrap();
    assert_eq!(t.selection_text().unwrap(), "selected");
    for _ in 0..10000 {
        t.feed(b"\r\nreplacement");
    }
    assert_eq!(t.selection_text().unwrap(), "");
    t.extend_selection(at(0, 0)).unwrap();
    assert_eq!(t.selection_text().unwrap(), "");
    t.begin_selection(at(1, 0), 2).unwrap();
    t.feed(b"\x1bcfresh");
    assert_eq!(t.selection_text().unwrap(), "");
}

#[test]
fn history_erase_clears_selection_even_when_later_output_replaces_history() {
    let mut t = Terminal::new(12, 2, 100).unwrap();
    t.feed(b"selected\r\nnext\r\nlast");
    t.scroll(Scroll::Top);
    t.begin_selection(at(0, 1), 2).unwrap();
    // An OSC containing the same bytes is not an erase.
    t.feed(b"\x1b]2;not a clear: [3J\x07");
    assert_eq!(t.selection_text().unwrap(), "selected");
    // Split an actual sequence across feeds, followed by enough output to
    // grow history beyond its previous size in the very same write.
    t.feed(b"\x1b[");
    t.feed(b"3J\r\nnew\r\nmore\r\nreplacement");
    assert_eq!(t.selection_text().unwrap(), "");
    assert!(t.frame().unwrap().selection().unwrap().is_empty());
    t.scroll(Scroll::Top);
    t.begin_selection(at(0, 1), 2).unwrap();
    t.set_scrollback(0).unwrap();
    assert_eq!(t.selection_text().unwrap(), "");
}

#[test]
fn clearing_a_selection_on_the_other_screen_cannot_leave_a_stale_highlight() {
    let mut t = Terminal::new(12, 2, 100).unwrap();
    t.feed(b"selected");
    t.begin_selection(at(0, 1), 2).unwrap();
    t.feed(b"\x1b[?1049hother");
    assert_eq!(t.selection_text().unwrap(), "");
    t.clear_selection().unwrap();
    t.feed(b"\x1b[?1049l");
    assert_eq!(t.selection_text().unwrap(), "");
    assert!(t.frame().unwrap().selection().unwrap().is_empty());
}
