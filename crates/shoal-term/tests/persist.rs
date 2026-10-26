use shoal_term::{CaptureFormat, Persisted, Terminal};

/// Primary-screen output with history, styles and a soft-wrapped line.
fn primary() -> Vec<u8> {
    let mut bytes: Vec<u8> = (0..40)
        .flat_map(|i| format!("\x1b[3{}mline {i}\x1b[0m\r\n", i % 8).into_bytes())
        .collect();
    bytes.extend_from_slice(b"a soft-wrapped line longer than twenty\r\n$ ");
    bytes
}

fn replay(vt: &[u8]) -> Terminal {
    let mut terminal = Terminal::new(20, 5, 1000).unwrap();
    terminal.feed(vt);
    terminal
}

#[test]
fn format_vt_saves_the_primary_screen_beneath_every_alternate_screen() {
    // The reference never left its primary screen.
    let mut reference = Terminal::new(20, 5, 1000).unwrap();
    reference.feed(&primary());
    let want_vt = reference.capture(CaptureFormat::Vt, true).unwrap();
    let want_text = reference.capture(CaptureFormat::Text, true).unwrap();
    assert!(String::from_utf8_lossy(&want_text).starts_with("line 0\nline 1\n"));
    assert!(matches!(reference.persisted().unwrap(), Persisted::Vt(_)));
    let saved = replay(&reference.format_vt().unwrap());
    assert_eq!(saved.capture(CaptureFormat::Vt, true).unwrap(), want_vt);

    for mode in [47, 1047, 1049] {
        let mut live = Terminal::new(20, 5, 1000).unwrap();
        live.feed(&primary());
        live.feed(format!("\x1b[?{mode}hFULL SCREEN").as_bytes());
        let showing = live.capture(CaptureFormat::Vt, true).unwrap();
        assert!(matches!(
            live.persisted().unwrap(),
            Persisted::Snapshot { .. }
        ));

        let saved = replay(&live.format_vt().unwrap());
        assert!(!saved.alternate_screen().unwrap(), "mode {mode}");
        assert_eq!(
            saved.capture(CaptureFormat::Text, true).unwrap(),
            want_text,
            "mode {mode}"
        );
        assert_eq!(
            saved.capture(CaptureFormat::Vt, true).unwrap(),
            want_vt,
            "mode {mode}"
        );
        // The live terminal is untouched and still on its alternate screen.
        assert!(live.alternate_screen().unwrap(), "mode {mode}");
        assert_eq!(live.capture(CaptureFormat::Vt, true).unwrap(), showing);
    }
}

#[test]
fn persisted_snapshots_cross_threads_for_the_worker() {
    let mut live = Terminal::new(20, 5, 1000).unwrap();
    live.feed(&primary());
    live.feed(b"\x1b[?1049hvim");
    let persisted = live.persisted().unwrap();
    let vt = std::thread::spawn(move || persisted.into_vt().unwrap())
        .join()
        .unwrap();
    let text = replay(&vt).capture(CaptureFormat::Text, false).unwrap();
    assert!(String::from_utf8_lossy(&text).ends_with("$"), "{text:?}");
}

#[test]
fn replayed_output_is_inert_and_sits_above_the_divider() {
    use shoal_term::Mode;
    // A program left everything on, and the file ends inside an OSC.
    let mut saved = primary();
    saved.extend_from_slice(
        b"\x1b[1;31m\x1b[4h\x1b[?6h\x1b[?25l\x1b[?1h\x1b[?1000;1006;2004;1004;2026h\
          \x1b[?1049hFULL SCREEN\x1b]0;unfinished",
    );
    let mut terminal = Terminal::new(20, 5, 1000).unwrap();
    terminal
        .replay_saved(&saved, "── restored ──\x1b[31m")
        .unwrap();
    assert!(terminal.is_ground().unwrap());
    assert!(!terminal.alternate_screen().unwrap());
    assert!(terminal.mode(Mode::Dec(25)).unwrap(), "cursor shown");
    assert!(terminal.mode(Mode::Dec(7)).unwrap(), "autowrap");
    for mode in [1, 6, 1000, 1004, 1006, 2004, 2026] {
        assert!(!terminal.mode(Mode::Dec(mode)).unwrap(), "?{mode}");
    }
    assert!(!terminal.mode(Mode::Ansi(4)).unwrap(), "insert");
    assert_eq!(terminal.cursor().unwrap(), (0, 4));
    let text = String::from_utf8(terminal.capture(CaptureFormat::Text, true).unwrap()).unwrap();
    let marker = text.find("line 39").unwrap();
    // Control characters in the divider are dropped, never interpreted.
    let divider = text.find("── restored ──[31m").unwrap();
    assert!(marker < divider, "{text}");
    assert!(!text.contains("FULL SCREEN"), "{text}");
    // The divider starts its own line after the old prompt, and is dim.
    assert!(text.contains("$\n── restored"), "{text:?}");
    // The divider is dim, and new output below it starts unstyled.
    terminal.feed(b"fresh");
    let vt = String::from_utf8(terminal.capture(CaptureFormat::Vt, false).unwrap()).unwrap();
    assert!(
        vt.contains("$ \r\n\x1b[0m\x1b[2m── restored ──[31m\x1b[0m\r\nfresh"),
        "{vt:?}"
    );
}

#[test]
fn the_divider_goes_below_the_last_text_wherever_the_cursor_was() {
    // Saved bytes, the divider's row, then the cursor's row.
    for (saved, divider, cursor) in [
        (&b""[..], 0, 1),
        (b"one\r\ntwo\x1b[H", 2, 3),
        (b"one\x1b[5;10H", 1, 2),
        // A full screen scrolls to make room.
        (b"1\r\n2\r\n3\r\n4\r\n5", 3, 4),
    ] {
        let mut terminal = Terminal::new(20, 5, 1000).unwrap();
        terminal.replay_saved(saved, "divider").unwrap();
        let text = terminal.capture(CaptureFormat::Text, false).unwrap();
        let text = String::from_utf8(text).unwrap();
        let lines: Vec<_> = text.split('\n').collect();
        assert_eq!(
            lines.iter().position(|line| *line == "divider"),
            Some(divider),
            "{text:?}"
        );
        assert_eq!(terminal.cursor().unwrap(), (0, cursor), "{text:?}");
    }
}

#[test]
fn replay_aborts_unfinished_utf8_and_dcs() {
    for tail in [&b"\xe2\x82"[..], b"\x1bP1$q", b"\x1b[38;2;1"] {
        let mut terminal = Terminal::new(20, 5, 1000).unwrap();
        let mut saved = b"old".to_vec();
        saved.extend_from_slice(tail);
        terminal.replay_saved(&saved, "divider").unwrap();
        assert!(terminal.is_ground().unwrap(), "{tail:?}");
        let text = terminal.capture(CaptureFormat::Text, false).unwrap();
        assert!(
            String::from_utf8_lossy(&text).contains("\ndivider"),
            "{tail:?}: {text:?}"
        );
    }
}
