//! A pane's numbered output stream (M1 spec §4.2, §5.5): output batched into
//! items of at most 64 KiB, at most one every 4 ms, each with the next
//! sequence number, and fanned out to subscribers. The caller passes the time
//! in, so batching never needs a timer while the pane is quiet.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use shoal_model::GridSize;
use tokio::sync::mpsc::UnboundedSender;

/// The most output in one item.
pub const BATCH_BYTES: usize = 64 * 1024;
/// The least time between two output items.
pub const BATCH_INTERVAL: Duration = Duration::from_millis(4);

/// One numbered step of a pane's stream. Sequence numbers start at 1; a
/// pane that has produced nothing is at 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Output { seq: u64, bytes: Arc<[u8]> },
    Resized { seq: u64, size: GridSize },
}

impl Item {
    pub fn seq(&self) -> u64 {
        match self {
            Self::Output { seq, .. } | Self::Resized { seq, .. } => *seq,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriberId(u64);

/// A new subscriber gets every item after `seq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subscribed {
    pub id: SubscriberId,
    pub seq: u64,
}

#[derive(Debug, Default)]
pub struct Stream {
    seq: u64,
    pending: Vec<u8>,
    last_flush: Option<Instant>,
    subscribers: Vec<(SubscriberId, UnboundedSender<Item>)>,
    next_subscriber: u64,
}

impl Stream {
    /// The last sequence number handed out. Output still being batched has
    /// none yet; [`Stream::flush`] first when it must count.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Add output, sending it now unless an item went out less than
    /// [`BATCH_INTERVAL`] ago.
    pub fn push(&mut self, mut bytes: &[u8], now: Instant) {
        while !bytes.is_empty() {
            let room = BATCH_BYTES - self.pending.len();
            let (now_bytes, rest) = bytes.split_at(room.min(bytes.len()));
            self.pending.extend_from_slice(now_bytes);
            bytes = rest;
            if self.pending.len() == BATCH_BYTES {
                self.flush(now);
            }
        }
        let due = self
            .last_flush
            .is_none_or(|last| now >= last + BATCH_INTERVAL);
        if due {
            self.flush(now);
        }
    }

    /// When batched output must go out; `None` when nothing is waiting.
    pub fn deadline(&self) -> Option<Instant> {
        // Output is held only after an earlier flush, so `last_flush` is set.
        self.last_flush
            .filter(|_| !self.pending.is_empty())
            .map(|last| last + BATCH_INTERVAL)
    }

    /// Send whatever output is waiting as one item.
    pub fn flush(&mut self, now: Instant) {
        if self.pending.is_empty() {
            return;
        }
        let bytes: Arc<[u8]> = std::mem::take(&mut self.pending).into();
        self.last_flush = Some(now);
        self.seq += 1;
        let seq = self.seq;
        self.send(Item::Output { seq, bytes });
    }

    /// Number a resize after the output that came before it.
    pub fn resized(&mut self, size: GridSize, now: Instant) {
        self.flush(now);
        self.seq += 1;
        let seq = self.seq;
        self.send(Item::Resized { seq, size });
    }

    pub fn subscribe(&mut self, sink: UnboundedSender<Item>, now: Instant) -> Subscribed {
        self.flush(now);
        self.next_subscriber += 1;
        let id = SubscriberId(self.next_subscriber);
        self.subscribers.push((id, sink));
        Subscribed { id, seq: self.seq }
    }

    pub fn unsubscribe(&mut self, id: SubscriberId) {
        self.subscribers.retain(|(subscriber, _)| *subscriber != id);
    }

    fn send(&mut self, item: Item) {
        // A subscriber whose receiver is gone is dropped.
        self.subscribers
            .retain(|(_, sink)| sink.send(item.clone()).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    use super::*;

    fn subscribed(stream: &mut Stream, now: Instant) -> UnboundedReceiver<Item> {
        let (sink, items) = unbounded_channel();
        stream.subscribe(sink, now);
        items
    }

    fn drain(items: &mut UnboundedReceiver<Item>) -> Vec<(u64, Vec<u8>)> {
        let mut got = Vec::new();
        while let Ok(item) = items.try_recv() {
            match item {
                Item::Output { seq, bytes } => got.push((seq, bytes.to_vec())),
                Item::Resized { seq, size } => {
                    got.push((seq, format!("{}x{}", size.cols, size.rows).into_bytes()))
                }
            }
        }
        got
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn output_after_quiet_goes_out_at_once_and_a_burst_waits_4_ms() {
        let start = Instant::now();
        let mut stream = Stream::default();
        let mut items = subscribed(&mut stream, start);
        stream.push(b"a", start);
        assert_eq!(drain(&mut items), [(1, b"a".to_vec())]);
        assert_eq!(stream.deadline(), None);

        stream.push(b"b", start + ms(1));
        stream.push(b"c", start + ms(2));
        assert_eq!(drain(&mut items), []);
        assert_eq!(stream.deadline(), Some(start + ms(4)));
        stream.flush(start + ms(4));
        assert_eq!(drain(&mut items), [(2, b"bc".to_vec())]);

        // Quiet for longer than the interval: the next output is not held.
        stream.push(b"d", start + ms(20));
        assert_eq!(drain(&mut items), [(3, b"d".to_vec())]);
        assert_eq!(stream.seq(), 3);
    }

    #[test]
    fn items_never_exceed_64_kib() {
        let start = Instant::now();
        let mut stream = Stream::default();
        let mut items = subscribed(&mut stream, start);
        stream.push(b"x", start);
        stream.push(b"z", start + ms(1));
        stream.push(&vec![b'y'; BATCH_BYTES + 10], start + ms(2));
        let got = drain(&mut items);
        let sizes: Vec<_> = got.iter().map(|(seq, bytes)| (*seq, bytes.len())).collect();
        // The held "z" and the first y's fill an item to the cap; 11 y's wait.
        assert_eq!(sizes, [(1, 1), (2, BATCH_BYTES)]);
        assert_eq!(got[1].1[0], b'z');
        assert_eq!(stream.deadline(), Some(start + ms(6)));
        stream.flush(start + ms(6));
        assert_eq!(drain(&mut items), [(3, vec![b'y'; 11])]);
    }

    #[test]
    fn a_resize_follows_the_output_before_it() {
        let start = Instant::now();
        let mut stream = Stream::default();
        let mut items = subscribed(&mut stream, start);
        stream.push(b"a", start);
        stream.push(b"b", start + ms(1));
        stream.resized(
            GridSize {
                cols: 100,
                rows: 30,
            },
            start + ms(2),
        );
        assert_eq!(
            drain(&mut items),
            [
                (1, b"a".to_vec()),
                (2, b"b".to_vec()),
                (3, b"100x30".to_vec())
            ]
        );
    }

    #[test]
    fn subscribers_start_after_everything_already_numbered() {
        let start = Instant::now();
        let mut stream = Stream::default();
        stream.push(b"a", start);
        stream.push(b"b", start + ms(1));
        let (sink, mut late) = unbounded_channel();
        // Subscribing numbers the held "b", so it is not lost or doubled.
        assert_eq!(stream.subscribe(sink, start + ms(2)).seq, 2);
        stream.push(b"c", start + ms(10));
        assert_eq!(drain(&mut late), [(3, b"c".to_vec())]);
    }

    #[test]
    fn unsubscribed_and_dropped_subscribers_get_nothing_more() {
        let start = Instant::now();
        let mut stream = Stream::default();
        let (sink, mut first) = unbounded_channel();
        let id = stream.subscribe(sink, start).id;
        let second = subscribed(&mut stream, start);
        let mut third = subscribed(&mut stream, start);
        drop(second);
        stream.unsubscribe(id);
        stream.push(b"a", start);
        assert_eq!(drain(&mut first), []);
        assert_eq!(drain(&mut third), [(1, b"a".to_vec())]);
        assert_eq!(stream.subscribers.len(), 1);
    }
}
