//! Ordered control requests with independent socket readers and writers (§5).

mod streams;

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use serde::Serialize;
use serde_json::Value;
use shoal_proto::{
    Call, ClientKind, EnvelopeError, Frame, HelloResult, Message, PROTOCOL_VERSION,
    ProtocolMismatchData, Request, RequestError, RequestId, Response, RpcError, error_code,
};
use tokio::{
    io::AsyncReadExt,
    net::{UnixStream, unix::OwnedReadHalf},
    sync::{mpsc, watch},
};

use crate::actor::{ConnectionId, Handle, Outgoing, OutgoingFrame};

/// What `hello` reports about this daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Daemon {
    pub version: &'static str,
    pub core: &'static str,
    pub pid: u32,
}

enum Incoming {
    Json(Result<Message, EnvelopeError>),
    Input(shoal_model::PaneId, Vec<u8>),
}

/// A slow socket never blocks the workspace actor or another connection.
pub async fn run(stream: UnixStream, daemon: Arc<Daemon>, actor: Handle) {
    let (reader, writer) = stream.into_split();
    let (incoming, mut frames) = mpsc::channel(32);
    let (output, replies) = mpsc::channel(64);
    let streams = Arc::new(tokio::sync::Notify::new());
    let (close, mut closed) = watch::channel(false);
    let outgoing = Outgoing {
        frames: output,
        streams: streams.clone(),
        close,
    };
    let mut reading = tokio::spawn(read_frames(reader, incoming));
    let mut writing = tokio::spawn(streams::write(writer, replies, streams));
    let mut connection = None;
    // EOF is observed even while a pane is starting or a capture is pending.
    tokio::select! {
        _ = serve_frames(&mut frames, outgoing, daemon, &actor, &mut connection) => {},
        _ = &mut reading => {},
        _ = &mut writing => {},
        _ = closed.changed() => {},
    }
    if let Some(id) = connection {
        actor.disconnect(id);
    }
    reading.abort();
    // Drain a final protocol error/hello reply before closing. Only an actively
    // closing connection has this deadline; idle sockets have none.
    if !writing.is_finished()
        && tokio::time::timeout(Duration::from_millis(250), &mut writing)
            .await
            .is_err()
    {
        writing.abort();
    }
}

async fn read_frames(mut reader: OwnedReadHalf, incoming: mpsc::Sender<Incoming>) {
    let mut buffer = Vec::new();
    let mut chunk = vec![0; 64 * 1024];
    loop {
        match shoal_proto::decode(&buffer) {
            Ok(Some((frame, used))) => {
                let event = match frame {
                    Frame::Json(object) => Incoming::Json(Message::from_object(object)),
                    Frame::Input(input) => Incoming::Input(input.pane, input.bytes.to_vec()),
                    Frame::Output(_) | Frame::Snapshot(_) | Frame::Resized(_) => {
                        tracing::warn!("closing a connection that sent a daemon-only frame");
                        return;
                    }
                };
                buffer.drain(..used);
                if incoming.send(event).await.is_err() {
                    return;
                }
            }
            Ok(None) => match reader.read(&mut chunk).await {
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
            },
            Err(error) => {
                tracing::warn!(%error, "closing a connection after a protocol error");
                return;
            }
        }
    }
}

