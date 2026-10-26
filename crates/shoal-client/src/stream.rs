//! Attach streams (M1 spec §5.5): a snapshot, then every numbered item in
//! order. Anything out of order means the mirror is stale, so the client
//! drops items until a fresh snapshot from re-attaching replaces it.

use std::{
    sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError},
    time::Duration,
};

use shoal_model::{GridSize, PaneId};
use shoal_proto::DetachReason;
use tokio::sync::mpsc::UnboundedSender;

use crate::{Error, client::Command};

/// One step of a pane's stream, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    /// Replace the mirror: decode these bytes into a fresh terminal. Comes
    /// first, again after any re-attach (a gap or a reconnect), and when the
    /// daemon resyncs a slow reader.
    Snapshot {
        seq: u64,
        size: GridSize,
        bytes: Vec<u8>,
    },
    Output {
        seq: u64,
        bytes: Vec<u8>,
    },
    Resized {
        seq: u64,
        size: GridSize,
    },
    /// Attaching failed, as `pane.attach` reported it. The stream ends.
    Failed(Error),
    /// The daemon stopped streaming without being asked. The stream ends.
    Detached(DetachReason),
}

/// A pane's stream. Dropping it detaches.
#[derive(Debug)]
pub struct AttachStream {
    pub(crate) pane: PaneId,
    pub(crate) id: u64,
    pub(crate) items: Receiver<StreamItem>,
    pub(crate) commands: UnboundedSender<Command>,
}

impl AttachStream {
    pub fn pane(&self) -> PaneId {
        self.pane
    }

    /// The next item, waiting for it. `None` once the stream has ended.
    pub fn recv(&self) -> Option<StreamItem> {
        self.items.recv().ok()
    }

    /// The next item within `timeout`. `Ok(None)` if nothing came; `Err` once
    /// the stream has ended.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<StreamItem>, Error> {
        match self.items.recv_timeout(timeout) {
            Ok(item) => Ok(Some(item)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(Error::Stopped),
        }
    }

    /// The next item if one is waiting. `Err` once the stream has ended.
    pub fn try_recv(&self) -> Result<Option<StreamItem>, Error> {
        match self.items.try_recv() {
            Ok(item) => Ok(Some(item)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(Error::Stopped),
        }
    }
}

impl Drop for AttachStream {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Detach {
            pane: self.pane,
            stream: self.id,
        });
    }
}

/// Where a pane's stream is: the last sequence number delivered, or waiting
/// for a snapshot.
#[derive(Debug, Default)]
pub(crate) struct Tracker {
    last: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Next {
    Deliver,
    /// Waiting for a snapshot: drop it.
    Skip,
    /// Out of order: drop it and re-attach.
    Gap,
}

impl Tracker {
    pub fn snapshot(&mut self, seq: u64) {
        self.last = Some(seq);
    }

    pub fn item(&mut self, seq: u64) -> Next {
        match self.last {
            None => Next::Skip,
            Some(last) if last.checked_add(1) == Some(seq) => {
                self.last = Some(seq);
                Next::Deliver
            }
            Some(_) => {
                self.last = None;
                Next::Gap
            }
        }
    }

    /// The connection is gone; nothing counts until the next snapshot.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_follow_the_snapshot_in_order_and_a_gap_waits_for_the_next() {
        let mut tracker = Tracker::default();
        assert_eq!(tracker.item(1), Next::Skip, "nothing before a snapshot");
        tracker.snapshot(40);
        assert_eq!(tracker.item(41), Next::Deliver);
        assert_eq!(tracker.item(42), Next::Deliver);
        assert_eq!(tracker.item(44), Next::Gap, "43 is missing");
        assert_eq!(tracker.item(45), Next::Skip);
        tracker.snapshot(50);
        assert_eq!(tracker.item(51), Next::Deliver);
        assert_eq!(tracker.item(51), Next::Gap, "a repeat is out of order too");
        tracker.snapshot(60);
        tracker.reset();
        assert_eq!(tracker.item(61), Next::Skip);
        tracker.snapshot(u64::MAX);
        assert_eq!(tracker.item(0), Next::Gap);
    }
}
