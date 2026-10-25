//! The terminal view: one libghostty terminal fed by the PTY, drawn as a grid.

use std::ops::Range;
use std::time::Instant;

use gpui::{
    App, Bounds, Context, ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable,
    Font, FontFallbacks, KeyDownEvent, Pixels, Render, ScrollDelta, ScrollWheelEvent, SharedString,
    UTF16Selection, Window, canvas, div, point, prelude::*, px, size,
};
use libghostty_vt::key;
use libghostty_vt::style::Palette;
use libghostty_vt::terminal::{ScrollViewport, Terminal};

use crate::draw::Builder;
use crate::keys;
use crate::paint::{self, Metrics, PAD, Painter, rgba};
use crate::pty::Pty;
use crate::stats::Stats;
use crate::theme::Theme;

pub struct TermView {
    pub term: Terminal<'static, 'static>,
    pub pty: Pty,
    builder: Builder,
    key_encoder: key::Encoder<'static>,
    key_event: key::Event<'static>,
    focus: FocusHandle,
    theme: Theme,
    opacity: f32,
    font: Font,
    metrics: Option<Metrics>,
    metrics_for: Option<(Size, Option<(u16, u16)>)>,
    /// `SPIKE_GRID=200x60`: fixed grid, without changing the font size.
    fixed_grid: Option<(u16, u16)>,
    base_font_px: f32,
    preedit: String,
    painter: Painter,
    /// Fractional scroll lines not yet applied (touchpads send small deltas).
    scroll_acc: f32,
    pub stats: Stats,
    pub feed_bytes: u64,
    /// (chunks, slices, time in vt_write) for the streaming summary.
    pub feed_chunks: u64,
    pub feed_slices: u64,
    pub feed_parse: std::time::Duration,
    pub bench: Option<crate::bench::Bench>,
}

type Size = gpui::Size<Pixels>;

