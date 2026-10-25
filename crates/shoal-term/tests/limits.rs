use shoal_term::{MAX_HISTORY_ROWS, RestoreError, Terminal};

fn snapshot(cols: u16, rows: u16, lines: usize) -> Vec<u8> {
    let mut terminal = Terminal::new(cols, rows, u32::MAX).unwrap();
    for i in 0..lines {
        terminal.feed(format!("{i}\r\n").as_bytes());
    }
    terminal.snapshot().unwrap()
}

#[test]
fn truncated_and_corrupted_snapshots_are_errors() {
    let bytes = snapshot(20, 5, 2000);
    assert!(Terminal::restore(&bytes, 20, 5, 100).is_ok());
    for len in [
        0,
        1,
        9,
        10,
        30,
        bytes.len() / 2,
        bytes.len() - 10,
        bytes.len() - 1,
    ] {
        let result = Terminal::restore(&bytes[..len], 20, 5, 100);
        assert!(
            matches!(result, Err(RestoreError::Decode(_))),
            "truncated at {len}: {result:?}"
        );
    }
    // Every byte is covered by the magic, the version or a record checksum.
    for at in (0..bytes.len()).step_by(7) {
        let mut corrupt = bytes.clone();
        corrupt[at] ^= 0x20;
        let result = Terminal::restore(&corrupt, 20, 5, 100);
        assert!(result.is_err(), "flipped byte {at} still decoded");
    }
}

#[test]
fn bytes_after_finish_are_an_error() {
    let bytes = snapshot(20, 5, 50);
    let len = bytes.len();
    let padded = [&bytes[..], &[0, 1, 2, 3]].concat();
    let result = Terminal::restore(&padded, 20, 5, 100);
    assert!(
        matches!(result, Err(RestoreError::TrailingBytes { consumed, len: total }) if consumed == len && total == len + 4),
        "{result:?}"
    );
    // A second complete snapshot appended is trailing bytes too.
    let doubled = [&bytes[..], &bytes[..]].concat();
    assert!(matches!(
        Terminal::restore(&doubled, 20, 5, 100),
        Err(RestoreError::TrailingBytes { .. })
    ));
}

#[test]
fn sizes_must_be_in_range_and_match_the_decoded_terminal() {
    let bytes = snapshot(20, 5, 10);
    for (cols, rows) in [(0, 5), (20, 0), (1001, 5), (20, 1001)] {
        assert!(matches!(
            Terminal::restore(&bytes, cols, rows, 100),
            Err(RestoreError::Size { .. })
        ));
    }
    for (cols, rows) in [(21, 5), (20, 4)] {
        assert!(matches!(
            Terminal::restore(&bytes, cols, rows, 100),
            Err(RestoreError::SizeMismatch {
                got_cols: 20,
                got_rows: 5,
                ..
            })
        ));
    }
    let edge = snapshot(1000, 1, 3);
    let terminal = Terminal::restore(&edge, 1000, 1, 100).unwrap();
    assert_eq!(terminal.cols().unwrap(), 1000);
}

#[test]
fn declared_history_over_the_limit_is_refused_and_just_under_is_accepted() {
    let rows = usize::try_from(MAX_HISTORY_ROWS).unwrap();
    let over = snapshot(10, 2, rows + 1000);
    match Terminal::restore(&over, 10, 2, 100) {
        Err(RestoreError::History { rows: declared }) => assert!(declared > MAX_HISTORY_ROWS),
        other => panic!("{other:?}"),
    }
    let under = snapshot(10, 2, rows - 1000);
    let terminal = Terminal::restore(&under, 10, 2, 200_000).unwrap();
    // The 2-row screen holds the last line and the cursor's empty row.
    assert_eq!(terminal.scrollback_rows().unwrap(), rows - 1001);
}

#[test]
fn a_restored_mirror_keeps_only_its_configured_scrollback() {
    let bytes = snapshot(20, 5, 20_000);
    let terminal = Terminal::restore(&bytes, 20, 5, 1000).unwrap();
    let history = terminal.scrollback_rows().unwrap();
    // libghostty prunes whole pages, so a little more than the limit remains.
    assert!((1000..20_000).contains(&history), "{history} rows kept");
}
