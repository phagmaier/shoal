use std::{
    error::Error,
    path::Path,
    sync::mpsc,
    thread::{self, JoinHandle},
};

use shoal_config::{
    Config, ConfigUpdate, ConfigWatcher, LoadedTheme, ThemeSource, ThemeUpdate, ThemeWatcher,
    WatchError, XdgPaths,
};
use shoal_proto::{NoticeLevel, Notification};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::Settings;

/// Current last-good settings and all currently outstanding config/theme notices.
#[derive(Debug)]
pub struct Update {
    pub settings: Settings,
    pub notices: Vec<Notification>,
}

/// Owns the native watches. Dropping this handle requests their shutdown without
/// joining workers on the actor: the coordinator stops and joins them itself.
pub struct Watchers {
    control: mpsc::Sender<Event>,
    _worker: JoinHandle<()>,
}

impl Drop for Watchers {
    fn drop(&mut self) {
        let _ = self.control.send(Event::Stop);
    }
}

enum Event {
    Config(Box<ConfigUpdate>),
    Theme {
        generation: u64,
        update: ThemeUpdate,
    },
    Stop,
}

/// Install native watches before their initial reads when possible. A failed
/// native watch leaves that source unwatched, with a diagnostic and a direct
/// initial load. Invalid subsequent config edits retain the last good config.
/// No worker wakes while idle.
pub fn watch(
    paths: &XdgPaths,
) -> Result<(Settings, UnboundedReceiver<Update>, Watchers), Box<dyn Error>> {
    let (control, events) = mpsc::channel();
    let config_events = control.clone();
    let config_path = paths.config_file();
    let (config_watcher, config, watch_notice) =
        match ConfigWatcher::new(&config_path, move |update| {
            let _ = config_events.send(Event::Config(Box::new(update)));
        }) {
            Ok((watcher, config)) => (Some(watcher), config, None),
            // This also covers failure to create the native watcher's worker.
            // Do not report a live coordinator when a required thread cannot start.
            Err(error @ WatchError::Io(_)) => return Err(error.into()),
            Err(error) => {
                let (loaded, load_error) = match Config::load(&config_path) {
                    Ok(loaded) => (loaded, None),
                    Err(error) => (Default::default(), Some(error.into())),
                };
                let config = ConfigUpdate {
                    loaded,
                    error: load_error,
                };
                let watch_notice = notice(
                    NoticeLevel::Error,
                    format!(
                        "{}: config watch unavailable: {error}; changes will not be watched",
                        config_path.display()
                    ),
                    None,
                );
                (None, config, Some(watch_notice))
            }
        };
    let source = ThemeSource::from_config(&config.loaded.config.theme, paths);
    let (theme_watcher, theme) = match theme_watcher(source.clone(), 0, control.clone()) {
        Ok((watcher, theme)) => (Some(watcher), theme),
        Err(error @ WatchError::Io(_)) => return Err(error.into()),
        Err(error) => (
            None,
            ThemeUpdate {
                theme: source.load(),
                error: Some(error),
            },
        ),
    };
    let mut config_notices = config_notices(&config, &config_path);
    config_notices.extend(watch_notice);
    let mut theme_notices = theme_notices(&theme, &source);
    if theme_watcher.is_none() {
        theme_notices.push(unwatched_theme_notice(&source));
    }
    let settings = Settings {
        palette: theme.theme.palette(&config.loaded.config),
        config: config.loaded.config.clone(),
    };
    let (output, updates) = unbounded_channel();
    let state = State {
        paths: paths.clone(),
        settings: settings.clone(),
        config_notices,
        theme_notices,
        source,
        theme: theme.theme,
        generation: 0,
        theme_watcher,
        _config_watcher: config_watcher,
        control: control.clone(),
        output,
    };
    if !state.config_notices.is_empty() || !state.theme_notices.is_empty() {
        state.publish();
    }
    let worker = thread::Builder::new()
        .name("shoal-settings".into())
        .spawn(move || state.run(events))?;
    Ok((
        settings,
        updates,
        Watchers {
            control,
            _worker: worker,
        },
    ))
}

struct State {
    paths: XdgPaths,
    settings: Settings,
    config_notices: Vec<Notification>,
    theme_notices: Vec<Notification>,
    source: ThemeSource,
    theme: LoadedTheme,
    generation: u64,
    theme_watcher: Option<ThemeWatcher>,
    _config_watcher: Option<ConfigWatcher>,
    control: mpsc::Sender<Event>,
    output: UnboundedSender<Update>,
}

