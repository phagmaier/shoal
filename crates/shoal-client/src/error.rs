//! What a client call can fail with.

use shoal_proto::RpcError;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    /// No daemon could be reached, even after auto-start.
    #[error("cannot connect to shoald: {0}")]
    Connect(String),
    /// The daemon answered with an error.
    #[error("{} (error {})", .0.message, .0.code)]
    Rpc(RpcError),
    #[error("shoald did not answer within {0:?}")]
    Timeout(std::time::Duration),
    /// The connection is down: the request was never sent, or it was lost
    /// with the connection.
    #[error("not connected to shoald")]
    NotConnected,
    /// The daemon broke the protocol, so the connection was closed.
    #[error("protocol error from shoald: {0}")]
    Protocol(String),
    /// The client's background thread has ended.
    #[error("the client has stopped")]
    Stopped,
}

impl Error {
    /// The daemon's error code, for an [`Error::Rpc`].
    pub fn code(&self) -> Option<i32> {
        match self {
            Self::Rpc(error) => Some(error.code),
            _ => None,
        }
    }
}
