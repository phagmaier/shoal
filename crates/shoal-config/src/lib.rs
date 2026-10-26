//! Configuration, keybindings, themes and user-specific paths for Shoal.
//!
//! [`Config::parse`] returns an entire validated edit plus unknown-key warnings.
//! [`ConfigFile`] keeps the last good edit across synchronous reload failures.
//! [`ThemeSource::load`] reads the active Omarchy theme (or a theme file) with
//! warnings instead of errors; [`LoadedTheme::palette`] layers `[theme.colors]`
//! on top and maps the result onto what Shoal draws (M1 spec §9.2).
//! [`ConfigWatcher::new`] and [`ThemeWatcher::new`] install native watches and
//! return the initial update; subsequent updates go to their callback on a worker
//! thread. Forward them to the daemon actor or GUI's foreground channel without
//! blocking that callback. Drop a watcher to stop and join its worker.

mod chord;
mod keymap;
mod load;
mod palette;
mod paths;
pub mod schema;
pub mod shell;
mod theme;
mod watch;
pub use chord::{Chord, ChordError, Key, Modifiers};
pub use keymap::{Action, ActionError, Keymap};
pub use load::{ConfigError, ConfigFile, ConfigWarning, LoadedConfig};
pub use palette::{Palette, ThemeColors};
pub use paths::{PathError, XdgPaths};
pub use schema::Config;
pub use theme::{LoadedTheme, ThemeSource, ThemeWarning};
pub use watch::{ConfigUpdate, ConfigWatcher, ThemeUpdate, ThemeWatcher, WatchError};