impl TermView {
    pub fn new(pty: Pty, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let theme = crate::theme::load();
        let mut term = Terminal::new(80, 24).expect("terminal");
        term.set_scrollback_max_lines(Some(10_000))
            .and_then(|t| t.set_scrollback_max_bytes(None))
            .expect("scrollback");
        let _ = term.set_default_fg_color(Some(theme.foreground));
        let _ = term.set_default_bg_color(Some(theme.background));
        let _ = term.set_default_cursor_color(Some(theme.accent));
        if let Ok(mut palette) = term.default_color_palette() {
            palette.0[..16].copy_from_slice(&theme.ansi);
            let _ = term.set_default_color_palette(Some(Palette(palette.0)));
        }
        let replies = pty.writer.clone();
        term.on_pty_write(move |_, data| {
            if let Ok(mut w) = replies.lock() {
                let _ = std::io::Write::write_all(&mut **w, data);
            }
        })
        .expect("pty write handler");

        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let fixed_grid = std::env::var("SPIKE_GRID").ok().and_then(|g| {
            let (c, r) = g.split_once('x')?;
            Some((c.parse().ok()?, r.parse().ok()?))
        });
        let base_font_px = std::env::var("SPIKE_FONT_PX")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(14.0);
        Self {
            term,
            pty,
            builder: Builder::new(),
            key_encoder: key::Encoder::new().expect("key encoder"),
            key_event: key::Event::new().expect("key event"),
            focus,
            theme,
            opacity: 0.88,
            font: Font {
                family: SharedString::from(
                    std::env::var("SPIKE_FONT").unwrap_or_else(|_| "JetBrains Mono".into()),
                ),
                fallbacks: Some(FontFallbacks::from_fonts(vec![
                    "Symbols Nerd Font Mono".into(),
                    "Noto Color Emoji".into(),
                ])),
                ..Font::default()
            },
            metrics: None,
            metrics_for: None,
            fixed_grid,
            base_font_px,
            preedit: String::new(),
            painter: Painter::new(std::env::var("SPIKE_RENDER").map_or(true, |v| v != "shape")),
            scroll_acc: 0.0,
            stats: Stats::default(),
            feed_bytes: 0,
            feed_chunks: 0,
            feed_slices: 0,
            feed_parse: std::time::Duration::ZERO,
            bench: crate::bench::Bench::from_env(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        if self.feed_bytes == 0 {
            // Streaming statistics exclude empty startup/configure frames.
            self.stats.reset();
        }
        self.feed_bytes += bytes.len() as u64;
        self.feed_chunks += 1;
        let t = Instant::now();
        self.term.vt_write(bytes);
        self.feed_parse += t.elapsed();
    }

    fn metrics_at(&self, font_px: f32, window: &Window) -> Metrics {
        let ts = window.text_system();
        let id = ts.resolve_font(&self.font);
        let font_size = px(font_px);
        let cell_w = ts
            .advance(id, font_size, 'm')
            .map(|s| s.width)
            .unwrap_or(px(font_px * 0.6));
        let scale = window.scale_factor();
        let ascent = ts.ascent(id, font_size);
        let descent = ts.descent(id, font_size).abs();
        // Rows land on whole device pixels.
        let line_h = px(((f32::from(ascent + descent) * 1.15) * scale).round() / scale);
        Metrics {
            font_size,
            cell_w,
            line_h,
            baseline: (line_h - ascent - descent) / 2.0 + ascent,
        }
    }

    /// Work out font metrics and grid size for the current window size,
    /// resizing the terminal and PTY when the grid changes.
    fn layout(&mut self, window: &mut Window) {
        let viewport = window.viewport_size();
        let key = (viewport, self.fixed_grid);
        if self.metrics_for == Some(key) {
            return;
        }
        self.metrics_for = Some(key);
        let avail_w = f32::from(viewport.width) - 2.0 * PAD;
        let avail_h = f32::from(viewport.height) - 2.0 * PAD;
        let base = self.metrics_at(self.base_font_px, window);
        let (metrics, cols, rows) = match self.fixed_grid {
            Some((cols, rows)) => (base, cols, rows),
            None => {
                let cols = (avail_w / f32::from(base.cell_w)).floor().max(2.0) as u16;
                let rows = (avail_h / f32::from(base.line_h)).floor().max(1.0) as u16;
                (base, cols, rows)
            }
        };
        let scale = window.scale_factor();
        if self.term.cols().ok() != Some(cols) || self.term.rows().ok() != Some(rows) {
            let _ = self.term.resize(
                cols,
                rows,
                (f32::from(metrics.cell_w) * scale) as u32,
                (f32::from(metrics.line_h) * scale) as u32,
            );
            self.pty.resize(cols, rows);
        }
        eprintln!(
            "SPIKE_GEOMETRY viewport={:.0}x{:.0} scale={} font_px={:.2} cell_px={:.3}x{:.3} grid={}x{} fits={}",
            f32::from(viewport.width),
            f32::from(viewport.height),
            scale,
            f32::from(metrics.font_size),
            f32::from(metrics.cell_w),
            f32::from(metrics.line_h),
            cols,
            rows,
            f32::from(metrics.cell_w) * cols as f32 <= avail_w
                && f32::from(metrics.line_h) * rows as f32 <= avail_h,
        );
        self.metrics = Some(metrics);
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let m = ks.modifiers;
        let named = keys::named(&ks.key);
        // Plain text (Shift at most) goes through the input handler, which
        // is also how input-method commits arrive.
        if named.is_none() && !m.control && !m.alt && !m.platform && ks.key_char.is_some() {
            return;
        }
        let (k, unshifted) = match named {
            Some(k) => (k, '\0'),
            None => match keys::printable(&ks.key) {
                Some(p) => p,
                None => return,
            },
        };
        let mods = keys::mods(ks);
        let text = if m.control {
            None
        } else {
            ks.key_char.as_deref()
        };
        let mut consumed = key::Mods::empty();
        if text.is_some() && m.shift {
            consumed |= key::Mods::SHIFT;
        }
        self.key_event
            .set_action(key::Action::Press)
            .set_key(k)
            .set_mods(mods)
            .set_consumed_mods(consumed)
            .set_unshifted_codepoint(unshifted)
            .set_utf8(text);
        let mut out = Vec::with_capacity(16);
        if self
            .key_encoder
            .set_options_from_terminal(&self.term)
            .encode_to_vec(&self.key_event, &mut out)
            .is_ok()
            && !out.is_empty()
        {
            self.term.scroll_viewport(ScrollViewport::Bottom);
            self.pty.write(&out);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_scroll(&mut self, ev: &ScrollWheelEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let line_h = self.metrics.as_ref().map_or(px(16.0), |m| m.line_h);
        let lines = match ev.delta {
            ScrollDelta::Lines(p) => -p.y * 3.0,
            ScrollDelta::Pixels(p) => -f32::from(p.y) / f32::from(line_h),
        };
        // Keep the remainder so many small touchpad deltas add up.
        self.scroll_acc += lines;
        let n = self.scroll_acc.trunc();
        self.scroll_acc -= n;
        let n = n as isize;
        if n != 0 {
            self.term.scroll_viewport(ScrollViewport::Delta(n));
            cx.notify();
        }
    }
}

impl Focusable for TermView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TermView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.layout(window);
        if let Some(bench) = self.bench.as_mut().filter(|b| !b.pending) {
            // The benchmark writes its output just before the next frame,
            // outside both renderer timers and GPUI's frame-duration timer.
            bench.pending = true;
            let entity = cx.entity();
            window.on_next_frame(move |_, cx| {
                entity.update(cx, |v, cx| v.bench_step(cx));
            });
        }
        let frame_start = Instant::now();
        let t0 = Instant::now();
        let frame = self.builder.build(&self.term);
        let build = t0.elapsed();

        let metrics = self.metrics.take();
        let m = Metrics {
            font_size: metrics.as_ref().map_or(px(14.0), |m| m.font_size),
            cell_w: metrics.as_ref().map_or(px(8.0), |m| m.cell_w),
            line_h: metrics.as_ref().map_or(px(16.0), |m| m.line_h),
            baseline: metrics.as_ref().map_or(px(12.0), |m| m.baseline),
        };
        self.metrics = metrics;
        let t1 = Instant::now();
        let (rows, resolved) =
            self.painter
                .prepare(&frame.rows, &self.font, &m, self.opacity, window);
        self.stats
            .push_build(build, frame.update, t1.elapsed(), frame.rebuilt, resolved);
        let cursor = frame.cursor;
        let bg = self.theme.background;
        let accent = self.theme.accent;
        let opacity = self.opacity;
        let focused = self.focus.is_focused(window);
        let entity: Entity<Self> = cx.entity();
        let focus = self.focus.clone();
        let paint_times = self.stats.paint_sink();
        let feed_bytes = self.feed_bytes;

        div()
            .size_full()
            .bg(rgba(bg, opacity))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let t = Instant::now();
                        window.handle_input(&focus, ElementInputHandler::new(bounds, entity), cx);
                        // One layer: every primitive shares a draw order, which
                        // skips GPUI's per-primitive bounds-tree insert.
                        let layered = std::env::var_os("SPIKE_NO_LAYER").is_none();
                        let paint = |window: &mut Window, cx: &mut App| {
                            paint::paint(&rows, cursor, bounds, &m, accent, focused, window, cx)
                        };
                        let glyphs = if layered {
                            window.paint_layer(bounds, |window| paint(window, cx))
                        } else {
                            paint(window, cx)
                        };
                        paint_times.record(t.elapsed(), frame_start.elapsed(), feed_bytes, glyphs);
                    },
                )
                .size_full(),
            )
    }
}

