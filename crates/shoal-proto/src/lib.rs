//! Shoal protocol version 1 (M1 spec §5), independent of sockets and runtimes.
//!
//! Decode a frame, then parse its JSON object as a [`Message`]. Requests retain
//! raw method names and parameters so the daemon can distinguish unknown methods
//! and bad parameters from invalid envelopes. Binary payloads borrow the input;
//! snapshot decoding and stream ordering belong to the terminal/client layers.

mod envelope;
mod error;
mod frame;
mod methods;

pub use frame::{
    Frame, FrameError, Input, MAX_FRAME_LEN, Output, Resized, Snapshot, decode, decode_exact,
};

pub const PROTOCOL_VERSION: u32 = 1;

pub use envelope::{EnvelopeError, Message, RawNotification, Request, RequestId, Response};
pub use error::{
    AttachFailureData, AttachFailureReason, CoreMismatchData, ProtocolMismatchData, RequestError,
    RpcError, error_code,
};
pub use methods::{
    AttachResult, Call, CaptureFormat, CaptureResult, ClientKind, DaemonStatusResult, DetachReason,
    Empty, HelloResult, NoticeLevel, Notification, PaneResult, SessionResult, SizeResult,
    TabCreateResult, WorkspaceState,
};
