//! Focused checks behind the M1 attach and persistence spec text (added after
//! the recorded T00 runs, 2026-10-25):
//!
//! - an unfinished OSC longer than the 4096-byte continuation cap, and the
//!   recovery at the first ground point with `vt_write_until_ground`;
//! - saving the main screen while an alternate screen is active, through a
//!   decoded scratch terminal;
//! - truncated and trailing snapshot bytes, and where the consumed length can
//!   be read at this pin.
//!
//! Prints what it observes. Like the rest of snapcheck it never fails, so read
//! the output.

use std::io::Read;

use crate::compare::{Report, compare};
use crate::{formatter_dump, new_terminal};
use libghostty_vt::build_info;
use libghostty_vt::snapshot::Decoder;
use libghostty_vt::terminal::Terminal;

type Term = Terminal<'static, 'static>;

fn encode(t: &Term) -> Result<Vec<u8>, String> {
    t.encode_snapshot_alloc(None)
        .map(|b| b.map(|b| b.to_vec()).unwrap_or_default())
        .map_err(|e| format!("{e:?}"))
}

fn decode(bytes: &[u8]) -> Result<Term, String> {
    Decoder::new_buf(bytes)
        .and_then(|d| d.decode())
        .map_err(|e| format!("{e:?}"))
}

fn show(label: &str, r: &Report) {
    println!("  {label}: {}", r.summary());
    if !r.is_match() {
        for line in r.details().lines() {
            println!("    {line}");
        }
    }
}

fn overlong_osc(len: usize, terminator: &[u8], label: &str) {
    println!("\n## Unfinished OSC 2 of {len} bytes, then {label}");
    let mut a = new_terminal(80, 24);
    a.vt_write(b"before\r\n\x1b]2;");
    a.vt_write(&vec![b'x'; len]);
    println!("  is_vt_ground: {:?}", a.is_vt_ground());
    println!(
        "  continuation size query: {:?}",
        a.continuation_buf(&mut [])
    );
    match encode(&a) {
        Ok(b) => println!("  encode while unfinished: ok, {} bytes", b.len()),
        Err(e) => println!("  encode while unfinished: error {e}"),
    }

    // Output with no ground point is consumed whole.
    println!(
        "  vt_write_until_ground(\"zz\"): {:?}",
        a.vt_write_until_ground(b"zz")
    );

    let mut chunk = b"yy".to_vec();
    chunk.extend_from_slice(terminator);
    chunk.extend_from_slice(b"after\r\n");
    let consumed = a.vt_write_until_ground(&chunk);
    println!(
        "  vt_write_until_ground(next chunk, {} bytes): {consumed:?}",
        chunk.len()
    );
    let split = consumed.ok().flatten().unwrap_or(chunk.len());
    println!("  is_vt_ground after split: {:?}", a.is_vt_ground());
    let snap = match encode(&a) {
        Ok(b) => {
            println!("  encode at the split: ok, {} bytes", b.len());
            b
        }
        Err(e) => {
            println!("  encode at the split: error {e}");
            return;
        }
    };
    let mut c = match decode(&snap) {
        Ok(c) => c,
        Err(e) => {
            println!("  decode: error {e}");
            return;
        }
    };
    show("decoded vs source at the split", &compare(&a, &c));
    a.vt_write(&chunk[split..]);
    c.vt_write(&chunk[split..]);
    show("after the rest of the chunk", &compare(&a, &c));
    println!(
        "  title bytes: source {}, decoded {}",
        a.title().map_or(0, <[u8]>::len),
        c.title().map_or(0, <[u8]>::len)
    );
}

fn primary_output() -> String {
    let mut s = String::new();
    for i in 0..100 {
        s.push_str(&format!("\x1b[3{}mprimary line {i}\x1b[0m\r\n", i % 8));
    }
    s.push_str("prompt$ ");
    s
}

