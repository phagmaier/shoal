//! Headless cost breakdown of the draw-list builder on the full-damage
//! styled benchmark screen: what each libghostty cell accessor costs.
//! `cargo run --release --bin rowbench`. T00 diagnosis only.

#[path = "../bench.rs"]
#[allow(dead_code)]
mod bench;
#[path = "../draw.rs"]
#[allow(dead_code)]
mod draw;
#[path = "../draw_ref.rs"]
#[allow(dead_code)]
mod draw_ref;

use std::hint::black_box;
use std::time::{Duration, Instant};

use libghostty_vt::render::{CellIterator, RenderState, RowIterator};
use libghostty_vt::terminal::Terminal;

const ITERS: usize = 300;

fn main() {
    let mut term = Terminal::new(200, 60).expect("terminal");
    term.set_scrollback_max_lines(Some(10_000)).expect("scrollback");
    // SAFETY: no other thread exists yet to read the environment.
    unsafe { std::env::set_var("SPIKE_BENCH", "100000") };
    let mut fixture = bench::Bench::from_env().expect("bench");
    let mut builder = draw::Builder::new();
    let mut state = RenderState::new().expect("state");
    let mut rows = RowIterator::new().expect("rows");
    let mut cells = CellIterator::new().expect("cells");
    let mut text = String::new();

    let mut times: Vec<(&str, Duration)> = Vec::new();
    let mut add = |name: &'static str, d: Duration| match times.iter_mut().find(|t| t.0 == name) {
        Some(t) => t.1 += d,
        None => times.push((name, d)),
    };
    let cell_count = 200 * 60;
    if std::env::args().nth(1).as_deref() == Some("build") {
        // Profiling target: only the cached builder, on full damage.
        for _ in 0..ITERS * 10 {
            fixture.before_frame(&mut term);
            black_box(builder.build(&term));
        }
        return;
    }
    for _ in 0..ITERS {
        fixture.before_frame(&mut term);
        let t = Instant::now();
        black_box(builder.build(&term));
        add("Builder::build (update + all dirty rows)", t.elapsed());

        // Same screen again, through a second state that sees it as new.
        fixture.before_frame(&mut term);
        let t = Instant::now();
        let snapshot = state.update(&term).expect("update");
        add("RenderState::update", t.elapsed());
        let mut it = rows.update(&snapshot).expect("rows");
        let mut sum = 0u64;
        let t = Instant::now();
        while let Some(row) = it.next() {
            let mut c = cells.update(row).expect("cells");
            while let Some(cell) = c.next() {
                sum += 1;
                black_box(&cell);
            }
        }
        add("cell iteration only", t.elapsed());
        let mut it = rows.update(&snapshot).expect("rows");
        let t = Instant::now();
        while let Some(row) = it.next() {
            let mut c = cells.update(row).expect("cells");
            while let Some(cell) = c.next() {
                let raw = cell.raw_cell().expect("raw");
                sum += raw.codepoint().unwrap_or(0) as u64;
            }
        }
        add("iteration + raw_cell + codepoint", t.elapsed());
        let mut it = rows.update(&snapshot).expect("rows");
        let t = Instant::now();
        while let Some(row) = it.next() {
            let mut c = cells.update(row).expect("cells");
            while let Some(cell) = c.next() {
                let raw = cell.raw_cell().expect("raw");
                sum += raw.codepoint().unwrap_or(0) as u64;
                sum += raw.wide().map_or(0, |w| w as u64);
                sum += raw.content_tag().map_or(0, |w| w as u64);
                sum += raw.style_id().is_ok() as u64;
            }
        }
        add("iteration + 4 raw getters", t.elapsed());
        let mut it = rows.update(&snapshot).expect("rows");
        let t = Instant::now();
        while let Some(row) = it.next() {
            for raw in row.cells_raw().expect("raw") {
                sum += raw.codepoint().unwrap_or(0) as u64;
                sum += raw.wide().map_or(0, |w| w as u64);
                sum += raw.content_tag().map_or(0, |w| w as u64);
                sum += raw.style_id().is_ok() as u64;
            }
        }
        add("cells_raw bulk + 4 raw getters", t.elapsed());
        let mut it = rows.update(&snapshot).expect("rows");
        let t = Instant::now();
        while let Some(row) = it.next() {
            let mut c = cells.update(row).expect("cells");
            while let Some(cell) = c.next() {
                sum += cell.style().map_or(0, |s| s.bold as u64);
            }
        }
        add("iteration + style() every cell", t.elapsed());
        let mut it = rows.update(&snapshot).expect("rows");
        let t = Instant::now();
        while let Some(row) = it.next() {
            let mut c = cells.update(row).expect("cells");
            while let Some(cell) = c.next() {
                sum += cell.fg_color().ok().flatten().map_or(0, |c| c.r as u64);
                sum += cell.bg_color().ok().flatten().map_or(0, |c| c.r as u64);
            }
        }
        add("iteration + fg_color + bg_color every cell", t.elapsed());
        let mut it = rows.update(&snapshot).expect("rows");
        let t = Instant::now();
        while let Some(row) = it.next() {
            let mut c = cells.update(row).expect("cells");
            while let Some(cell) = c.next() {
                let _ = cell.graphemes_utf8(&mut text);
                sum += text.len() as u64;
            }
        }
        add("iteration + graphemes_utf8 every cell", t.elapsed());
        let mut it = rows.update(&snapshot).expect("rows");
        let colors = snapshot.colors().expect("colors");
        let t = Instant::now();
        while let Some(row) = it.next() {
            let mut c = cells.update(row).expect("cells");
            let mut cached: Option<(libghostty_vt::style::Id, libghostty_vt::style::Style)> = None;
            while let Some(cell) = c.next() {
                let raw = cell.raw_cell().expect("raw");
                let wide = raw.wide().map_or(0, |w| w as u64);
                let tag = raw.content_tag().map_or(0, |w| w as u64);
                let id = raw.style_id().ok();
                let cp = raw.codepoint().unwrap_or(0) as u64;
                let s = match (cached, id) {
                    (Some((cid, s)), Some(id)) if cid == id => s,
                    _ => {
                        let s = cell.style().unwrap_or_default();
                        let f = cell.fg_color().ok().flatten().unwrap_or(colors.foreground);
                        let b = cell.bg_color().ok().flatten();
                        sum += f.r as u64 + b.map_or(0, |b| b.g as u64);
                        cached = id.map(|id| (id, s));
                        s
                    }
                };
                sum += wide + tag + cp + s.bold as u64;
            }
        }
        add("B: build_row's FFI calls + style cache only", t.elapsed());
        let _ = snapshot.clean();
        black_box(sum);
    }
    println!("200x60 full-damage styled screen, {ITERS} iterations; per frame and per cell:");
    for (name, d) in times {
        let per_frame = d.as_secs_f64() * 1e3 / ITERS as f64;
        println!("{name:<46} {per_frame:>6.3} ms/frame  {:>6.1} ns/cell", per_frame * 1e6 / cell_count as f64);
    }
}
