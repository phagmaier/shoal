//! T00 snapshot fidelity harness.
//!
//! For each scenario: feed bytes into terminal A, dump A with the VT formatter
//! and restore the dump into a fresh terminal B, and (for comparison) encode A
//! with libghostty's binary snapshot codec and decode it into C. Then compare
//! B and C against A cell by cell, mode by mode, and on other state. Finally
//! feed the same follow-up bytes to all three (what a mirror receives after
//! attaching) and compare again, which catches hidden state such as scroll
//! regions, tab stops, charsets and saved cursors.
//!
//! Usage: snapcheck [--programs] [--verbose]

mod compare;
mod programs;
mod scenarios;

use compare::{Report, compare};
use libghostty_vt::fmt::{Format, Formatter, FormatterOptions};
use libghostty_vt::snapshot::Decoder;
use libghostty_vt::terminal::Terminal;

pub const SCROLLBACK_LINES: usize = 10_000;

pub struct Outcome {
    pub name: String,
    pub dump_bytes: usize,
    pub snap_bytes: usize,
    pub fmt_restore: Report,
    pub fmt_after: Report,
    pub cont_after: Report,
    pub snap_restore: Report,
    pub snap_after: Report,
}

pub fn new_terminal(cols: u16, rows: u16) -> Terminal<'static, 'static> {
    let mut t = Terminal::new(cols, rows).expect("terminal");
    t.set_scrollback_max_lines(Some(SCROLLBACK_LINES))
        .expect("scrollback limit");
    // The default byte limit is 10,000 bytes, which keeps almost no history
    // and makes writing very slow. Let the line limit govern instead.
    t.set_scrollback_max_bytes(std::env::var("SNAPCHECK_MAX_BYTES").ok().and_then(|v| v.parse().ok()))
        .expect("scrollback byte limit");
    // Keep the bytes of an unfinished escape sequence, so a snapshot taken
    // mid-sequence can be completed (binary snapshot, formatter + continuation).
    t.set_continuation_max_bytes(4096)
        .expect("continuation tracking");
    t
}

/// The formatter options the daemon would use for an attach snapshot:
/// everything the formatter can emit.
pub fn snapshot_options<'t, 's>() -> FormatterOptions<'t, 's> {
    FormatterOptions::new()
        .with_format(Format::Vt)
        .with_unwrap(false)
        .with_trim(false)
        .with_palette(true)
        .with_modes(true)
        .with_scrolling_region(true)
        .with_tabstops(true)
        .with_pwd(true)
        .with_keyboard(true)
        .with_cursor(true)
        .with_style(true)
        .with_hyperlink(true)
        .with_protection(true)
        .with_kitty_keyboard(true)
        .with_charsets(true)
}

pub fn formatter_dump(t: &Terminal<'static, 'static>) -> Vec<u8> {
    let mut f = Formatter::new(t, snapshot_options()).expect("formatter");
    f.format_alloc(None).expect("format").to_vec()
}

/// Restore terminal A three ways (formatter, formatter + continuation, binary
/// snapshot), compare, then feed every chunk `next_after` yields to all of
/// them (live, so A can answer queries in between) and compare again.
pub fn check(
    name: &str,
    a: &mut Terminal<'static, 'static>,
    mut next_after: impl FnMut() -> Option<Vec<u8>>,
) -> Outcome {
    let cols = a.cols().expect("cols");
    let rows = a.rows().expect("rows");

    let dump = formatter_dump(a);
    let mut b = new_terminal(cols, rows);
    b.vt_write(&dump);

    // Formatter dump plus the parser's unfinished-sequence bytes.
    let cont = a
        .continuation_alloc(None)
        .ok()
        .flatten()
        .map(|bytes| bytes.to_vec())
        .unwrap_or_default();
    let mut d = new_terminal(cols, rows);
    d.vt_write(&dump);
    d.vt_write(&cont);

    let snap = a
        .encode_snapshot_alloc(None)
        .map(|bytes| bytes.map(|b| b.to_vec()).unwrap_or_default());
    let mut c = match &snap {
        Ok(bytes) => Decoder::new_buf(bytes).and_then(|d| d.decode()).ok(),
        Err(_) => None,
    };
    if let Some(c) = c.as_mut() {
        c.set_scrollback_max_lines(Some(SCROLLBACK_LINES))
            .expect("scrollback limit");
    }

    let fmt_restore = compare(a, &b);
    let snap_restore = c.as_ref().map_or_else(Report::failed, |c| compare(a, c));

    while let Some(after) = next_after() {
        a.vt_write(&after);
        b.vt_write(&after);
        d.vt_write(&after);
        if let Some(c) = c.as_mut() {
            c.vt_write(&after);
        }
    }

    Outcome {
        name: name.to_owned(),
        dump_bytes: dump.len(),
        snap_bytes: snap.as_ref().map_or(0, Vec::len),
        fmt_restore,
        fmt_after: compare(a, &b),
        cont_after: compare(a, &d),
        snap_restore,
        snap_after: c.as_ref().map_or_else(Report::failed, |c| compare(a, c)),
    }
}

