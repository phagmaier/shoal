//! Configuration, keybindings and user-specific paths for Shoal.
//!
//! [`Config::parse`] returns an entire validated edit plus unknown-key warnings.
//! [`ConfigFile`] keeps the last good edit across synchronous reload failures.
//! [`ConfigWatcher::new`] installs native watches and returns the initial update;
//! subsequent updates go to its callback on a worker thread. Forward them to the
//! daemon actor or GUI's foreground channel without blocking that callback. Drop
//! the watcher to stop and join its worker. Theme-file lookup belongs to T06.

mod chord;
mod keymap;
mod load;
mod paths;
pub mod schema;
mod watch;
pub use chord::{Chord, ChordError, Key, Modifiers};
pub use keymap::{Action, ActionError, Keymap};
pub use load::{ConfigError, ConfigFile, ConfigWarning, LoadedConfig};
pub use paths::{PathError, XdgPaths};
pub use schema::Config;
pub use watch::{ConfigUpdate, ConfigWatcher, WatchError};
