//! JSON-RPC envelopes retain unknown methods and invalid parameters for dispatch.

use serde::de::{DeserializeOwned, Error as _};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::{Call, RequestError, RpcError};

/// A string or integral JSON number. The private representation excludes null,
/// booleans, fractional numbers and arrays, including on serialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct RequestId(Value);

impl From<u64> for RequestId {
    fn from(value: u64) -> Self {
        Self(value.into())
    }
}
impl From<i64> for RequestId {
    fn from(value: i64) -> Self {
        Self(value.into())
    }
}
impl From<String> for RequestId {
    fn from(value: String) -> Self {
        Self(value.into())
    }
}
impl From<&str> for RequestId {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}
impl<'de> Deserialize<'de> for RequestId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if value.is_string() || value.is_i64() || value.is_u64() {
            Ok(Self(value))
        } else {
            Err(D::Error::custom("request id must be a string or integer"))
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub id: RequestId,
    pub method: String,
    pub params: Value,
}

impl Request {
    pub fn from_call(id: RequestId, call: &Call) -> Result<Self, serde_json::Error> {
        #[derive(Deserialize)]
        struct Parts {
            method: String,
            params: Value,
        }
        let parts: Parts = serde_json::from_value(serde_json::to_value(call)?)?;
        Ok(Self {
            id,
            method: parts.method,
            params: parts.params,
        })
    }

    /// Dispatch validation is separate from envelope/framing validation.
    pub fn call(&self) -> Result<Call, RequestError> {
        if !Call::METHOD_NAMES.contains(&self.method.as_str()) {
            return Err(RequestError::UnknownMethod);
        }
        if !self.params.is_object() {
            return Err(RequestError::BadParams(serde_json::Error::custom(
                "parameters must be an object",
            )));
        }
        serde_json::from_value(serde_json::json!({
            "method": self.method, "params": self.params,
        }))
        .map_err(RequestError::BadParams)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawNotification {
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Response<T> {
    Success {
        id: RequestId,
        result: T,
    },
    Error {
        id: Option<RequestId>,
        error: RpcError,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Request(Request),
    Response(Response<Value>),
    Notification(RawNotification),
}

impl Message {
    /// A validated JSON-RPC envelope. Additional non-envelope fields are ignored.
    pub fn from_object(mut object: Map<String, Value>) -> Result<Self, EnvelopeError> {
        if object.remove("jsonrpc").as_ref().and_then(Value::as_str) != Some("2.0") {
            return Err(EnvelopeError::Invalid("jsonrpc must be 2.0"));
        }
        if let Some(method) = object.remove("method") {
            if object.contains_key("result") || object.contains_key("error") {
                return Err(EnvelopeError::Invalid(
                    "method and response fields cannot coexist",
                ));
            }
            let method = serde_json::from_value(method)?;
            let params = object
                .remove("params")
                .unwrap_or_else(|| Value::Object(Map::new()));
            return match object.remove("id") {
                Some(id) => Ok(Self::Request(Request {
                    id: serde_json::from_value(id)?,
                    method,
                    params,
                })),
                None => Ok(Self::Notification(RawNotification { method, params })),
            };
        }
        if object.contains_key("params") {
            return Err(EnvelopeError::Invalid("response cannot have params"));
        }
        let id = object
            .remove("id")
            .ok_or(EnvelopeError::Invalid("response needs an id"))?;
        match (object.remove("result"), object.remove("error")) {
            (Some(result), None) => Ok(Self::Response(Response::Success {
                id: serde_json::from_value(id)?,
                result,
            })),
            (None, Some(error)) => {
                if !error.is_object() {
                    return Err(EnvelopeError::Invalid("error must be an object"));
                }
                Ok(Self::Response(Response::Error {
                    id: serde_json::from_value(id)?,
                    error: serde_json::from_value(error)?,
                }))
            }
            _ => Err(EnvelopeError::Invalid(
                "response needs exactly one of result or error",
            )),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    #[error("invalid JSON-RPC envelope: {0}")]
    Invalid(&'static str),
    #[error("invalid JSON-RPC field")]
    Field(#[from] serde_json::Error),
}

impl Serialize for Request {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut wire = serializer.serialize_struct("Request", 4)?;
        wire.serialize_field("jsonrpc", "2.0")?;
        wire.serialize_field("id", &self.id)?;
        wire.serialize_field("method", &self.method)?;
        wire.serialize_field("params", &self.params)?;
        wire.end()
    }
}
impl Serialize for RawNotification {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut wire = serializer.serialize_struct("Notification", 3)?;
        wire.serialize_field("jsonrpc", "2.0")?;
        wire.serialize_field("method", &self.method)?;
        wire.serialize_field("params", &self.params)?;
        wire.end()
    }
}
impl<T: Serialize> Serialize for Response<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut wire = serializer.serialize_struct("Response", 3)?;
        wire.serialize_field("jsonrpc", "2.0")?;
        match self {
            Self::Success { id, result } => {
                wire.serialize_field("id", id)?;
                wire.serialize_field("result", result)?;
            }
            Self::Error { id, error } => {
                wire.serialize_field("id", id)?;
                wire.serialize_field("error", error)?;
            }
        }
        wire.end()
    }
}
impl Serialize for Message {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Request(request) => request.serialize(serializer),
            Self::Response(response) => response.serialize(serializer),
            Self::Notification(notification) => notification.serialize(serializer),
        }
    }
}
impl<'de> Deserialize<'de> for Message {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::from_object(Map::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}
impl<'de> Deserialize<'de> for Request {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Message::deserialize(deserializer)? {
            Message::Request(request) => Ok(request),
            _ => Err(D::Error::custom("expected a request")),
        }
    }
}
impl<'de> Deserialize<'de> for RawNotification {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Message::deserialize(deserializer)? {
            Message::Notification(notification) => Ok(notification),
            _ => Err(D::Error::custom("expected a notification")),
        }
    }
}
impl<'de, T: DeserializeOwned> Deserialize<'de> for Response<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match Message::deserialize(deserializer)? {
            Message::Response(Response::Success { id, result }) => Ok(Self::Success {
                id,
                result: serde_json::from_value(result).map_err(D::Error::custom)?,
            }),
            Message::Response(Response::Error { id, error }) => Ok(Self::Error { id, error }),
            _ => Err(D::Error::custom("expected a response")),
        }
    }
}
