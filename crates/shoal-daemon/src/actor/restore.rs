//! Restore on daemon start (M1 spec §6.4), before the actor serves anyone.
//! Reading, decompressing and starting shells run on blocking workers, all
//! panes in parallel.

use std::{path::PathBuf, sync::Arc};

use jiff::tz::TimeZone;
use shoal_daemon::{
    pane,
    persist::{self, Loaded, Restored, SavedPane},
};
use shoal_model::{PaneId, Timestamp};
use shoal_proto::{NoticeLevel, Notification};
use tokio::task::spawn_blocking;

use super::Actor;

/// A restored pane's shell, where it actually started, and whether its old
/// output was read from its scrollback file.
type Started = Result<(pane::Pane, PathBuf, bool), String>;

impl Actor {
    /// Seed the workspace from the last save. Never fails: anything unusable
    /// means starting empty, or without the panes that could not start.
    pub(super) async fn restore(&mut self) {
        let store = self.persistence.store().clone();
        // Reading the local time zone is file I/O too.
        let (loaded, zone) =
            spawn_blocking(move || (store.load(Timestamp::now()), TimeZone::system()))
                .await
                .unwrap_or_else(|error| {
                    let reason = error.to_string();
                    (
                        Loaded::Broken {
                            reason,
                            moved_to: None,
                        },
                        TimeZone::UTC,
                    )
                });
        match loaded {
            Loaded::Empty => {}
            Loaded::Broken { reason, moved_to } => {
                tracing::warn!(%reason, moved_to = ?moved_to, "the saved workspace is unusable; starting empty");
                let message = match moved_to.as_ref().and_then(|path| path.file_name()) {
                    Some(name) => format!(
                        "The saved workspace couldn't be restored and was moved to {}.",
                        name.to_string_lossy()
                    ),
                    None => "The saved workspace couldn't be restored.".into(),
                };
                self.restore_notices.push(notice(message));
            }
            Loaded::Restored(restored) => self.start_restored(restored, zone).await,
        }
    }

    async fn start_restored(&mut self, restored: Restored, zone: TimeZone) {
        let Restored {
            workspace,
            saved_at,
            panes,
        } = restored;
        self.workspace = workspace;
        self.next_id = self.workspace.next_id;
        let shared = Arc::new(Shared {
            saved_at,
            zone,
            persist: self.persistence.enabled(),
            store: self.persistence.store().clone(),
            settings: self.pane_settings.clone(),
            events: self.events.clone(),
        });
        let workers: Vec<_> = panes
            .into_iter()
            .map(|(id, saved)| {
                let shared = shared.clone();
                (id, spawn_blocking(move || shared.start(id, saved)))
            })
            .collect();
        let mut failed = Vec::new();
        for (id, worker) in workers {
            let started = worker
                .await
                .unwrap_or_else(|error| Err(format!("restore worker failed: {error}")));
            match started {
                Ok((runtime, cwd, loaded)) => {
                    self.panes.insert(id, runtime);
                    if let Some(record) = self.workspace.panes.get_mut(&id) {
                        record.cwd = Some(cwd);
                    }
                    if loaded {
                        self.persistence.restored(id);
                    }
                }
                Err(reason) => {
                    tracing::warn!(pane = %id, %reason, "could not restore a pane");
                    failed.push(id);
                }
            }
        }
        // Without a shell a pane can't exist; its tab or session may go too.
        for &id in &failed {
            self.close_pane(id);
        }
        if !failed.is_empty() {
            let count = failed.len();
            let panes = if count == 1 { "pane" } else { "panes" };
            self.restore_notices.push(notice(format!(
                "{count} saved {panes} couldn't be restored."
            )));
        }
        tracing::info!(panes = self.panes.len(), "restored the saved workspace");
        // Revision 1 is the first state anyone sees (step 7).
        self.refresh_metadata_titles();
        self.workspace.next_id = self.next_id;
        debug_assert!(
            shoal_model::validate(&self.workspace).is_ok(),
            "invalid workspace after restore"
        );
        self.observe_persistence();
    }
}

/// What every restored pane's worker shares.
struct Shared {
    saved_at: Timestamp,
    zone: TimeZone,
    persist: bool,
    store: persist::Store,
    settings: pane::PaneSettings,
    events: pane::Events,
}

impl Shared {
    /// One pane's blocking half: its old output, its divider and its shell.
    fn start(&self, id: PaneId, saved: SavedPane) -> Started {
        // With saving off, old output stays hidden; the next layout write
        // deletes it (§6.3).
        let (output, unreadable) = match (self.persist, &saved.scrollback) {
            (true, Some(_)) => match self.store.read_scrollback(id) {
                Ok(bytes) => (Some(bytes), false),
                Err(error) => {
                    tracing::warn!(pane = %id, %error, "skipped unreadable scrollback");
                    (None, true)
                }
            },
            _ => (None, false),
        };
        let folder_gone = saved.cwd.as_ref().is_some_and(|cwd| !cwd.is_dir());
        let divider = persist::divider(
            self.saved_at,
            &self.zone,
            saved.program.as_deref(),
            folder_gone,
            unreadable,
        );
        let cwd = pane::spawn::directory(saved.cwd.as_deref(), &self.settings.home);
        let loaded = output.is_some();
        let spec = pane::PaneSpec {
            id,
            size: saved.size,
            argv: Vec::new(),
            cwd: Some(cwd.clone()),
            window_env: Default::default(),
            restore: Some(pane::Restore {
                saved: output.unwrap_or_default(),
                divider,
            }),
        };
        let runtime = pane::spawn(spec, &self.settings, self.events.clone())
            .map_err(|error| error.to_string())?;
        Ok((runtime, cwd, loaded))
    }
}

fn notice(message: String) -> Notification {
    Notification::DaemonNotice {
        level: NoticeLevel::Warn,
        message,
        key: None,
    }
}
