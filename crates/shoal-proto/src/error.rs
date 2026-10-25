//! M1 error codes and their specified data payloads.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod error_code {
    pub const INVALID_REQUEST: i32 = -32600;
    pub const UNKNOWN_METHOD: i32 = -32601;
    pub const NOT_FOUND: i32 = 1001;
    pub const INVALID_STATE: i32 = 1002;
    pub const VERSION_MISMATCH: i32 = 1003;
    pub const BAD_PARAMS: i32 = 1004;
    pub const INTERNAL: i32 = 1005;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    #[error("unknown method")]
    UnknownMethod,
    #[error("bad parameters")]
    BadParams(#[source] serde_json::Error),
}

impl RequestError {
    pub const fn code(&self) -> i32 {
        match self {
            Self::UnknownMethod => error_code::UNKNOWN_METHOD,
            Self::BadParams(_) => error_code::BAD_PARAMS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProtocolMismatchData {
    pub daemon_protocol: u32,
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CoreMismatchData {
    pub daemon_core: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AttachFailureData {
    pub reason: AttachFailureReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachFailureReason {
    TooLarge,
    MidSequence,
}
