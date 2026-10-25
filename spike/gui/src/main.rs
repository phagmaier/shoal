//! T00 spike: a GPUI window running bash through a PTY into libghostty-vt.
//! Throwaway code on the spike/t00 branch.
//!
//! Environment:
//!   SPIKE_CMD="..."        run this under `bash -c` instead of `bash -l`
//!   SPIKE_GRID=200x60      fixed grid at the requested font size (never shrunk)
//!   SPIKE_FONT_PX=14       font size in logical pixels (default 14)
//!   SPIKE_WINDOW=1800x1400 requested window size in logical pixels (default 900x600)
//!   SPIKE_BENCH=<frames>   redraw continuously, print frame times, quit
//!   SPIKE_BENCH_MODE=styled|plain|static|typing|scroll
//!   SPIKE_STATS=1          print frame time summary when the shell exits
//!   SPIKE_RENDER=shape     shape text runs each frame (default: place cached glyphs)
//!   SPIKE_NO_LAYER=1       paint without a layer (slow; for comparison)
//!   SPIKE_VERIFY_ROWS=1    compare cached rows with a full rebuild every frame
//!   SPIKE_FLOOD_FRAME_MS=N skip a frame request if one was made within N ms
//!                          and the feed slice used its whole budget (default 0).
//!                          Measured: slices usually end early, when the channel
//!                          is momentarily empty, so this rarely applies.
//!   ZED_MEASUREMENTS=1     GPUI prints its own "frame duration" per frame

mod bench;
mod draw;
mod draw_ref;
mod keys;
mod paint;
mod pty;
mod stats;
mod theme;
mod view;

use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui::{
    App, AppContext, Bounds, TitlebarOptions, WindowBackgroundAppearance, WindowBounds,
    WindowDecorations, WindowOptions, px, size,
};
use gpui_platform::application;

use view::TermView;

static SYMBOLS_FONT: &[u8] = include_bytes!("../../assets/SymbolsNerdFontMono-Regular.ttf");

/// Longest stretch of VT parsing before the feed task lets the event loop
/// run. GPUI runs foreground tasks as idle callbacks after pending events,
/// so yielding is what lets a due frame (and input) in during a flood.
const FEED_BUDGET: Duration = Duration::from_millis(4);

/// Return to the executor once, rescheduling immediately.
struct YieldNow(bool);

impl Future for YieldNow {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            return Poll::Ready(());
        }
        self.0 = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

fn main() {
    application().run(|cx: &mut App| {
        if let Err(e) = cx
            .text_system()
            .add_fonts(vec![Cow::Borrowed(SYMBOLS_FONT)])
        {
            eprintln!("could not load the symbols font: {e}");
        }
        let (pty, mut rx) = match pty::spawn(80, 24) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("could not start the shell: {e}");
                cx.quit();
                return;
            }
        };
        let (width, height) = std::env::var("SPIKE_WINDOW")
            .ok()
            .and_then(|s| {
                let (w, h) = s.split_once('x')?;
                Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))
            })
            .unwrap_or((900.0, 600.0));
        let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Shoal spike".into()),
                        ..Default::default()
                    }),
                    window_background: WindowBackgroundAppearance::Transparent,
                    app_id: Some("dev.shoal.Spike".into()),
                    window_decorations: Some(WindowDecorations::Server),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| TermView::new(pty, window, cx)),
            )
            .expect("open window");
        let view = window.update(cx, |_, _, cx| cx.entity()).expect("view");
        let weak = view.downgrade();
        cx.spawn(async move |cx| {
            // Feed what has arrived for at most FEED_BUDGET, request a frame,
            // then yield so the event loop can draw it before parsing more.
            // Ready chunks are merged (up to MERGE bytes) before each write,
            // as round 1 did; merging did not measurably change parse time.
            const MERGE: usize = 1 << 20;
            let mut batch = Vec::with_capacity(MERGE);
            let flood_gap = Duration::from_millis(
                std::env::var("SPIKE_FLOOD_FRAME_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(0),
            );
            let mut last_notify: Option<Instant> = None;
            while let Some(first) = rx.next().await {
                let start = Instant::now();
                if weak
                    .update(cx, |v, cx| {
                        v.feed_slices += 1;
                        batch.clear();
                        batch.extend_from_slice(&first);
                        let mut flooding = false;
                        loop {
                            if start.elapsed() >= FEED_BUDGET {
                                flooding = true;
                                break;
                            }
                            match rx.try_recv().ok() {
                                Some(more) if batch.len() + more.len() <= MERGE => {
                                    batch.extend_from_slice(&more)
                                }
                                Some(more) => {
                                    v.feed(&batch);
                                    batch.clear();
                                    batch.extend_from_slice(&more);
                                }
                                None => break,
                            }
                        }
                        v.feed(&batch);
                        // A slice that drained the channel always gets a frame,
                        // so the final output is never left undrawn.
                        let recent = last_notify.is_some_and(|t| t.elapsed() < flood_gap);
                        if !(flooding && recent) {
                            last_notify = Some(Instant::now());
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    return;
                }
                YieldNow(false).await;
            }
            // The shell exited.
            let _ = weak.update(cx, |v, _| {
                if std::env::var_os("SPIKE_STATS").is_some() {
                    eprintln!("PTY bytes consumed: {}", v.feed_bytes);
                    eprintln!(
                        "feed: {} chunks, {} slices, {:.1} ms in vt_write",
                        v.feed_chunks,
                        v.feed_slices,
                        v.feed_parse.as_secs_f64() * 1e3
                    );
                    eprintln!("{}", v.stats.summary());
                }
                if let Some(s) = v.verify_summary() {
                    eprintln!("{s}");
                }
            });
            cx.update(|cx| cx.quit());
        })
        .detach();
        cx.activate(true);
    });
}
