//! The worker's receiving side: decoding frames, matching responses to
//! requests, the `hello` that completes a connection, notifications, and pane
//! frames for their streams.

use std::io;

use serde_json::Value;
use shoal_model::{GridSize, PaneId};
use shoal_proto::{Call, Empty, Frame, HelloResult, Message, Notification, Response, RpcError};
use tokio::io::AsyncReadExt as _;

use super::{Link, Reply, Worker};
use crate::{Error, Event, StreamItem, stream::Next};

const READ_BYTES: usize = 64 * 1024;

/// A decoded frame, owned, so the read buffer can be drained first.
enum Incoming {
    Json(serde_json::Map<String, Value>),
    Snapshot {
        pane: PaneId,
        seq: u64,
        size: GridSize,
        bytes: Vec<u8>,
    },
    Output {
        pane: PaneId,
        seq: u64,
        bytes: Vec<u8>,
    },
    Resized {
        pane: PaneId,
        seq: u64,
        size: GridSize,
    },
}

impl Worker {
    pub(super) fn read(&mut self, read: io::Result<usize>) {
        match read {
            Ok(0) => self.lose(Error::Connect("shoald closed the connection".into())),
            Ok(_) => loop {
                let incoming = match self.next_frame() {
                    Ok(Some(incoming)) => incoming,
                    Ok(None) => return,
                    Err(reason) => {
                        tracing::warn!(%reason, "closing the connection");
                        self.lose(Error::Protocol(reason));
                        return;
                    }
                };
                self.incoming(incoming);
                if self.link.is_none() {
                    return;
                }
            },
            Err(error) => self.lose(Error::Connect(error.to_string())),
        }
    }

    fn next_frame(&mut self) -> Result<Option<Incoming>, String> {
        let Some(link) = &mut self.link else {
            return Ok(None);
        };
        let Some((frame, used)) = shoal_proto::decode(&link.buffer).map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        let incoming = match frame {
            Frame::Json(object) => Incoming::Json(object),
            Frame::Snapshot(snapshot) => Incoming::Snapshot {
                pane: snapshot.pane,
                seq: snapshot.seq,
                size: snapshot.size,
                bytes: snapshot.bytes.to_vec(),
            },
            Frame::Output(output) => Incoming::Output {
                pane: output.pane,
                seq: output.seq,
                bytes: output.bytes.to_vec(),
            },
            Frame::Resized(resized) => Incoming::Resized {
                pane: resized.pane,
                seq: resized.seq,
                size: resized.size,
            },
            Frame::Input(_) => return Err("INPUT from the daemon".into()),
        };
        link.buffer.drain(..used);
        Ok(Some(incoming))
    }