impl TermView {
    /// One benchmark step: write the next synthetic output, then redraw.
    fn bench_step(&mut self, cx: &mut Context<Self>) {
        let Some(bench) = self.bench.as_mut() else {
            return;
        };
        bench.pending = false;
        let t = Instant::now();
        bench.before_frame(&mut self.term);
        let fixture = t.elapsed();
        if bench.warmup_done() {
            self.stats.reset();
        }
        if bench.finished() {
            let grid = (self.term.cols().unwrap_or(0), self.term.rows().unwrap_or(0));
            eprintln!("BENCH grid {}x{}\n{}", grid.0, grid.1, self.stats.summary());
            if let Some(v) = self.builder.verify_summary() {
                eprintln!("{v}");
            }
            cx.quit();
            return;
        }
        self.stats.fixture.push(fixture);
        cx.notify();
    }

    pub fn verify_summary(&self) -> Option<String> {
        self.builder.verify_summary()
    }
}

impl EntityInputHandler for TermView {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        (!self.preedit.is_empty()).then(|| 0..self.preedit.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.preedit.clear();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit.clear();
        if std::env::var_os("SPIKE_LOG_INPUT").is_some() {
            eprintln!("input handler text: {} bytes", text.len());
        }
        self.term.scroll_viewport(ScrollViewport::Bottom);
        self.pty.write(text.as_bytes());
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preedit = new_text.to_owned();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // Put the input method's popup at the cursor.
        let m = self.metrics.as_ref()?;
        let x = self.term.cursor_x().ok()? as f32;
        let y = self.term.cursor_y().ok()? as f32;
        Some(Bounds::new(
            element_bounds.origin + point(px(PAD) + m.cell_w * x, px(PAD) + m.line_h * y),
            size(m.cell_w, m.line_h),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
