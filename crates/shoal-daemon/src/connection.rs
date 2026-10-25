//! One client connection: frames in, the `hello` handshake, then requests in
//! order (M1 spec §5.1–5.3).

use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use shoal_proto::{
    Call, EnvelopeError, Frame, HelloResult, Message, PROTOCOL_VERSION, ProtocolMismatchData,
    Request, RequestError, RequestId, Response, RpcError, error_code,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

/// What `hello` reports about this daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Daemon {
    pub version: &'static str,
    pub core: &'static str,
    pub pid: u32,
}

/// Serve `stream` until the peer leaves, breaks the protocol or fails `hello`.
pub async fn run(mut stream: UnixStream, daemon: Arc<Daemon>) {
    let mut handler = Handler::new(daemon);
    let mut buffer = Vec::new();
    let mut chunk = vec![0; 64 * 1024];
    loop {
        let decoded = match shoal_proto::decode(&buffer) {
            Ok(Some((frame, used))) => Some((handler.handle(frame), used)),
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(%error, "closing a connection after a protocol error");
                return;
            }
        };
        if let Some((outcome, used)) = decoded {
            buffer.drain(..used);
            if let Some(reply) = outcome.reply
                && let Err(error) = stream.write_all(&reply).await
            {
                tracing::debug!(%error, "a connection stopped accepting replies");
                return;
            }
            if outcome.close {
                return;
            }
            continue;
        }
        match stream.read(&mut chunk).await {
            Ok(0) => {
                if !buffer.is_empty() {
                    tracing::warn!("a connection closed partway through a frame");
                }
                return;
            }
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(error) => {
                tracing::debug!(%error, "a connection failed");
                return;
            }
        }
    }
}

/// The reply to one frame, already encoded, and whether the connection ends.
#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub reply: Option<Vec<u8>>,
    pub close: bool,
}

pub struct Handler {
    daemon: Arc<Daemon>,
    greeted: bool,
}

impl Handler {
    pub fn new(daemon: Arc<Daemon>) -> Self {
        Self {
            daemon,
            greeted: false,
        }
    }

    pub fn handle(&mut self, frame: Frame<'_>) -> Outcome {
        let message = match frame {
            Frame::Json(object) => Message::from_object(object),
            // Only window input travels as binary from a client; T13 routes it.
            Frame::Input(_) if self.greeted => return Outcome::none(),
            Frame::Input(_) => return self.before_hello(None),
            Frame::Output(_) | Frame::Snapshot(_) | Frame::Resized(_) => {
                tracing::warn!("closing a connection that sent a daemon-only frame");
                return Outcome::close(None);
            }
        };
        match (message, self.greeted) {
            (Ok(Message::Request(request)), false) if request.method == "hello" => {
                self.hello(&request)
            }
            (Ok(Message::Request(request)), false) => self.before_hello(Some(request.id)),
            (Ok(_), false) | (Err(_), false) => self.before_hello(None),
            (Ok(Message::Request(request)), true) => Outcome::reply(dispatch(&request)),
            // Clients send no notifications in M1, and the daemon makes no requests.
            (Ok(Message::Notification(_) | Message::Response(_)), true) => Outcome::none(),
            (Err(error), true) => Outcome::reply(invalid_request(&error)),
        }
    }

    fn hello(&mut self, request: &Request) -> Outcome {
        let protocol = match request.call() {
            Ok(Call::Hello { protocol, .. }) => protocol,
            Ok(_) => unreachable!("invariant: a request named hello parses as Call::Hello"),
            Err(error) => {
                return Outcome::close(Some(error_response(&request.id, request_error(&error))));
            }
        };
        if protocol != PROTOCOL_VERSION {
            let data = ProtocolMismatchData {
                daemon_protocol: PROTOCOL_VERSION,
                pid: self.daemon.pid,
            };
            let error = RpcError {
                code: error_code::VERSION_MISMATCH,
                message: format!("shoald speaks protocol {PROTOCOL_VERSION}, not {protocol}"),
                data: serde_json::to_value(data).ok(),
            };
            return Outcome::close(Some(error_response(&request.id, error)));
        }
        self.greeted = true;
        let result = HelloResult {
            protocol: PROTOCOL_VERSION,
            daemon_version: self.daemon.version.to_owned(),
            core: self.daemon.core.to_owned(),
            pid: self.daemon.pid,
        };
        Outcome::reply(success(&request.id, result))
    }

    /// Anything but `hello` first gets an error, then the connection closes.
    fn before_hello(&self, id: Option<RequestId>) -> Outcome {
        let error = RpcError {
            code: error_code::INVALID_STATE,
            message: "hello must come first".into(),
            data: None,
        };
        Outcome::close(Some(encode(&Response::<Value>::Error { id, error })))
    }
}

impl Outcome {
    fn none() -> Self {
        Self {
            reply: None,
            close: false,
        }
    }

    fn reply(reply: Vec<u8>) -> Self {
        Self {
            reply: Some(reply),
            close: false,
        }
    }

    fn close(reply: Option<Vec<u8>>) -> Self {
        Self { reply, close: true }
    }
}

/// A request after `hello`. T12 and T13 implement the workspace and pane methods.
fn dispatch(request: &Request) -> Vec<u8> {
    let error = match request.call() {
        Err(error) => request_error(&error),
        Ok(Call::Hello { .. }) => RpcError {
            code: error_code::INVALID_STATE,
            message: "hello was already sent".into(),
            data: None,
        },
        Ok(_) => RpcError {
            code: error_code::UNKNOWN_METHOD,
            message: format!("{} is not implemented yet", request.method),
            data: None,
        },
    };
    error_response(&request.id, error)
}

fn request_error(error: &RequestError) -> RpcError {
    let message = match error {
        RequestError::UnknownMethod => "unknown method".to_owned(),
        RequestError::BadParams(source) => format!("bad parameters: {source}"),
    };
    RpcError {
        code: error.code(),
        message,
        data: None,
    }
}

fn invalid_request(error: &EnvelopeError) -> Vec<u8> {
    let error = RpcError {
        code: error_code::INVALID_REQUEST,
        message: error.to_string(),
        data: None,
    };
    encode(&Response::<Value>::Error { id: None, error })
}

fn success(id: &RequestId, result: impl Serialize) -> Vec<u8> {
    encode(&Response::Success {
        id: id.clone(),
        result,
    })
}

fn error_response(id: &RequestId, error: RpcError) -> Vec<u8> {
    encode(&Response::<Value>::Error {
        id: Some(id.clone()),
        error,
    })
}

/// A JSON frame. Responses are objects well under the frame limit.
fn encode(message: &impl Serialize) -> Vec<u8> {
    let Ok(Value::Object(object)) = serde_json::to_value(message) else {
        unreachable!("invariant: responses serialize to JSON objects");
    };
    Frame::Json(object)
        .encode()
        .expect("invariant: a response fits in a frame")
}