async fn serve_frames(
    frames: &mut mpsc::Receiver<Incoming>,
    outgoing: Outgoing,
    daemon: Arc<Daemon>,
    actor: &Handle,
    connection: &mut Option<ConnectionId>,
) {
    let sink = outgoing.frames.downgrade();
    let mut initial_sink = Some(outgoing);
    while let Some(frame) = frames.recv().await {
        if connection.is_none() {
            match frame {
                Incoming::Json(Ok(Message::Request(request))) if request.method == "hello" => {
                    match hello(&request, &daemon) {
                        Ok(Greeting {
                            client,
                            env,
                            core,
                            result,
                        }) => {
                            if !send(&sink, success(&request.id, result)) {
                                return;
                            }
                            let outgoing = initial_sink.take().expect("invariant: first hello");
                            *connection = Some(actor.connect(client, env, core, outgoing).await);
                        }
                        Err(error) => {
                            send(&sink, error_response(Some(request.id), error));
                            return;
                        }
                    }
                }
                other => {
                    let id = match other {
                        Incoming::Json(Ok(Message::Request(request))) => Some(request.id),
                        _ => None,
                    };
                    send(
                        &sink,
                        error_response(
                            id,
                            rpc_error(error_code::INVALID_STATE, "hello must come first"),
                        ),
                    );
                    return;
                }
            }
            continue;
        }
        match frame {
            Incoming::Json(Ok(Message::Request(request))) => {
                let id = connection.expect("invariant: greeted connection has an id");
                actor.request(id, request).await;
            }
            Incoming::Json(Err(error)) => {
                if !send(
                    &sink,
                    error_response(
                        None,
                        rpc_error(error_code::INVALID_REQUEST, error.to_string()),
                    ),
                ) {
                    return;
                }
            }
            Incoming::Input(pane, bytes) => {
                actor
                    .input(
                        connection.expect("invariant: greeted input has an id"),
                        pane,
                        bytes,
                    )
                    .await;
            }
            // Clients send no notifications in M1.
            Incoming::Json(Ok(Message::Notification(_) | Message::Response(_))) => {}
        }
    }
}

struct Greeting {
    client: ClientKind,
    env: BTreeMap<String, String>,
    core: Option<String>,
    result: HelloResult,
}

fn hello(request: &Request, daemon: &Daemon) -> Result<Greeting, RpcError> {
    let (protocol, client, env, core) =
        match request.call().map_err(|error| request_error(&error))? {
            Call::Hello {
                protocol,
                client,
                env,
                core,
                ..
            } => (protocol, client, env, core),
            _ => unreachable!("invariant: request named hello parses as Call::Hello"),
        };
    if protocol != PROTOCOL_VERSION {
        return Err(RpcError {
            code: error_code::VERSION_MISMATCH,
            message: format!("shoald speaks protocol {PROTOCOL_VERSION}, not {protocol}"),
            data: serde_json::to_value(ProtocolMismatchData {
                daemon_protocol: PROTOCOL_VERSION,
                pid: daemon.pid,
            })
            .ok(),
        });
    }
    Ok(Greeting {
        client,
        env,
        core,
        result: HelloResult {
            protocol: PROTOCOL_VERSION,
            daemon_version: daemon.version.to_owned(),
            core: daemon.core.to_owned(),
            pid: daemon.pid,
        },
    })
}

fn request_error(error: &RequestError) -> RpcError {
    let message = match error {
        RequestError::UnknownMethod => "unknown method".to_owned(),
        RequestError::BadParams(source) => format!("bad parameters: {source}"),
    };
    rpc_error(error.code(), message)
}

fn rpc_error(code: i32, message: impl Into<String>) -> RpcError {
    RpcError {
        code,
        message: message.into(),
        data: None,
    }
}

fn success(id: &RequestId, result: impl Serialize) -> Vec<u8> {
    encode(&Response::Success {
        id: id.clone(),
        result,
    })
}

fn error_response(id: Option<RequestId>, error: RpcError) -> Vec<u8> {
    encode(&Response::<Value>::Error { id, error })
}

fn encode(message: &impl Serialize) -> Vec<u8> {
    let Ok(Value::Object(object)) = serde_json::to_value(message) else {
        unreachable!("invariant: control responses serialize to JSON objects");
    };
    Frame::Json(object)
        .encode()
        .expect("invariant: small control response fits a frame")
}

fn send(sink: &mpsc::WeakSender<OutgoingFrame>, bytes: Vec<u8>) -> bool {
    sink.upgrade()
        .is_some_and(|sink| sink.try_send(OutgoingFrame::Wire(bytes.into())).is_ok())
}