fn alt_screen_persist(mode: u16, leave: &[u8]) {
    println!("\n## Save the main screen under alternate screen mode {mode}");
    let primary = primary_output();
    let mut a = new_terminal(80, 24);
    a.vt_write(primary.as_bytes());
    a.vt_write(format!("\x1b[?{mode}h").as_bytes());
    a.vt_write(b"\x1b[2J\x1b[HALT SCREEN\r\nfull-screen program");

    // Oracle: the same main-screen output, never entering the alternate screen.
    let mut expected = new_terminal(80, 24);
    expected.vt_write(primary.as_bytes());

    let mut direct = new_terminal(80, 24);
    direct.vt_write(&formatter_dump(&a));
    show(
        "formatter on the live terminal (current rule)",
        &compare(&expected, &direct),
    );

    let before = a.active_screen();
    let snap = match encode(&a) {
        Ok(b) => b,
        Err(e) => {
            println!("  encode: error {e}");
            return;
        }
    };
    let mut scratch = match decode(&snap) {
        Ok(t) => t,
        Err(e) => {
            println!("  decode: error {e}");
            return;
        }
    };
    scratch.vt_write(leave);
    let mut restored = new_terminal(80, 24);
    restored.vt_write(&formatter_dump(&scratch));
    show(
        &format!(
            "snapshot -> scratch -> {} -> formatter (cursor is irrelevant to persistence)",
            String::from_utf8_lossy(leave).replace('\x1b', "ESC")
        ),
        &compare(&expected, &restored),
    );
    println!(
        "  live terminal screen before / after: {before:?} / {:?}",
        a.active_screen()
    );
}

/// Counts the bytes a reader-based decoder pulls from its source.
struct Counting<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Read for Counting<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.data.len() - self.pos);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn consumption(cols: u16, rows: u16, lines: usize) {
    println!(
        "\n## Truncated and trailing snapshot bytes, {cols}x{rows} with {lines} lines written"
    );
    let mut a = new_terminal(cols, rows);
    for i in 0..lines {
        a.vt_write(format!("\x1b[3{}mhistory line {i}\x1b[0m\r\n", i % 8).as_bytes());
    }
    let snap = match encode(&a) {
        Ok(b) => b,
        Err(e) => {
            println!("  encode: error {e}");
            return;
        }
    };
    println!("  snapshot bytes: {}", snap.len());
    println!(
        "  one-shot decode, last byte missing: {:?}",
        decode(&snap[..snap.len() - 1]).map(|_| "ok")
    );
    let mut trailing = snap.clone();
    trailing.extend_from_slice(b"JUNK");
    println!(
        "  one-shot decode, 4 trailing bytes: {:?}",
        decode(&trailing).map(|_| "ok")
    );

    let mut reader = Counting {
        data: &trailing,
        pos: 0,
    };
    let result = Decoder::new(&mut reader)
        .and_then(|d| d.decode())
        .map(|_| "ok");
    println!(
        "  reader decode with trailing bytes: {result:?}, bytes read {}",
        reader.pos
    );

    match Decoder::new_buf(&trailing).and_then(Decoder::ready) {
        Ok(mut inc) => {
            let mut pages = 0;
            let mut last_offset = None;
            loop {
                match inc.next() {
                    Ok(Some(p)) => {
                        pages += 1;
                        last_offset = p.as_decoder().source_offset().ok();
                    }
                    Ok(None) => break,
                    Err(e) => {
                        println!("  incremental decode: error {e:?}");
                        return;
                    }
                }
            }
            println!(
                "  incremental decode: {pages} history pages, source offset after the last page {last_offset:?}"
            );
        }
        Err(e) => println!("  incremental decode: ready error {e:?}"),
    }
}

pub fn run() {
    println!("# Design checks");
    let version = build_info::version_string().map(|v| String::from_utf8_lossy(v).into_owned());
    println!(
        "libghostty-vt {version:?}, optimize {:?}, simd {:?}, kitty graphics {:?}, tmux control {:?}",
        build_info::optimize_mode(),
        build_info::supports_simd(),
        build_info::supports_kitty_graphics(),
        build_info::supports_tmux_control_mode(),
    );
    overlong_osc(4000, b"\x07", "BEL (control: under the 4096-byte cap)");
    overlong_osc(5000, b"\x07", "BEL");
    overlong_osc(5000, b"\x1b\\", "ST");
    for mode in [1049, 1047, 47] {
        alt_screen_persist(mode, b"\x1b[?1049l");
        alt_screen_persist(mode, b"\x1b[?1049l\x1b[?1047l\x1b[?47l");
    }
    consumption(80, 24, 300);
    consumption(200, 60, 10_000);
}
