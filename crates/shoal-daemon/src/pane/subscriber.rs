//! Byte-bounded, replaceable stream queues. Only the queue is shared; terminals
//! and PTYs always stay on their pane thread. A socket writer holds at most one
//! additional frame while blocked in write_all.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use tokio::sync::{Notify, mpsc::error::TryRecvError};

use super::{Item, Snapshot};

pub const QUEUE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub enum Delivery {
    Item(Item),
    Snapshot(Snapshot),
    Detached,
    Failed,
}

impl Delivery {
    fn cost(&self) -> usize {
        match self {
            Self::Item(Item::Output { bytes, .. }) => 21 + bytes.len(),
            Self::Item(Item::Resized { .. }) => 25,
            // One replacement snapshot, bounded by the protocol's frame limit,
            // is held separately from the 8 MiB of incremental stream items.
            _ => 0,
        }
    }
}

#[derive(Debug, Default)]
struct State {
    items: VecDeque<Delivery>,
    bytes: usize,
    resync: bool,
    closed: bool,
    ended: bool,
}

#[derive(Debug)]
pub struct Receiver {
    state: Arc<Mutex<State>>,
}

#[derive(Debug)]
pub(super) struct Sender {
    state: Arc<Mutex<State>>,
    notify: Arc<Notify>,
}

pub(super) fn channel(notify: Arc<Notify>) -> (Sender, Receiver) {
    let state = Arc::new(Mutex::new(State::default()));
    (
        Sender {
            state: state.clone(),
            notify: notify.clone(),
        },
        Receiver { state },
    )
}

impl Sender {
    pub fn push(&self, item: Item) {
        let mut state = self
            .state
            .lock()
            .expect("invariant: queue lock is not poisoned");
        if state.closed || state.resync {
            return;
        }
        let delivery = Delivery::Item(item);
        let bytes = state.bytes + delivery.cost();
        if bytes > QUEUE_BYTES {
            state.items.clear();
            state.bytes = 0;
            state.resync = true;
        } else {
            state.bytes = bytes;
            state.items.push_back(delivery);
            self.notify.notify_one();
        }
    }

    pub fn needs_resync(&self) -> bool {
        let state = self
            .state
            .lock()
            .expect("invariant: queue lock is not poisoned");
        state.resync && !state.closed
    }

    pub fn closed(&self) -> bool {
        self.state
            .lock()
            .expect("invariant: queue lock is not poisoned")
            .closed
    }

    pub fn replace(&self, delivery: Delivery) {
        let mut state = self
            .state
            .lock()
            .expect("invariant: queue lock is not poisoned");
        if state.closed {
            return;
        }
        state.items.clear();
        state.bytes = 0;
        state.resync = false;
        state.items.push_back(delivery);
        self.notify.notify_one();
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.state
            .lock()
            .expect("invariant: queue lock is not poisoned")
            .ended = true;
        self.notify.notify_one();
    }
}

impl Receiver {
    pub fn try_recv(&mut self) -> Result<Delivery, TryRecvError> {
        let mut state = self
            .state
            .lock()
            .expect("invariant: queue lock is not poisoned");
        if let Some(item) = state.items.pop_front() {
            state.bytes -= item.cost();
            Ok(item)
        } else if state.ended {
            Err(TryRecvError::Disconnected)
        } else {
            Err(TryRecvError::Empty)
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        let mut state = self
            .state
            .lock()
            .expect("invariant: queue lock is not poisoned");
        state.closed = true;
        state.items.clear();
        state.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn byte_overflow_discards_stale_items_and_pauses_until_replaced() {
        let (sender, mut receiver) = channel(Arc::new(Notify::new()));
        let bytes: Arc<[u8]> = vec![b'x'; 65536].into();
        for seq in 1..=128 {
            sender.push(Item::Output {
                seq,
                bytes: bytes.clone(),
            });
        }
        // 128 payloads fit exactly, but their wire headers overflow the cap.
        assert!(sender.needs_resync());
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        sender.push(Item::Output {
            seq: 129,
            bytes: bytes.clone(),
        });
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        sender.replace(Delivery::Snapshot(Snapshot {
            seq: 129,
            size: shoal_model::GridSize { cols: 80, rows: 24 },
            bytes: vec![1, 2, 3],
        }));
        sender.push(Item::Output { seq: 130, bytes });
        assert!(matches!(
            receiver.try_recv(),
            Ok(Delivery::Snapshot(Snapshot { seq: 129, .. }))
        ));
        assert!(matches!(
            receiver.try_recv(),
            Ok(Delivery::Item(Item::Output { seq: 130, .. }))
        ));
        assert!(!sender.needs_resync());
    }
}
