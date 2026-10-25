//! Decode memory measured as resident memory, since terminal pages are mapped
//! straight from the OS. One test in its own process keeps the numbers clean.

use shoal_term::{RestoreError, Terminal};

const MIB: f64 = 1024.0 * 1024.0;

fn status_bytes(key: &str) -> usize {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|line| line.starts_with(key)).unwrap();
    line.split_whitespace()
        .nth(1)
        .unwrap()
        .parse::<usize>()
        .unwrap()
        * 1024
}

/// Peak resident growth while `decode` runs, with the high-water mark reset.
fn peak_growth<T>(decode: impl FnOnce() -> T) -> (T, usize) {
    std::fs::write("/proc/self/clear_refs", "5").unwrap();
    let before = status_bytes("VmRSS:");
    let result = decode();
    (result, status_bytes("VmHWM:").saturating_sub(before))
}

fn snapshot(cols: u16, rows: u16, lines: usize, styled: bool) -> Vec<u8> {
    let mut terminal = Terminal::new(cols, rows, u32::MAX).unwrap();
    for i in 0..lines {
        let line = if styled {
            format!(
                "\x1b[3{}m{i:05} \x1b[1mbold\x1b[0m {} \x1b[4munder\x1b[0m\r\n",
                i % 8,
                "x".repeat(i % 150)
            )
        } else {
            format!("{i}\r\n")
        };
        terminal.feed(line.as_bytes());
    }
    terminal.snapshot().unwrap()
}

#[test]
fn decode_memory_is_measured_and_the_cell_budget_refuses_before_paging_history() {
    // The T07 measurement: 200 × 60 with 10,000 styled lines of history.
    let bytes = snapshot(200, 60, 10_000, true);
    let (mirror, peak) = peak_growth(|| Terminal::restore(&bytes, 200, 60, 10_000).unwrap());
    assert_eq!(mirror.scrollback_rows().unwrap(), 9_941);
    println!(
        "200x60 with 10,000 lines: snapshot {:.1} MiB, peak decode growth {:.1} MiB",
        bytes.len() as f64 / MIB,
        peak as f64 / MIB
    );
    assert!(peak < 64 << 20, "{peak} bytes");
    drop(mirror);

    // Within every row and size limit, but 99,060,000 declared cells: about
    // 390 MiB of pages if decoded. Refused at the first history page instead.
    let wide = snapshot(1000, 60, 99_000, false);
    let (result, peak) = peak_growth(|| Terminal::restore(&wide, 1000, 60, 100_000));
    assert!(
        matches!(result, Err(RestoreError::Cells { .. })),
        "{result:?}"
    );
    println!(
        "refused 1000x60 with 99,000 lines after {:.1} MiB",
        peak as f64 / MIB
    );
    assert!(peak < 32 << 20, "{peak} bytes");
}
