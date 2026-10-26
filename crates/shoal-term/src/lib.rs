//! Shoal's only wrapper around libghostty-vt. No other crate imports libghostty.
//!
//! The daemon's pane terminal is [`Terminal::pane`]: the one terminal that
//! answers program queries and reports metadata events. Windows build mirrors
//! with [`Terminal::restore`] from a [`Terminal::snapshot`] taken by a daemon
//! with the same [`core_id`], then feed them live output. Persistence formats the
//! primary screen with [`Terminal::persisted`] and [`Persisted::into_vt`].
//! Windows draw a terminal through [`Terminal::frame`] and encode input with
//! [`Terminal::encode_key`], [`Terminal::encode_mouse`],
//! [`Terminal::encode_focus`] and [`Terminal::encode_paste`].
//! Terminals are not `Send`; each stays on the thread that made it.

mod alloc;
mod core;
mod input;
mod interaction;
mod mouse;
mod pane;
mod persist;
mod render;
mod snapshot;
mod terminal;

pub use core::core_id;
pub use input::{Key, KeyAction, KeyEvent, Mods};
pub use interaction::{CellPoint, Scroll, TextLine, Viewport};
pub use mouse::{CellSize, MouseAction, MouseButton, MouseEvent};
pub use pane::{EventSink, PaneSetup, PromptMark, TermEvent, Theme};
pub use persist::{FormatVtError, Persisted};
pub use render::{
    Cell, CellStyle, CellWidth, Colors, Cursor, CursorPosition, CursorShape, Dirty, Frame, Rgb,
    RowCells, RowId, Rows, Underline,
};
pub use snapshot::{
    MAX_DECODE_BYTES, MAX_DECODE_CELLS, MAX_HISTORY_ROWS, MAX_SIZE, RestoreError, SnapshotError,
};
pub use terminal::{CONTINUATION_BYTES, CaptureFormat, Mode, ReplySink, TermError, Terminal};
