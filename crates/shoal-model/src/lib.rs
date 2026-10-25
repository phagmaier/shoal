//! Shoal's workspace model: plain data shared by the daemon, windows and CLI
//! (M1 spec §3). Pure: no I/O, no async, no clock and no randomness.

mod id;
mod validate;
mod workspace;

pub use id::{PaneId, ParseIdError, SessionId, SplitId, TabId};
pub use validate::{AnyId, ValidationError, validate};
pub use workspace::{
    Axis, GridSize, Layout, Pane, PaneKind, PaneStatus, Session, Tab, Timestamp, Workspace,
};