impl State {
    fn run(mut self, events: mpsc::Receiver<Event>) {
        while let Ok(event) = events.recv() {
            match event {
                Event::Stop => return,
                Event::Config(update) => self.config(*update),
                Event::Theme { generation, update } => {
                    // A replaced watch can already have queued an update. Never
                    // let it overwrite the newly selected source's palette.
                    if generation != self.generation {
                        continue;
                    }
                    self.theme_notices = theme_notices(&update, &self.source);
                    self.theme = update.theme;
                }
            }
            self.settings.palette = self.theme.palette(&self.settings.config);
            if !self.publish() {
                return;
            }
        }
    }

    fn config(&mut self, update: ConfigUpdate) {
        self.config_notices = config_notices(&update, &self.paths.config_file());
        self.settings.config = update.loaded.config;
        let source = ThemeSource::from_config(&self.settings.config.theme, &self.paths);
        if source == self.source && self.theme_watcher.is_some() {
            return;
        }
        self.generation += 1;
        self.source = source;
        // Native watcher setup, reads and the old watch's blocking Drop all
        // happen here, never on the actor or inside a watcher callback.
        match theme_watcher(self.source.clone(), self.generation, self.control.clone()) {
            Ok((watcher, update)) => {
                self.theme_watcher = Some(watcher);
                self.theme_notices = theme_notices(&update, &self.source);
                self.theme = update.theme;
            }
            Err(error) => {
                self.theme_watcher = None;
                let update = ThemeUpdate {
                    theme: self.source.load(),
                    error: Some(error),
                };
                self.theme_notices = theme_notices(&update, &self.source);
                self.theme_notices
                    .push(unwatched_theme_notice(&self.source));
                self.theme = update.theme;
            }
        }
    }

    fn publish(&self) -> bool {
        self.output
            .send(Update {
                settings: self.settings.clone(),
                notices: self
                    .config_notices
                    .iter()
                    .chain(&self.theme_notices)
                    .cloned()
                    .collect(),
            })
            .is_ok()
    }
}

fn theme_watcher(
    source: ThemeSource,
    generation: u64,
    events: mpsc::Sender<Event>,
) -> Result<(ThemeWatcher, ThemeUpdate), WatchError> {
    ThemeWatcher::new(source, move |update| {
        // Unbounded sends do not wait for the coordinator. In particular,
        // joining a replaced watcher cannot deadlock with its callback.
        let _ = events.send(Event::Theme { generation, update });
    })
}

fn config_notices(update: &ConfigUpdate, path: &Path) -> Vec<Notification> {
    let mut notices = Vec::new();
    if let Some(error) = &update.error {
        let message = match error {
            WatchError::Config(_) => error.to_string(),
            _ => format!("{}: {error}", path.display()),
        };
        notices.push(notice(NoticeLevel::Error, message, None));
    }
    notices.extend(update.loaded.warnings.iter().map(|warning| {
        notice(
            NoticeLevel::Warn,
            warning.to_string(),
            Some(warning.key.clone()),
        )
    }));
    notices
}

fn theme_notices(update: &ThemeUpdate, source: &ThemeSource) -> Vec<Notification> {
    let mut notices = Vec::new();
    if let Some(error) = &update.error {
        notices.push(notice(
            NoticeLevel::Warn,
            format!("{}: {error}", source_name(source)),
            Some("theme.source".into()),
        ));
    }
    notices.extend(update.theme.warnings.iter().map(|warning| {
        let key = warning
            .reason
            .strip_suffix(": color must be #rrggbb")
            .map(|key| format!("theme.colors.{key}"))
            .unwrap_or_else(|| "theme.source".into());
        notice(NoticeLevel::Warn, warning.to_string(), Some(key))
    }));
    if matches!(source, ThemeSource::Omarchy(_)) && update.theme.folder.is_none() {
        notices.push(notice(
            NoticeLevel::Warn,
            format!(
                "{}: theme folder not found; using built-in theme colors",
                source_name(source)
            ),
            Some("theme.source".into()),
        ));
    }
    notices
}

fn unwatched_theme_notice(source: &ThemeSource) -> Notification {
    notice(
        NoticeLevel::Warn,
        format!(
            "{}: theme watch unavailable; changes will not be watched until config is reloaded",
            source_name(source)
        ),
        Some("theme.source".into()),
    )
}

fn source_name(source: &ThemeSource) -> String {
    match source {
        ThemeSource::File(path) => path.display().to_string(),
        ThemeSource::Omarchy(currents) => format!(
            "{} or {}",
            currents[0].join("theme").display(),
            currents[1].join("theme").display()
        ),
    }
}

fn notice(level: NoticeLevel, message: String, key: Option<String>) -> Notification {
    match level {
        NoticeLevel::Error => tracing::error!(%message, "settings"),
        NoticeLevel::Warn => tracing::warn!(%message, "settings"),
        NoticeLevel::Info => tracing::info!(%message, "settings"),
    }
    Notification::DaemonNotice {
        level,
        message,
        key,
    }
}
