//! The daemon's current config and palette, including native watching and
//! banner-worthy diagnostics for the workspace actor (M1 spec §9).

mod watch;

pub use watch::{Update, Watchers, watch};

use std::{ffi::OsString, path::PathBuf};

use shoal_config::{Config, Palette, ThemeSource, XdgPaths};
use shoal_term::{Rgb, Theme};

use crate::pane::PaneSettings;

/// The config and palette in force.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub config: Config,
    pub palette: Palette,
}

impl Settings {
    /// Read `config.toml` and the active theme. Nothing here stops the daemon:
    /// an unreadable or invalid config means defaults, and theme problems
    /// fall through to the next color source. Each problem is logged.
    pub fn load(paths: &XdgPaths) -> Self {
        let file = paths.config_file();
        let config = match Config::load(&file) {
            Ok(loaded) => {
                for warning in &loaded.warnings {
                    tracing::warn!(%warning, "config");
                }
                loaded.config
            }
            Err(error) => {
                tracing::error!(%error, "config not loaded; using defaults");
                Config::default()
            }
        };
        let theme = ThemeSource::from_config(&config.theme, paths).load();
        for warning in &theme.warnings {
            tracing::warn!(%warning, "theme");
        }
        match &theme.folder {
            Some(folder) => tracing::info!(folder = %folder.display(), "theme loaded"),
            None => tracing::info!("theme loaded without an Omarchy theme folder"),
        }
        let palette = theme.palette(&config);
        Self { config, palette }
    }

    /// What every new pane starts with. `base_env` is the daemon's own
    /// environment, captured once at startup.
    pub fn pane(
        &self,
        paths: &XdgPaths,
        socket: PathBuf,
        base_env: Vec<(OsString, OsString)>,
    ) -> PaneSettings {
        let program = &self.config.shell.program;
        PaneSettings {
            shell_program: (!program.is_empty()).then(|| program.clone()),
            shell_integration: self.config.shell.integration,
            prompt_mode: self.config.prompt.mode,
            scrollback: self.config.scrollback.lines,
            persist: self.config.scrollback.persist,
            theme: theme(&self.palette),
            socket,
            home: paths.home().to_owned(),
            base_env,
        }
    }
}

/// The colors a pane terminal answers with (M1 spec §9.2).
pub fn theme(palette: &Palette) -> Theme {
    let rgb = |color: shoal_config::schema::Rgb| {
        let [r, g, b] = color.0;
        Rgb { r, g, b }
    };
    Theme {
        foreground: rgb(palette.foreground),
        background: rgb(palette.background),
        cursor: rgb(palette.cursor),
        ansi: palette.ansi.map(rgb),
    }
}