    fn incoming(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::Json(object) => match Message::from_object(object) {
                Ok(Message::Response(response)) => self.response(response),
                Ok(Message::Notification(raw)) => match raw.notification() {
                    Ok(notification) => self.notification(notification),
                    // Reserved or newer notifications are not for M1 clients.
                    Err(error) => {
                        tracing::debug!(method = raw.method, %error, "ignored a notification")
                    }
                },
                Ok(Message::Request(request)) => {
                    tracing::debug!(method = request.method, "ignored a request from shoald");
                }
                Err(error) => self.lose(Error::Protocol(error.to_string())),
            },
            Incoming::Snapshot {
                pane,
                seq,
                size,
                bytes,
            } => {
                if let Some(attachment) = self.attachments.get_mut(&pane) {
                    attachment.tracker.snapshot(seq);
                    let _ = attachment
                        .items
                        .send(StreamItem::Snapshot { seq, size, bytes });
                }
            }
            Incoming::Output { pane, seq, bytes } => {
                self.item(pane, seq, StreamItem::Output { seq, bytes });
            }
            Incoming::Resized { pane, seq, size } => {
                self.item(pane, seq, StreamItem::Resized { seq, size });
            }
        }
    }

    fn item(&mut self, pane: PaneId, seq: u64, item: StreamItem) {
        let Some(attachment) = self.attachments.get_mut(&pane) else {
            return;
        };
        match attachment.tracker.item(seq) {
            Next::Deliver => {
                let _ = attachment.items.send(item);
            }
            Next::Skip => {}
            Next::Gap => {
                tracing::warn!(%pane, seq, "stream gap; re-attaching");
                let stream = attachment.stream;
                self.attach(pane, stream);
            }
        }
    }

    fn response(&mut self, response: Response<Value>) {
        let (id, result) = match response {
            Response::Success { id, result } => (id, Ok(result)),
            Response::Error {
                id: Some(id),
                error,
            } => (id, Err(error)),
            Response::Error { id: None, error } => {
                tracing::warn!(
                    code = error.code,
                    message = error.message,
                    "error from shoald"
                );
                return;
            }
        };
        let Some(pending) = serde_json::to_value(&id)
            .ok()
            .and_then(|id| id.as_u64())
            .and_then(|id| self.pending.remove(&id))
        else {
            return;
        };
        match pending.reply {
            Reply::Hello => self.hello(result),
            Reply::Subscribe(reply) => {
                if result.is_ok() {
                    self.subscribed = true;
                }
                if let Some(reply) = reply {
                    let _ = reply.send(result.map_err(Error::Rpc));
                }
            }
            Reply::Attach { pane, stream } => {
                if let Err(error) = result {
                    self.attach_failed(pane, stream, Error::Rpc(error));
                }
            }
            reply => self.resolve(reply, result.map_err(Error::Rpc)),
        }
    }

    fn hello(&mut self, result: Result<Value, RpcError>) {
        let hello = match result {
            Ok(value) => match serde_json::from_value::<HelloResult>(value) {
                Ok(hello) => hello,
                Err(error) => return self.lose(Error::Protocol(error.to_string())),
            },
            Err(error) => return self.lose(Error::Rpc(error)),
        };
        self.backoff.reset();
        let held = match &mut self.link {
            Some(link) => {
                link.hello = None;
                std::mem::take(&mut link.held)
            }
            None => Vec::new(),
        };
        match self.ready.take() {
            Some(ready) => {
                let _ = ready.send(Ok(hello));
            }
            None => {
                tracing::info!(pid = hello.pid, "reconnected to shoald");
                let _ = self.events.send(Event::Connected(hello));
            }
        }
        for notification in held {
            self.notification(notification);
        }
        if self.subscribed {
            let timeout = self.options.request_timeout;
            self.send(
                &Call::WorkspaceSubscribe(Empty {}),
                timeout,
                Reply::Subscribe(None),
            );
        }
        let streams: Vec<_> = self
            .attachments
            .iter()
            .map(|(&pane, attachment)| (pane, attachment.stream))
            .collect();
        for (pane, stream) in streams {
            self.attach(pane, stream);
        }
    }

    fn notification(&mut self, notification: Notification) {
        if let Some(link) = self.link.as_mut().filter(|link| link.hello.is_some()) {
            link.held.push(notification);
            return;
        }
        if let Notification::PaneDetached { pane, reason } = notification {
            if let Some(attachment) = self.attachments.remove(&pane) {
                let _ = attachment.items.send(StreamItem::Detached(reason));
            }
            return;
        }
        let _ = self.events.send(Event::Notification(notification));
    }

    pub(super) fn attach_failed(&mut self, pane: PaneId, stream: u64, error: Error) {
        if self
            .attachments
            .get(&pane)
            .is_some_and(|attachment| attachment.stream == stream)
            && let Some(attachment) = self.attachments.remove(&pane)
        {
            let _ = attachment.items.send(StreamItem::Failed(error));
        }
    }
}

pub(super) async fn read(link: &mut Option<Link>) -> io::Result<usize> {
    let Some(link) = link else {
        return std::future::pending().await;
    };
    link.buffer.reserve(READ_BYTES);
    link.reader.read_buf(&mut link.buffer).await
}
