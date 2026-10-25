//! Frame timing: draw-list build (in render) and paint (in the canvas).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub struct Stats {
    pub build: Vec<Duration>,
    /// (paint only, render start to end of paint)
    pub paint: Rc<RefCell<Vec<(Duration, Duration)>>>,
}

#[derive(Clone)]
pub struct PaintSink(Rc<RefCell<Vec<(Duration, Duration)>>>);

impl PaintSink {
    pub fn record(&self, paint: Duration, total: Duration, feed_bytes: u64) {
        self.0.borrow_mut().push((paint, total));
        if std::env::var_os("SPIKE_STATS").is_some() {
            let at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_micros());
            eprintln!(
                "SPIKE_FRAME unix_us={at} feed_bytes={feed_bytes} paint_us={} total_us={}",
                paint.as_micros(), total.as_micros(),
            );
        }
    }
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let i = ((v.len() - 1) as f64 * p).round() as usize;
    v[i]
}

fn line(name: &str, mut v: Vec<f64>) -> String {
    format!(
        "{name:<34} n={:<5} p50={:>6.2} ms  p95={:>6.2} ms  p99={:>6.2} ms  max={:>6.2} ms",
        v.len(),
        pct(&mut v, 0.5),
        pct(&mut v, 0.95),
        pct(&mut v, 0.99),
        pct(&mut v, 1.0),
    )
}

impl Stats {
    pub fn paint_sink(&self) -> PaintSink {
        PaintSink(self.paint.clone())
    }

    pub fn reset(&mut self) {
        self.build.clear();
        self.paint.borrow_mut().clear();
    }

    pub fn summary(&self) -> String {
        let ms = |d: &Duration| d.as_secs_f64() * 1e3;
        let paint = self.paint.borrow();
        [
            line("build draw list (render)", self.build.iter().map(ms).collect()),
            line("shape + paint (canvas)", paint.iter().map(|p| ms(&p.0)).collect()),
            line("render start -> paint end", paint.iter().map(|p| ms(&p.1)).collect()),
            glyph_totals(),
        ]
        .join("\n")
    }
}

thread_local! {
    static GLYPH: std::cell::Cell<(Duration, Duration, u64)> = const { std::cell::Cell::new((Duration::ZERO, Duration::ZERO, 0)) };
}

pub fn add_lookup(d: Duration) {
    GLYPH.with(|g| {
        let (l, p, n) = g.get();
        g.set((l + d, p, n));
    });
}

pub fn add_glyph(d: Duration) {
    GLYPH.with(|g| {
        let (l, p, n) = g.get();
        g.set((l, p + d, n + 1));
    });
}

pub fn glyph_totals() -> String {
    GLYPH.with(|g| {
        let (l, p, n) = g.get();
        format!(
            "glyph path totals: {n} glyphs, lookup {:.0} ns/glyph, paint_glyph {:.0} ns/glyph",
            l.as_nanos() as f64 / n.max(1) as f64,
            p.as_nanos() as f64 / n.max(1) as f64
        )
    })
}
