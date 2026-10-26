//! `shoal-gui`, the window (M1 spec §7): connects to `shoald`, shows one
//! session and mirrors the workspace as the daemon changes it. `shoal` and
//! `shoal attach` start it detached.

mod actions;
mod chrome;
mod drag;
mod draw;
mod field;
mod fonts;
mod glyphs;
mod input;
mod keys;
mod metrics;
mod mirror;
mod panes;
mod rename;
mod retention;
mod selection;
mod session;
mod shapes;
mod sizing;
mod switcher;
mod terminal;
mod view;

use std::{ffi::OsString, io, process::ExitCode, sync::mpsc::Receiver};

use futures::{StreamExt as _, channel::mpsc};
use gpui::{
    App, AppContext as _, Bounds, TitlebarOptions, WindowBackgroundAppearance, WindowBounds,
    WindowDecorations, WindowOptions, px, size,
};
use shoal_client::{Event, lifecycle::SystemdOrSpawn, model::SessionId};
use shoal_config::{Config, ThemeSource, XdgPaths};
use tracing_subscriber::filter::LevelFilter;

use session::{Choice, Opened};
use view::ShoalWindow;

const USAGE: &str = "usage: shoal-gui [--session <id>]";
const APP_ID: &str = "dev.shoal.Shoal";

fn main() -> ExitCode {
    init_logging();
    let choice = match parse(std::env::args_os().skip(1)) {
        Ok(choice) => choice,
        Err(message) => {
            eprintln!("shoal-gui: {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(choice) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            tracing::error!("{message}");
            eprintln!("shoal-gui: {message}");
            ExitCode::FAILURE
        }
    }
}

/// `--session <id>` opens that session; nothing means smart open in the
/// current folder.
fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Choice, String> {
    let mut args = args.into_iter();
    let choice = match args.next() {
        None => Choice::Smart {
            cwd: std::env::current_dir().ok(),
        },
        Some(flag) if flag == "--session" => {
            let id = args.next().ok_or("--session needs a session id")?;
            let id = id.to_str().ok_or("arguments must be UTF-8")?;
            Choice::Session(
                id.parse::<SessionId>()
                    .map_err(|_| format!("{id:?} isn't a session id"))?,
            )
        }
        Some(other) => return Err(format!("unknown argument {other:?}")),
    };
    match args.next() {
        Some(extra) => Err(format!("unexpected argument {extra:?}")),
        None => Ok(choice),
    }
}

/// Logs go to stderr, at the level `SHOAL_LOG` names (default `info`).
fn init_logging() {
    let level = std::env::var("SHOAL_LOG")
        .ok()
        .and_then(|level| level.parse::<LevelFilter>().ok())
        .unwrap_or(LevelFilter::INFO);
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(io::stderr)
        .init();
}

fn run(choice: Choice) -> Result<(), String> {
    let paths = XdgPaths::from_env().map_err(|e| e.to_string())?;
    let config = match Config::load(&paths.config_file()) {
        Ok(loaded) => loaded.config,
        Err(error) => {
            // T46 shows this as a banner; the window still opens.
            tracing::warn!(%error, "config is invalid; using defaults");
            Config::default()
        }
    };
    let theme = ThemeSource::from_config(&config.theme, &paths).load();
    for warning in &theme.warnings {
        tracing::warn!(%warning, "theme");
    }
    let palette = theme.palette(&config);
    let socket = paths.socket_file().map_err(|e| e.to_string())?;
    let env = std::env::vars_os()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)));
    let opened = session::open(session::options(socket, env), SystemdOrSpawn::new(), choice)
        .map_err(|error| error.to_string())?;
    for event in &opened.early {
        tracing::info!(?event, "before the first workspace");
    }
    tracing::info!(session = %opened.session, daemon = opened.hello.pid, "connected");
    gpui_platform::application().run(move |cx: &mut App| open_window(opened, config, palette, cx));
    Ok(())
}

fn open_window(opened: Opened, config: Config, palette: shoal_config::Palette, cx: &mut App) {
    fonts::load(cx);
    let Opened {
        client,
        session,
        workspace,
        events,
        ..
    } = opened;
    let bounds = Bounds::centered(None, size(px(900.0), px(600.0)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("Shoal".into()),
            ..Default::default()
        }),
        window_background: WindowBackgroundAppearance::Transparent,
        app_id: Some(APP_ID.into()),
        window_decorations: Some(WindowDecorations::Server),
        ..Default::default()
    };
    let window = match cx.open_window(options, |window, cx| {
        cx.new(|cx| ShoalWindow::new(client, session, workspace, config, palette, window, cx))
    }) {
        Ok(window) => window,
        Err(error) => {
            tracing::error!(%error, "can't open a window");
            cx.quit();
            return;
        }
    };
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
    let mut events = forward(events);
    cx.spawn(async move |cx| {
        while let Some(event) = events.next().await {
            let open = window.update(cx, |view, window, cx| {
                let shown = view.on_event(event, window, cx);
                if !shown {
                    tracing::info!("the session closed; closing the window");
                    window.remove_window();
                }
                shown
            });
            if !matches!(open, Ok(true)) {
                break;
            }
        }
    })
    .detach();
    cx.activate(true);
}

/// The client's blocking event channel, as a stream the window's
/// foreground task awaits. The forwarding thread ends with the client.
fn forward(events: Receiver<Event>) -> mpsc::UnboundedReceiver<Event> {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("shoal-events".into())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                if sender.unbounded_send(event).is_err() {
                    break;
                }
            }
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "can't start the event thread");
    }
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Choice, String> {
        super::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn arguments_choose_the_session() {
        assert_eq!(
            parse(&["--session", "s3"]),
            Ok(Choice::Session("s3".parse().unwrap()))
        );
        assert!(matches!(parse(&[]), Ok(Choice::Smart { cwd: Some(_) })));
        assert_eq!(
            parse(&["--session"]),
            Err("--session needs a session id".into())
        );
        assert_eq!(
            parse(&["--session", "main"]),
            Err("\"main\" isn't a session id".into())
        );
        assert_eq!(
            parse(&["--session", "s3", "x"]),
            Err("unexpected argument \"x\"".into())
        );
        assert_eq!(parse(&["main"]), Err("unknown argument \"main\"".into()));
    }
}
