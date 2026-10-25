//! Shoal's only wrapper around libghostty-vt. No other crate imports libghostty.
//!
//! The daemon's pane terminal is [`Terminal::with_replies`]: the one terminal
//! that answers program queries. Windows build mirrors with
//! [`Terminal::restore`] from a [`Terminal::snapshot`] taken by a daemon with
//! the same [`core_id`], then feed them live output. Persistence formats the
//! primary screen with [`Terminal::persisted`] and [`Persisted::into_vt`].
//! Terminals are not `Send`; each stays on the thread that made it.

mod alloc;
mod core;
mod persist;
mod snapshot;
mod terminal;

pub use core::core_id;
pub use persist::{FormatVtError, Persisted};
pub use snapshot::{
    MAX_DECODE_BYTES, MAX_DECODE_CELLS, MAX_HISTORY_ROWS, MAX_SIZE, RestoreError, SnapshotError,
};
pub use terminal::{CONTINUATION_BYTES, CaptureFormat, Mode, ReplySink, TermError, Terminal};
