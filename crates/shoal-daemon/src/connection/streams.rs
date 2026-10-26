//! One socket writer multiplexes bounded control and per-pane byte queues.

use std::{collections::VecDeque, io, sync::Arc};

use shoal_daemon::pane::{Delivery, Item, Receiver};
use shoal_model::PaneId;
use shoal_proto::{Frame, Notification, RawNotification};
use tokio::{
    io::AsyncWriteExt,
    net::unix::OwnedWriteHalf,
    sync::{Notify, mpsc},
};

use crate::actor::OutgoingFrame;

pub(super) async fn write(
    mut writer: OwnedWriteHalf,
    mut controls: mpsc::Receiver<OutgoingFrame>,
    notify: Arc<Notify>,
) -> io::Result<()> {
    let mut streams: VecDeque<(PaneId, Receiver)> = VecDeque::new();
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        match controls.try_recv() {
            Ok(control) => {
                control_frame(&mut writer, &mut streams, control).await?;
                continue;
            }
            Err(mpsc::error::TryRecvError::Disconnected) => return Ok(()),
            Err(mpsc::error::TryRecvError::Empty) => {}
        }
        let mut sent = false;
        // Round robin across panes; a noisy pane cannot starve the others.
        for _ in 0..streams.len() {
            let (pane, mut items) = streams
                .pop_front()
                .expect("invariant: counted stream exists");
            match items.try_recv() {
                Ok(delivery) => {
                    let detached = matches!(delivery, Delivery::Detached);
                    let bytes = encode(pane, delivery)?;
                    if !detached {
                        streams.push_back((pane, items));
                    }
                    writer.write_all(&bytes).await?;
                    sent = true;
                    break;
                }
                Err(mpsc::error::TryRecvError::Empty) => streams.push_back((pane, items)),
                Err(mpsc::error::TryRecvError::Disconnected) => {}
            }
        }
        if sent {
            continue;
        }
        tokio::select! {
            control = controls.recv() => match control {
                Some(control) => control_frame(&mut writer, &mut streams, control).await?,
                None => return Ok(()),
            },
            () = &mut notified => {},
        }
    }
}

async fn control_frame(
    writer: &mut OwnedWriteHalf,
    streams: &mut VecDeque<(PaneId, Receiver)>,
    control: OutgoingFrame,
) -> io::Result<()> {
    match control {
        OutgoingFrame::Wire(bytes) => writer.write_all(&bytes).await?,
        OutgoingFrame::Detach(pane) => streams.retain(|(id, _)| *id != pane),
        OutgoingFrame::Attach {
            pane,
            snapshot,
            response,
            items,
        } => {
            streams.retain(|(id, _)| *id != pane);
            // No other frame can intervene in the initial attach pair.
            writer.write_all(&snapshot).await?;
            writer.write_all(&response).await?;
            streams.push_back((pane, items));
        }
    }
    Ok(())
}

fn encode(pane: PaneId, delivery: Delivery) -> io::Result<Vec<u8>> {
    let result = match delivery {
        Delivery::Item(Item::Output { seq, bytes }) => Frame::Output(shoal_proto::Output {
            pane,
            seq,
            bytes: &bytes,
        })
        .encode(),
        Delivery::Item(Item::Resized { seq, size }) => {
            Frame::Resized(shoal_proto::Resized { pane, seq, size }).encode()
        }
        Delivery::Snapshot(snapshot) => Frame::Snapshot(shoal_proto::Snapshot {
            pane,
            seq: snapshot.seq,
            size: snapshot.size,
            bytes: &snapshot.bytes,
        })
        .encode(),
        Delivery::Detached => {
            let notification = RawNotification::from_notification(&Notification::PaneDetached {
                pane,
                reason: shoal_proto::DetachReason::MidSequence,
            })
            .map_err(io::Error::other)?;
            let serde_json::Value::Object(object) =
                serde_json::to_value(notification).map_err(io::Error::other)?
            else {
                unreachable!("invariant: notification is an object");
            };
            Frame::Json(object).encode()
        }
        Delivery::Failed => return Err(io::Error::other("pane resynchronization failed")),
    };
    result.map_err(io::Error::other)
}
