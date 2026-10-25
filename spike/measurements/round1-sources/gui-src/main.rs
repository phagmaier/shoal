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
//!   ZED_MEASUREMENTS=1     GPUI prints its own "frame duration" per frame

mod bench;
mod draw;
mod keys;
mod pty;
mod stats;
mod theme;
mod view;

use std::borrow::Cow;

use futures::StreamExt;
use gpui::{
    App, AppContext, Bounds, TitlebarOptions, WindowBackgroundAppearance, WindowBounds,
    WindowDecorations, WindowOptions, px, size,
};
use gpui_platform::application;

use view::TermView;

static SYMBOLS_FONT: &[u8] = include_bytes!("../../assets/SymbolsNerdFontMono-Regular.ttf");

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
            // Feed everything that has arrived, then let one frame happen.
            while let Some(mut batch) = rx.next().await {
                while batch.len() < 1 << 20 {
                    match rx.try_recv() {
                        Ok(more) => batch.extend_from_slice(&more),
                        Err(_) => break,
                    }
                }
                if weak
                    .update(cx, |v, cx| {
                        v.feed(&batch);
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
            // The shell exited.
            let _ = weak.update(cx, |v, _| {
                if std::env::var_os("SPIKE_STATS").is_some() {
                    eprintln!("PTY bytes consumed: {}", v.feed_bytes);
                    eprintln!("{}", v.stats.summary());
                }
            });
            cx.update(|cx| cx.quit());
        })
        .detach();
        cx.activate(true);
    });
}
