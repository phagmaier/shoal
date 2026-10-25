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
