//! Atomic attach, first-ground-point splits and subscriber resynchronization.

use shoal_term::SnapshotError;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Notify, oneshot};

use super::Runtime;
use crate::pane::{AttachError, Attachment, Delivery, Snapshot, SubscriberId};

struct Request {
    notify: Arc<Notify>,
    deadline: Instant,
    reply: oneshot::Sender<Result<Attachment, AttachError>>,
}

#[derive(Default)]
pub(super) struct Waiting {
    requests: Vec<Request>,
    pub resyncs: Vec<(SubscriberId, Instant)>,
}

impl Waiting {
    pub fn deadline(&self) -> Option<Instant> {
        self.requests
            .iter()
            .map(|r| r.deadline)
            .chain(self.resyncs.iter().map(|(_, deadline)| *deadline))
            .min()
    }
    fn active(&self) -> bool {
        !self.requests.is_empty() || !self.resyncs.is_empty()
    }
}

impl Runtime {
    fn attach_snapshot(&mut self, now: Instant) -> Result<Snapshot, AttachError> {
        self.stream.flush(now);
        let bytes = match self.terminal.snapshot() {
            Ok(bytes) => bytes,
            Err(SnapshotError::MidSequence) => return Err(AttachError::MidSequence),
            Err(SnapshotError::Encode(error)) => {
                tracing::error!(pane = %self.id, %error, "cannot encode pane snapshot");
                return Err(AttachError::Encode(error));
            }
        };
        if bytes.len() > shoal_proto::MAX_FRAME_LEN - 21 {
            return Err(AttachError::TooLarge);
        }
        Ok(Snapshot {
            seq: self.stream.seq(),
            size: self.size(),
            bytes,
        })
    }

    pub(super) fn begin_attach(
        &mut self,
        notify: Arc<Notify>,
        deadline: Instant,
        reply: oneshot::Sender<Result<Attachment, AttachError>>,
    ) {
        self.complete_attach(
            Request {
                notify,
                deadline,
                reply,
            },
            Instant::now(),
        );
    }

    fn complete_attach(&mut self, request: Request, now: Instant) {
        if request.reply.is_closed() {
            return;
        }
        if now >= request.deadline {
            let _ = request.reply.send(Err(AttachError::MidSequence));
            return;
        }
        match self.attach_snapshot(now) {
            Ok(snapshot) => {
                let (subscribed, items) = self.stream.subscribe(request.notify, now);
                let id = subscribed.id;
                if request
                    .reply
                    .send(Ok(Attachment {
                        snapshot,
                        subscribed,
                        items,
                    }))
                    .is_err()
                {
                    self.stream.unsubscribe(id);
                }
            }
            Err(AttachError::MidSequence) => self.waiting.requests.push(request),
            Err(error) => {
                let _ = request.reply.send(Err(error));
            }
        }
    }

    pub(super) fn expire_attaches(&mut self, now: Instant) {
        let requests = std::mem::take(&mut self.waiting.requests);
        for request in requests {
            if request.reply.is_closed() {
                continue;
            }
            if now >= request.deadline {
                let _ = request.reply.send(Err(AttachError::MidSequence));
            } else {
                self.waiting.requests.push(request);
            }
        }
        let resyncs = std::mem::take(&mut self.waiting.resyncs);
        for (id, deadline) in resyncs {
            if now >= deadline {
                self.stream.replace(id, Delivery::Detached);
                self.stream.unsubscribe(id);
            } else {
                self.waiting.resyncs.push((id, deadline));
            }
        }
    }

    pub(super) fn resync_overflows(&mut self, now: Instant) {
        for id in self.stream.resyncs() {
            if !self
                .waiting
                .resyncs
                .iter()
                .any(|(candidate, _)| *candidate == id)
            {
                self.resync(id, now + Duration::from_secs(2), now);
            }
        }
    }

    fn resync(&mut self, id: SubscriberId, deadline: Instant, now: Instant) {
        match self.attach_snapshot(now) {
            Ok(snapshot) => self.stream.replace(id, Delivery::Snapshot(snapshot)),
            Err(AttachError::MidSequence) => self.waiting.resyncs.push((id, deadline)),
            Err(error) => {
                tracing::error!(pane = %self.id, %error, "cannot resynchronize subscriber");
                self.stream.replace(id, Delivery::Failed);
                self.stream.unsubscribe(id);
            }
        }
    }

    fn reached_ground(&mut self, now: Instant) {
        self.expire_attaches(now);
        for request in std::mem::take(&mut self.waiting.requests) {
            self.complete_attach(request, now);
        }
        for (id, deadline) in std::mem::take(&mut self.waiting.resyncs) {
            self.resync(id, deadline, now);
        }
    }

    pub(super) fn feed(&mut self, output: &[u8]) {
        let now = Instant::now();
        let mut rest = output;
        if self.waiting.active() {
            match self.terminal.feed_until_ground(output) {
                Ok(Some(consumed)) => {
                    self.stream.push(&output[..consumed], now);
                    self.stream.flush(now);
                    self.reached_ground(now);
                    rest = &output[consumed..];
                }
                Ok(None) => {
                    self.stream.push(output, now);
                    return;
                }
                Err(error) => {
                    tracing::error!(pane = %self.id, %error, "cannot feed to parser ground point");
                    return;
                }
            }
        }
        self.terminal.feed(rest);
        self.stream.push(rest, now);
        self.resync_overflows(now);
    }
}
