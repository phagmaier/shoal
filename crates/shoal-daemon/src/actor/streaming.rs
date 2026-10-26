//! Stream registration and focus ownership live with the workspace actor.

use shoal_daemon::pane::{AttachError, Attachment};
use shoal_model::{GridSize, PaneId};
use shoal_proto::{Frame, Response, RpcError, error_code};

use super::{
    Actor, ConnectionId, Message, OutgoingFrame, Reply, encode, error, json, spawning::missing,
};

impl Actor {
    pub(super) fn attach(&mut self, reply: Reply, pane: PaneId) {
        let Some(connection) = self.connections.get(&reply.connection) else {
            return;
        };
        if connection.core.as_deref() != Some(shoal_term::core_id()) {
            self.finish(
                reply,
                Err(RpcError {
                    code: error_code::VERSION_MISMATCH,
                    message: "terminal core differs from shoald".into(),
                    data: Some(json!({"daemon_core": shoal_term::core_id()})),
                }),
            );
            return;
        }
        let Some(runtime) = self.panes.get(&pane) else {
            self.finish(reply, Err(missing("pane", pane)));
            return;
        };
        let result = runtime.attach(connection.outgoing.streams.clone());
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let result = match result.await {
                Ok(result) => result.map_err(attach_error),
                Err(_) => Err(error(
                    error_code::INTERNAL,
                    "pane thread ended during attach",
                )),
            };
            let _ = sender.send(Message::Attached {
                reply,
                pane,
                result,
            });
        });
    }

    pub(super) fn attached(
        &mut self,
        reply: Reply,
        pane: PaneId,
        result: Result<Attachment, RpcError>,
    ) {
        let attachment = match result {
            Ok(attachment) => attachment,
            Err(error) => {
                self.finish(reply, Err(error));
                return;
            }
        };
        if !self.connections.contains_key(&reply.connection) {
            if let Some(runtime) = self.panes.get(&pane) {
                runtime.unsubscribe(attachment.subscribed.id);
            }
            return;
        }
        if !self.panes.contains_key(&pane) {
            self.finish(reply, Err(missing("pane", pane)));
            return;
        }
        let snapshot = &attachment.snapshot;
        let wire = Frame::Snapshot(shoal_proto::Snapshot {
            pane,
            seq: snapshot.seq,
            size: snapshot.size,
            bytes: &snapshot.bytes,
        })
        .encode();
        let response = encode(&Response::Success {
            id: reply.id.clone(),
            result: json!({"seq":snapshot.seq,"cols":snapshot.size.cols,"rows":snapshot.size.rows}),
        });
        let (wire, response) = match (wire, response) {
            (Ok(wire), Ok(response)) => (wire.into(), response),
            _ => {
                self.panes[&pane].unsubscribe(attachment.subscribed.id);
                self.finish(
                    reply,
                    Err(error(error_code::INTERNAL, "cannot encode attach frames")),
                );
                return;
            }
        };
        let connection = self
            .connections
            .get_mut(&reply.connection)
            .expect("invariant: connected attach");
        if let Some(previous) = connection
            .attachments
            .insert(pane, attachment.subscribed.id)
        {
            self.panes[&pane].unsubscribe(previous);
        }
        self.send_frame(
            reply.connection,
            OutgoingFrame::Attach {
                pane,
                snapshot: wire,
                response,
                items: attachment.items,
            },
        );
        self.release(reply);
    }

    pub(super) fn detach(
        &mut self,
        connection: ConnectionId,
        pane: PaneId,
    ) -> Result<(), RpcError> {
        let runtime = self.panes.get(&pane).ok_or_else(|| missing("pane", pane))?;
        if let Some(subscriber) = self
            .connections
            .get_mut(&connection)
            .expect("invariant: connected detach")
            .attachments
            .remove(&pane)
        {
            runtime.unsubscribe(subscriber);
        }
        self.send_frame(connection, OutgoingFrame::Detach(pane));
        Ok(())
    }

    pub(super) fn set_size(&mut self, reply: Reply, pane: PaneId, cols: u16, rows: u16) {
        if !(1..=1000).contains(&cols) || !(1..=1000).contains(&rows) {
            self.finish(
                reply,
                Err(error(
                    error_code::BAD_PARAMS,
                    "pane dimensions must be between 1 and 1000",
                )),
            );
            return;
        }
        let Some(runtime) = self.panes.get(&pane) else {
            self.finish(reply, Err(missing("pane", pane)));
            return;
        };
        let result = runtime.set_size(
            (self.size_owner == Some(reply.connection)).then_some(GridSize { cols, rows }),
        );
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let result = result
                .await
                .map_err(|_| "pane thread ended during resize".to_owned());
            let _ = sender.send(Message::Sized {
                reply,
                pane,
                result,
            });
        });
    }

    pub(super) fn sized(
        &mut self,
        reply: Reply,
        pane: PaneId,
        result: Result<(u64, GridSize), String>,
    ) {
        let (seq, size) = match result {
            Ok(sized) => sized,
            Err(reason) => {
                self.finish(reply, Err(error(error_code::INTERNAL, reason)));
                return;
            }
        };
        let Some(record) = self.workspace.panes.get_mut(&pane) else {
            self.finish(reply, Err(missing("pane", pane)));
            return;
        };
        // Completion tasks from different connections may arrive in a different
        // order from the pane's commands. An older answer cannot regress state.
        if self
            .size_sequences
            .get(&pane)
            .is_none_or(|last| seq >= *last)
        {
            self.size_sequences.insert(pane, seq);
            if record.size != size {
                record.size = size;
                self.changed();
            }
        }
        self.finish(reply, Ok(json!({"cols":size.cols,"rows":size.rows})));
    }

    pub(super) fn remove_attachments(&mut self, pane: PaneId) {
        let connections: Vec<_> = self
            .connections
            .iter_mut()
            .filter_map(|(&id, connection)| {
                connection
                    .attachments
                    .remove(&pane)
                    .map(|subscriber| (id, subscriber))
            })
            .collect();
        for (id, subscriber) in connections {
            if let Some(runtime) = self.panes.get(&pane) {
                runtime.unsubscribe(subscriber);
            }
            if let Some(connection) = self.connections.get(&id)
                && connection
                    .outgoing
                    .frames
                    .try_send(OutgoingFrame::Detach(pane))
                    .is_err()
            {
                // A close cascade is still editing the layout and pane map.
                // Cancel socket work now, but change window counts only after
                // that complete model operation has restored its invariants.
                let _ = connection.outgoing.close.send(true);
                let _ = self.sender.send(Message::Disconnect(id));
            }
        }
        self.size_sequences.remove(&pane);
    }
}

fn attach_error(reason: AttachError) -> RpcError {
    match reason {
        AttachError::MidSequence | AttachError::TooLarge => RpcError {
            code: error_code::INVALID_STATE,
            data: Some(
                json!({"reason": if matches!(reason, AttachError::MidSequence) {"mid_sequence"} else {"too_large"}}),
            ),
            message: reason.to_string(),
        },
        AttachError::Encode(reason) => error(error_code::INTERNAL, reason.to_string()),
    }
}
