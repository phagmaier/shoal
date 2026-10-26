//! Scheduling persistence (M1 spec §6.3) without blocking the actor. Pane
//! threads capture; decoding, compressing and writing run on blocking workers,
//! one write per file at a time, and finish through actor messages.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use shoal_daemon::persist::{Document, Store};
use shoal_model::{PaneId, Timestamp};
use shoal_proto::error_code;
use shoal_term::{Persisted, SnapshotError};
use tokio::time::Instant;

use super::{Actor, Message, Reply, error, json};

const LAYOUT_INTERVAL: Duration = Duration::from_secs(1);

pub(super) struct Persistence {
    store: Store,
    enabled: bool,
    /// The newest layout state seen, to tell when a write is needed.
    latest: Option<Document>,
    dirty: bool,
    layout_busy: bool,
    last_layout: Option<Instant>,
    /// Flushes finished by the layout write in flight, and by the next one.
    layout_waiters: Vec<u64>,
    next_layout_waiters: Vec<u64>,
    /// Panes with a scrollback file written since persistence was enabled.
    saved: BTreeSet<PaneId>,
    writes: BTreeMap<PaneId, PaneWrite>,
    flushes: BTreeMap<u64, Flush>,
    next_flush: u64,
    pub last_persist_at: Option<Timestamp>,
}

/// A scrollback write in flight, and the newest capture waiting behind it.
struct PaneWrite {
    waiters: Vec<u64>,
    queued: Option<(Persisted, Vec<u64>)>,
}

/// Saving everything now, for `daemon.persist` or shutdown.
struct Flush {
    captures: BTreeSet<PaneId>,
    writes: BTreeSet<PaneId>,
    layout: bool,
    waiter: Option<Reply>,
    failed: Option<String>,
}

pub(super) enum Done {
    Captured {
        flush: u64,
        pane: PaneId,
        result: Result<Persisted, Skipped>,
    },
    Scrollback {
        pane: PaneId,
        result: Result<(), String>,
    },
    Layout(Result<(), String>),
}

pub(super) enum Skipped {
    /// Mid-sequence: the previous file stays (§6.3).
    MidSequence,
    Failed(String),
}

impl Persistence {
    pub fn new(store: Store, enabled: bool) -> Self {
        Self {
            store,
            enabled,
            latest: None,
            dirty: false,
            layout_busy: false,
            last_layout: None,
            layout_waiters: Vec::new(),
            next_layout_waiters: Vec::new(),
            saved: BTreeSet::new(),
            writes: BTreeMap::new(),
            flushes: BTreeMap::new(),
            next_flush: 1,
            last_persist_at: None,
        }
    }

