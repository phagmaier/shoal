//! Reboot persistence (M1 spec §6): the `workspace.json` format and the files
//! under the state folder. Restore (§6.4) builds on the same types.

mod files;
mod format;
mod load;

pub use files::{Store, write_atomic};
pub use format::{Document, FORMAT, SavedPane, SavedSession, SavedTab, scrollback_path};
pub use load::{Loaded, Restored, divider, parse};
