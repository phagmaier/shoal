//! One config/theme flow per window. Native watch replacement and its joining
//! Drop run on the coordinator, never on the GPUI thread. Nothing wakes idle.
use std::{sync::mpsc, thread};

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use shoal_config::{
    Config, ConfigUpdate, ConfigWatcher, LoadedTheme, Palette, ThemeSource, ThemeUpdate,
    ThemeWatcher, XdgPaths,
};

pub struct Update {
    pub config: Config,
    pub palette: Palette,
    pub notices: Vec<(String, String)>,
}

pub struct Watchers(mpsc::Sender<Event>);

impl Drop for Watchers {
    fn drop(&mut self) {
        let _ = self.0.send(Event::Stop);
    }
}

enum Event {
    Config(Box<ConfigUpdate>),
    Theme(u64, ThemeUpdate),
    Stop,
}

struct State {
    paths: XdgPaths,
    config: Config,
    source: ThemeSource,
    theme: LoadedTheme,
    generation: u64,
    theme_watcher: Option<ThemeWatcher>,
    _config_watcher: Option<ConfigWatcher>,
    control: mpsc::Sender<Event>,
}

pub fn watch(paths: XdgPaths) -> Result<(UnboundedReceiver<Update>, Watchers), std::io::Error> {
    let (control, events) = mpsc::channel();
    let (sender, receiver) = unbounded();
    let coordinator = control.clone();
    thread::Builder::new()
        .name("shoal-window-settings".into())
        .spawn(move || {
            let mut notices = Vec::new();
            let config_path = paths.config_file();
            let callbacks = coordinator.clone();
            let (watcher, initial) = match ConfigWatcher::new(&config_path, move |update| {
                let _ = callbacks.send(Event::Config(Box::new(update)));
            }) {
                Ok((watcher, initial)) => (Some(watcher), initial),
                Err(error) => {
                    notices.push((
                        "config-watch".into(),
                        format!(
                            "{}: {error}; changes will not be watched",
                            config_path.display()
                        ),
                    ));
                    let (loaded, error) = match Config::load(&config_path) {
                        Ok(loaded) => (loaded, None),
                        Err(error) => (Default::default(), Some(error.into())),
                    };
                    (None, ConfigUpdate { loaded, error })
                }
            };
            config_notices(&initial, &mut notices);
            let config = initial.loaded.config;
            let source = ThemeSource::from_config(&config.theme, &paths);
            let mut state = State {
                paths,
                config,
                theme: LoadedTheme::default(),
                source,
                generation: 0,
                theme_watcher: None,
                _config_watcher: watcher,
                control: coordinator,
            };
            state.replace_theme(&mut notices);
            if !state.publish(&sender, notices) {
                return;
            }
            while let Ok(event) = events.recv() {
                if matches!(event, Event::Stop) {
                    return;
                }
                let mut notices = Vec::new();
                if !state.apply(event, &mut notices) {
                    continue;
                }
                if !state.publish(&sender, notices) {
                    return;
                }
            }
        })?;
    Ok((receiver, Watchers(control)))
}

impl State {
    fn apply(&mut self, event: Event, notices: &mut Vec<(String, String)>) -> bool {
        match event {
            Event::Stop => return false,
            Event::Config(update) => {
                config_notices(&update, notices);
                self.config = update.loaded.config;
                let source = ThemeSource::from_config(&self.config.theme, &self.paths);
                if source != self.source || self.theme_watcher.is_none() {
                    self.source = source;
                    self.generation += 1;
                    self.replace_theme(notices);
                }
            }
            Event::Theme(generation, update) => {
                // A replaced watcher may already have queued an old-source update.
                if generation != self.generation {
                    return false;
                }
                theme_notices(&update, &self.source, notices);
                self.theme = update.theme;
            }
        }
        true
    }

    fn replace_theme(&mut self, notices: &mut Vec<(String, String)>) {
        let control = self.control.clone();
        let generation = self.generation;
        let update = match ThemeWatcher::new(self.source.clone(), move |update| {
            let _ = control.send(Event::Theme(generation, update));
        }) {
            Ok((watcher, update)) => {
                self.theme_watcher = Some(watcher);
                update
            }
            Err(error) => {
                self.theme_watcher = None;
                notices.push(("theme-watch".into(), format!("{}: theme watch unavailable; changes will not be watched until config is reloaded", source_name(&self.source))));
                ThemeUpdate {
                    theme: self.source.load(),
                    error: Some(error),
                }
            }
        };
        theme_notices(&update, &self.source, notices);
        self.theme = update.theme;
    }

    fn publish(&self, sender: &UnboundedSender<Update>, notices: Vec<(String, String)>) -> bool {
        sender
            .unbounded_send(Update {
                config: self.config.clone(),
                palette: self.theme.palette(&self.config),
                notices,
            })
            .is_ok()
    }
}

fn config_notices(update: &ConfigUpdate, notices: &mut Vec<(String, String)>) {
    if let Some(error) = &update.error {
        notices.push(("local-config-error".into(), error.to_string()));
    }
    notices.extend(
        update
            .loaded
            .warnings
            .iter()
            .map(|warning| (format!("config:{}", warning.key), warning.to_string())),
    );
}

fn theme_notices(update: &ThemeUpdate, source: &ThemeSource, notices: &mut Vec<(String, String)>) {
    if let Some(error) = &update.error {
        notices.push((
            "local-theme-error".into(),
            format!("{}: {error}", source_name(source)),
        ));
    }
    notices.extend(
        update
            .theme
            .warnings
            .iter()
            .map(|warning| ("local-theme".into(), warning.to_string())),
    );
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

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
