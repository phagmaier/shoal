//! `SPIKE_BENCH=<frames>`: redraw continuously and report frame times.
//! `SPIKE_BENCH_MODE=styled` (default) writes a full screen of new, heavily
//! styled text before every frame, so every row is rebuilt and reshaped;
//! `plain` does the same with one color per line; `static` changes nothing.
//! `typing` changes one cell per frame without scrolling; `scroll` appends
//! one log line per frame. Both start with the same styled screen.
//! The view calls `before_frame` from a next-frame callback, so writing and
//! parsing this output is outside every frame timer.

use libghostty_vt::terminal::Terminal;

const WARMUP: usize = 30;

pub struct Bench {
    frames: usize,
    mode: String,
    n: usize,
    /// A next-frame callback is registered.
    pub pending: bool,
}

impl Bench {
    pub fn from_env() -> Option<Self> {
        let frames = std::env::var("SPIKE_BENCH").ok()?.parse().ok()?;
        Some(Self {
            frames,
            mode: std::env::var("SPIKE_BENCH_MODE").unwrap_or_else(|_| "styled".into()),
            n: 0,
            pending: false,
        })
    }

    pub fn before_frame(&mut self, term: &mut Terminal<'static, 'static>) {
        self.n += 1;
        let cols = term.cols().unwrap_or(80) as usize;
        let rows = term.rows().unwrap_or(24) as usize;
        let mut out = String::new();
        match self.mode.as_str() {
            // Fill the screen once (after the grid is sized), then change nothing.
            "static" if self.n != 2 => {}
            "typing" if self.n > 2 => {
                let col = 1 + (self.n - 3) % cols.saturating_sub(1).max(1);
                let ch = (b'a' + ((self.n - 3) % 26) as u8) as char;
                out.push_str(&format!("\x1b[{rows};{col}H{ch}"));
            }
            "scroll" if self.n > 2 => {
                out.push_str(&format!(
                    "\r\n\x1b[32mINFO\x1b[0m frame={:06} request completed in 2 ms",
                    self.n,
                ));
            }
            "plain" if self.mode == "plain" => {
                for r in 0..rows {
                    let word = format!("{:06} line {r:02} ", self.n);
                    let text: String = word.chars().cycle().take(cols).collect();
                    out.push_str(&format!("\r\n\x1b[3{}m{text}\x1b[0m", r % 8));
                }
            }
            _ => {
                let words = ["src/", "main.rs", "error", "warn", "Cargo", "build", "→", "漢字", "", "ok"];
                for r in 0..rows {
                    out.push_str("\r\n");
                    let mut col = 0;
                    let mut i = self.n + r;
                    while col + 10 < cols {
                        let w = words[i % words.len()];
                        let sgr = match i % 6 {
                            0 => "1;31".to_string(),
                            1 => "32".to_string(),
                            2 => format!("38;5;{}", (i * 7) % 256),
                            3 => format!("38;2;{};{};200", (i * 13) % 256, (i * 29) % 256),
                            4 => "3;35".to_string(),
                            _ => "4;36".to_string(),
                        };
                        let cell = format!("{w:<9}");
                        out.push_str(&format!("\x1b[{sgr}m{cell}\x1b[0m "));
                        col += 10;
                        i += 1;
                    }
                }
            }
        }
        term.vt_write(out.as_bytes());
    }

    pub fn warmup_done(&self) -> bool {
        self.n == WARMUP
    }

    pub fn finished(&self) -> bool {
        self.n >= self.frames + WARMUP
    }
}