    /// When the debounced layout write is due, if one is waiting.
    pub fn layout_deadline(&self) -> Option<Instant> {
        (self.dirty && !self.layout_busy).then(|| {
            self.last_layout
                .map_or_else(Instant::now, |last| last + LAYOUT_INTERVAL)
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// A restored pane's scrollback file is still valid: keep naming it until
    /// that pane's next write replaces it.
    pub fn restored(&mut self, pane: PaneId) {
        if self.enabled {
            self.saved.insert(pane);
        }
    }

    /// Writes or flushes are unfinished.
    pub fn busy(&self) -> bool {
        self.layout_busy || !self.writes.is_empty() || !self.flushes.is_empty()
    }
}

impl Actor {
    /// After any workspace change: schedule a layout write if what is saved
    /// would differ, apart from its time.
    pub(super) fn observe_persistence(&mut self) {
        let workspace = &self.workspace;
        let state = &mut self.persistence;
        state.saved.retain(|id| workspace.panes.contains_key(id));
        let document = Document::new(workspace, Timestamp::UNIX_EPOCH, |id| {
            state.saved.contains(&id)
        });
        if !state
            .latest
            .as_ref()
            .is_some_and(|latest| latest.same_state(&document))
        {
            state.latest = Some(document);
            state.dirty = true;
        }
    }

    pub(super) fn set_persist_enabled(&mut self, enabled: bool) {
        let state = &mut self.persistence;
        if state.enabled == enabled {
            return;
        }
        state.enabled = enabled;
        if !enabled {
            // The next layout write deletes the files (§6.3).
            state.saved.clear();
            state.dirty = true;
            let queued: Vec<_> = state
                .writes
                .iter_mut()
                .filter_map(|(&pane, write)| Some((pane, write.queued.take()?.1)))
                .collect();
            for (pane, waiters) in queued {
                self.wrote(pane, waiters, None);
            }
        }
        self.observe_persistence();
    }

    pub(super) fn persistence_done(&mut self, done: Done) {
        match done {
            Done::Captured {
                flush,
                pane,
                result,
            } => {
                let Some(state) = self.persistence.flushes.get_mut(&flush) else {
                    return;
                };
                state.captures.remove(&pane);
                match result {
                    Ok(persisted) => {
                        state.writes.insert(pane);
                        self.save_scrollback(pane, persisted, vec![flush]);
                    }
                    Err(Skipped::MidSequence) => {}
                    Err(Skipped::Failed(reason)) => {
                        tracing::warn!(%pane, %reason, "could not capture scrollback");
                        state.failed.get_or_insert(reason);
                    }
                }
                self.advance_flush(flush);
            }
            Done::Scrollback { pane, result } => {
                let write = self
                    .persistence
                    .writes
                    .remove(&pane)
                    .expect("invariant: a finished write was in flight");
                let state = &mut self.persistence;
                let failed = match result {
                    Ok(()) => {
                        state.last_persist_at = Some(Timestamp::now());
                        if state.enabled && self.workspace.panes.contains_key(&pane) {
                            state.saved.insert(pane);
                        } else {
                            // Closed or disabled meanwhile: clean up again.
                            state.dirty = true;
                        }
                        None
                    }
                    Err(reason) => {
                        tracing::warn!(%pane, %reason, "could not save scrollback");
                        Some(reason)
                    }
                };
                self.wrote(pane, write.waiters, failed);
                if let Some((persisted, waiters)) = write.queued {
                    self.save_scrollback(pane, persisted, waiters);
                }
                self.observe_persistence();
            }
            Done::Layout(result) => {
                let state = &mut self.persistence;
                state.layout_busy = false;
                let failed = match result {
                    Ok(()) => {
                        state.last_persist_at = Some(Timestamp::now());
                        None
                    }
                    // Retried at the next change, never on a timer.
                    Err(reason) => {
                        tracing::warn!(%reason, "could not save the workspace");
                        Some(reason)
                    }
                };
                for flush in std::mem::take(&mut state.layout_waiters) {
                    self.finish_flush(flush, failed.clone());
                }
                if !self.persistence.next_layout_waiters.is_empty() {
                    self.write_layout();
                }
            }
        }
    }

    /// The pane thread's scheduled capture.
    pub(super) fn scheduled_scrollback(&mut self, pane: PaneId, persisted: Persisted) {
        self.save_scrollback(pane, persisted, Vec::new());
    }

    /// Capture every pane now, write each, then the layout. A shutdown flush
    /// (no reply) must be started before panes are hung up.
    pub(super) fn flush(&mut self, waiter: Option<Reply>) {
        let state = &mut self.persistence;
        let id = state.next_flush;
        state.next_flush += 1;
        let captures: BTreeSet<_> = if state.enabled {
            self.panes.keys().copied().collect()
        } else {
            BTreeSet::new()
        };
        for &pane in &captures {
            let receiver = self.panes[&pane].persist();
            let sender = self.sender.clone();
            tokio::spawn(async move {
                let result = match receiver.await {
                    Ok(Ok(persisted)) => Ok(persisted),
                    Ok(Err(SnapshotError::MidSequence)) => Err(Skipped::MidSequence),
                    Ok(Err(reason)) => Err(Skipped::Failed(reason.to_string())),
                    Err(_) => Err(Skipped::Failed("the pane thread ended".into())),
                };
                let _ = sender.send(Message::Persistence(Done::Captured {
                    flush: id,
                    pane,
                    result,
                }));
            });
        }
        state.flushes.insert(
            id,
            Flush {
                captures,
                writes: BTreeSet::new(),
                layout: false,
                waiter,
                failed: None,
            },
        );
        self.advance_flush(id);
    }

    pub(super) fn write_layout(&mut self) {
        let state = &mut self.persistence;
        state.layout_busy = true;
        state.dirty = false;
        state.last_layout = Some(Instant::now());
        state.layout_waiters = std::mem::take(&mut state.next_layout_waiters);
        let document = Document::new(&self.workspace, Timestamp::now(), |id| {
            state.saved.contains(&id)
        });
        // A pane closed while its write is in flight keeps its folder until
        // that write finishes and the following layout write.
        let keep: BTreeSet<_> = self
            .workspace
            .panes
            .keys()
            .chain(state.writes.keys())
            .copied()
            .collect();
        let (store, enabled, sender) = (state.store.clone(), state.enabled, self.sender.clone());
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                store
                    .write_layout(&document, &keep, enabled)
                    .map_err(|reason| reason.to_string())
            })
            .await
            .unwrap_or_else(|reason| Err(reason.to_string()));
            let _ = sender.send(Message::Persistence(Done::Layout(result)));
        });
    }

    fn save_scrollback(&mut self, pane: PaneId, persisted: Persisted, waiters: Vec<u64>) {
        let state = &mut self.persistence;
        if !state.enabled || !self.workspace.panes.contains_key(&pane) {
            self.wrote(pane, waiters, None);
            return;
        }
        if let Some(write) = state.writes.get_mut(&pane) {
            // The newer capture supersedes one still queued.
            let mut waiters = waiters;
            if let Some((_, older)) = write.queued.take() {
                waiters.extend(older);
            }
            write.queued = Some((persisted, waiters));
            return;
        }
        state.writes.insert(
            pane,
            PaneWrite {
                waiters,
                queued: None,
            },
        );
        let (store, sender) = (state.store.clone(), self.sender.clone());
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let vt = persisted.into_vt().map_err(|reason| reason.to_string())?;
                let compressed = zstd::bulk::compress(&vt, zstd::DEFAULT_COMPRESSION_LEVEL)
                    .map_err(|reason| reason.to_string())?;
                store
                    .write_scrollback(pane, &compressed)
                    .map_err(|reason| reason.to_string())
            })
            .await
            .unwrap_or_else(|reason| Err(reason.to_string()));
            let _ = sender.send(Message::Persistence(Done::Scrollback { pane, result }));
        });
    }

    /// A pane's write for these flushes is over, or was skipped.
    fn wrote(&mut self, pane: PaneId, waiters: Vec<u64>, failed: Option<String>) {
        for flush in waiters {
            if let Some(state) = self.persistence.flushes.get_mut(&flush) {
                state.writes.remove(&pane);
                if let Some(reason) = &failed {
                    state.failed.get_or_insert_with(|| reason.clone());
                }
                self.advance_flush(flush);
            }
        }
    }

    fn advance_flush(&mut self, id: u64) {
        let state = &mut self.persistence;
        let Some(flush) = state.flushes.get_mut(&id) else {
            return;
        };
        if flush.layout || !flush.captures.is_empty() || !flush.writes.is_empty() {
            return;
        }
        flush.layout = true;
        state.next_layout_waiters.push(id);
        if !state.layout_busy {
            self.write_layout();
        }
    }

    fn finish_flush(&mut self, id: u64, layout_failed: Option<String>) {
        let Some(flush) = self.persistence.flushes.remove(&id) else {
            return;
        };
        let Some(reply) = flush.waiter else { return };
        let result = match layout_failed.or(flush.failed) {
            None => Ok(json!({})),
            Some(reason) => Err(error(
                error_code::INTERNAL,
                format!("could not save: {reason}"),
            )),
        };
        self.finish(reply, result);
    }
}