/// Time dump + restore of a 200 x 60 terminal holding 10,000 styled lines.
fn bench() {
    use std::time::Instant;
    let mut a = new_terminal(200, 60);
    let mut text = String::new();
    for i in 0..10_000 {
        text.push_str(&format!(
            "{i:05} \x1b[1;32mok\x1b[0m \x1b[38;5;208mvalue={}\x1b[0m some plain words to fill the line out a bit \x1b[3mitalic\x1b[0m\r\n",
            i * 7
        ));
    }
    println!(
        "defaults: scrollback max bytes {:?}, max lines {:?}",
        a.scrollback_max_bytes(),
        a.scrollback_max_lines()
    );
    let t = Instant::now();
    a.vt_write(text.as_bytes());
    println!(
        "fed {} KiB in {:.1} ms; terminal: 200 x 60, {} total rows",
        text.len() / 1024,
        t.elapsed().as_secs_f64() * 1e3,
        a.total_rows().unwrap_or(0)
    );
    {
        let dump = formatter_dump(&a);
        for tracking in [false, true] {
            let mut b = new_terminal(200, 60);
            if tracking {
                b.set_continuation_max_bytes(4096).expect("tracking");
            }
            let t = Instant::now();
            b.vt_write(&dump);
            println!("restore with tracking={tracking}: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        }
        std::fs::write("/tmp/shoal-research/dump.vt", &dump).ok();
    }
    for _ in 0..3 {
        let t0 = Instant::now();
        let dump = formatter_dump(&a);
        let t1 = Instant::now();
        let mut b = new_terminal(200, 60);
        b.vt_write(&dump);
        let t2 = Instant::now();
        let snap = a.encode_snapshot_alloc(None).expect("encode").map(|b| b.to_vec()).unwrap_or_default();
        let t3 = Instant::now();
        let c = Decoder::new_buf(&snap).and_then(|d| d.decode()).expect("decode");
        let t4 = Instant::now();
        println!(
            "formatter: dump {:.1} ms + restore {:.1} ms ({} KiB) | binary: encode {:.1} ms + decode {:.1} ms ({} KiB) | rows {} / {}",
            (t1 - t0).as_secs_f64() * 1e3,
            (t2 - t1).as_secs_f64() * 1e3,
            dump.len() / 1024,
            (t3 - t2).as_secs_f64() * 1e3,
            (t4 - t3).as_secs_f64() * 1e3,
            snap.len() / 1024,
            b.total_rows().unwrap_or(0),
            c.total_rows().unwrap_or(0),
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--bench") {
        bench();
        return;
    }
    let verbose = args.iter().any(|a| a == "--verbose");
    let with_programs = args.iter().any(|a| a == "--programs");

    let mut outcomes = Vec::new();
    for s in scenarios::all() {
        let mut a = new_terminal(s.cols, s.rows);
        a.vt_write(&s.setup);
        let mut after = Some(s.after);
        outcomes.push(check(s.name, &mut a, || after.take()));
    }
    if with_programs {
        outcomes.extend(programs::run_all());
    }

    println!("| Scenario | Formatter: restore | Formatter: after more output | Formatter + continuation: after | Binary snapshot: restore | Binary snapshot: after | Dump bytes (fmt / bin) |");
    println!("| --- | --- | --- | --- | --- | --- | --- |");
    for o in &outcomes {
        println!(
            "| {} | {} | {} | {} | {} | {} | {} / {} |",
            o.name,
            o.fmt_restore.summary(),
            o.fmt_after.summary(),
            o.cont_after.summary(),
            o.snap_restore.summary(),
            o.snap_after.summary(),
            o.dump_bytes,
            o.snap_bytes,
        );
    }
    if verbose {
        for o in &outcomes {
            for (label, r) in [
                ("formatter restore", &o.fmt_restore),
                ("formatter after", &o.fmt_after),
                ("formatter + continuation after", &o.cont_after),
                ("snapshot restore", &o.snap_restore),
                ("snapshot after", &o.snap_after),
            ] {
                if !r.is_match() {
                    println!("\n## {} / {label}\n{}", o.name, r.details());
                }
            }
        }
    }
}
