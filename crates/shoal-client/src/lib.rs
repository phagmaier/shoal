//! The connection to `shoald` (M1 spec §4.1, §5): connecting with an
//! injectable auto-start, `hello`, requests matched by id with timeouts, the
//! notification stream, attach streams that re-attach on a gap, and
//! reconnecting with backoff.
//!
//! The client runs its own tokio runtime on a background thread. Its public
//! API is plain `std::sync::mpsc` channels and blocking calls; no tokio type
//! leaks out, so neither the CLI nor GPUI needs a tokio runtime.

mod backoff;
mod client;
mod error;
pub mod lifecycle;
mod options;
mod stream;
mod worker;

pub use client::{Client, Connected, Event};
pub use error::Error;
pub use options::{AutoStart, NoAutoStart, Options, wait_for_socket};
pub use stream::{AttachStream, StreamItem};

/// The workspace and protocol types callers build calls from and read
/// results into, so a client's users need no other Shoal crate.
pub use shoal_model as model;
pub use shoal_proto as proto;
