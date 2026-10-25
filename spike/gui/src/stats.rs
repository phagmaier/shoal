//! Frame timing: draw-list build and glyph resolution (in render), and paint
//! (in the canvas).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub struct Stats {
    pub build: Vec<Duration>,
    /// The `RenderState::update` part of `build`.
    pub update: Vec<Duration>,
    /// Per-row glyph resolution / shaping for rows the painter re-did.
    pub prepare: Vec<Duration>,
    /// (rows rebuilt by the builder, rows re-resolved by the painter)
    pub rows: Vec<(u16, u16)>,
    /// Benchmark output generation and VT parsing, outside every frame timer.
    pub fixture: Vec<Duration>,
    /// (paint only, render start to end of paint, glyphs)
    pub paint: Rc<RefCell<Vec<(Duration, Duration, usize)>>>,
}

#[derive(Clone)]
pub struct PaintSink(Rc<RefCell<Vec<(Duration, Duration, usize)>>>);

impl PaintSink {
    pub fn record(&self, paint: Duration, total: Duration, feed_bytes: u64, glyphs: usize) {
        self.0.borrow_mut().push((paint, total, glyphs));
        if std::env::var_os("SPIKE_STATS").is_some() {
            let at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_micros());
            eprintln!(
                "SPIKE_FRAME unix_us={at} feed_bytes={feed_bytes} paint_us={} total_us={}",
                paint.as_micros(),
                total.as_micros(),
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

fn line(name: &str, unit: &str, mut v: Vec<f64>) -> String {
    format!(
        "{name:<34} n={:<5} p50={:>6.2} {unit}  p95={:>6.2} {unit}  p99={:>6.2} {unit}  max={:>6.2} {unit}",
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

    pub fn push_build(&mut self, build: Duration, update: Duration, prepare: Duration, rebuilt: u16, resolved: u16) {
        self.build.push(build);
        self.update.push(update);
        self.prepare.push(prepare);
        self.rows.push((rebuilt, resolved));
    }

    pub fn reset(&mut self) {
        self.build.clear();
        self.update.clear();
        self.prepare.clear();
        self.rows.clear();
        self.fixture.clear();
        self.paint.borrow_mut().clear();
    }

    pub fn summary(&self) -> String {
        let ms = |d: &Duration| d.as_secs_f64() * 1e3;
        let paint = self.paint.borrow();
        let mut out = vec![
            line("build draw list (render)", "ms", self.build.iter().map(ms).collect()),
            line("  of which render-state update", "ms", self.update.iter().map(ms).collect()),
            line("resolve rows (render)", "ms", self.prepare.iter().map(ms).collect()),
            line("paint (canvas)", "ms", paint.iter().map(|p| ms(&p.0)).collect()),
            line("render start -> paint end", "ms", paint.iter().map(|p| ms(&p.1)).collect()),
            line("rows rebuilt / frame", "  ", self.rows.iter().map(|r| r.0 as f64).collect()),
            line("rows re-resolved / frame", "  ", self.rows.iter().map(|r| r.1 as f64).collect()),
            line("glyphs painted / frame", "  ", paint.iter().map(|p| p.2 as f64).collect()),
        ];
        if !self.fixture.is_empty() {
            out.push(line("bench output + VT parse (untimed)", "ms", self.fixture.iter().map(ms).collect()));
        }
        out.join("\n")
    }
}
