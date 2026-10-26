//! Event-driven scheduling, live settings, pane exits, and bounded shutdown.

use std::{
    collections::{BTreeSet, VecDeque},
    future::pending,
    time::Duration,
};

use shoal_daemon::{pane, settings};
use shoal_model::{PaneId, PaneStatus};
use shoal_proto::ClientKind;
use tokio::sync::oneshot;

use super::{Actor, Connection, Message, Registration};

impl Actor {
    pub(super) async fn run(mut self) {
        // Native watchers queue their initial problems before start returns.
        // Retain those before accepting clients, so startup replay is GUI-only.
        while let Ok(update) = self.updates.try_recv() {
            self.update(update);
        }
        loop {
            while let Some(id) = self.ready.pop_front() {
                let Some(connection) = self.connections.get_mut(&id) else {
                    continue;
                };
                if connection.busy {
                    continue;
                }
                if let Some((request, done)) = connection.requests.pop_front() {
                    connection.busy = true;
                    self.request(id, request, done);
                }
            }
            let deadline = self.notification_deadline();
            tokio::select! {
                message = self.receiver.recv() => match message {
                    Some(Message::Connect { client, env, core, outgoing, reply }) => {
                        let id = self.next_connection;
                        self.next_connection += 1;
                        if client == ClientKind::Gui {
                            self.window_env = env.into_iter().filter(|(key, _)| pane::spawn::WINDOW_ENV.contains(&key.as_str())).collect();
                        }
                        self.connections.insert(id, Connection {
                            outgoing, core, attachments: Default::default(), shown: None, focused: false, reservations: BTreeSet::new(), subscribed: false,
                            dirty: false, last_notification: None, busy: false, requests: VecDeque::new(),
                        });
                        let registration = Registration { id, sender: self.sender.clone(), active: true };
                        if reply.send(registration).is_ok() && client == ClientKind::Gui {
                            for notice in self.notices.clone() { self.notify(id, &notice); }
                        }
                    }
                    Some(Message::Request { connection, request, done }) => {
                        if let Some(state) = self.connections.get_mut(&connection) {
                            state.requests.push_back((request, done));
                            if !state.busy { self.ready.push_back(connection); }
                        }
                    }
                    Some(Message::Disconnect(id)) => self.disconnect(id),
                    Some(Message::Input { connection, pane, bytes, done }) => {
                        if self.connections.contains_key(&connection)
                            && let Some(runtime) = self.panes.get(&pane) { runtime.write(bytes); }
                        let _ = done.send(());
                    }
                    Some(Message::Attached { reply, pane, result }) => self.attached(reply, pane, result),
                    Some(Message::Sized { reply, pane, result }) => self.sized(reply, pane, result),
                    Some(Message::Spawned { pane, result }) => self.spawned(pane, result),
                    Some(Message::Captured { reply, result }) => { self.finish(reply, result); }
                    Some(Message::Stop(reply)) => {
                        self.stop(reply).await;
                        return;
                    }
                    None => return,
                },
                Some((id, event)) = self.event_receiver.recv() => self.event(id, event),
                update = self.updates.recv(), if self.updates_open => match update {
                    Some(update) => self.update(update),
                    None => self.updates_open = false,
                },
                () = async { match deadline { Some(deadline) => tokio::time::sleep_until(deadline).await, None => pending::<()>().await } } => self.flush_notifications(),
            }
        }
    }
    fn update(&mut self, update: settings::Update) {
        self.notices = update.notices;
        self.settings = update.settings;
        self.pane_settings.shell_program = (!self.settings.config.shell.program.is_empty())
            .then(|| self.settings.config.shell.program.clone());
        self.pane_settings.shell_integration = self.settings.config.shell.integration;
        self.pane_settings.prompt_mode = self.settings.config.prompt.mode;
        self.pane_settings.scrollback = self.settings.config.scrollback.lines;
        self.pane_settings.theme = settings::theme(&self.settings.palette);
        for pane in self.panes.values() {
            pane.apply_settings(self.pane_settings.theme, self.pane_settings.scrollback);
        }
        if self.refresh_metadata_titles() {
            self.changed();
        }
        let ids: Vec<_> = self.connections.keys().copied().collect();
        for notice in self.notices.clone() {
            for &id in &ids {
                self.notify(id, &notice);
            }
        }
    }

    pub(super) fn event(&mut self, id: PaneId, event: pane::Event) {
        if self.pending.contains_key(&id) {
            let events = self.early_events.entry(id).or_default();
            let mut event = event;
            // Keep just the newest metadata and any exit while a blocking
            // spawn finishes. Never lose an initial directory/title/bell.
            if let pane::Event::Metadata(value) = &mut event {
                for old in events.iter() {
                    if let pane::Event::Metadata(old) = old {
                        value.bell = value.bell.max(old.bell);
                    }
                }
                events.retain(|old| !matches!(old, pane::Event::Metadata(_)));
            }
            events.push(event);
            return;
        }
        if matches!(&event, pane::Event::Exited { .. }) {
            self.retiring.remove(&id);
        }
        if let pane::Event::Metadata(value) = event {
            self.metadata(id, value);
        } else if let pane::Event::Exited {
            code,
            signal,
            early_failure,
        } = event
            && let Some(record) = self.workspace.panes.get_mut(&id)
        {
            record.status = PaneStatus::Exited { code, signal };
            record.program = None;
            if self.settings.config.behavior.close_pane_on_exit && !early_failure {
                self.close_pane(id);
            }
            self.changed();
        }
    }

    async fn stop(&mut self, reply: oneshot::Sender<()>) {
        let started = std::time::Instant::now();
        let cutoff = *self
            .shutdown_cutoff
            .get_or_init(|| started + Duration::from_secs(2));
        let mut waiters = vec![reply];
        self.connections.clear();
        let mut running: BTreeSet<_> = self
            .workspace
            .panes
            .iter()
            .filter(|(_, pane)| pane.status == PaneStatus::Running)
            .map(|(&id, _)| id)
            .chain(self.retiring.keys().copied())
            .collect();
        let mut exited: BTreeSet<_> = self
            .early_events
            .iter()
            .filter(|(_, events)| {
                events
                    .iter()
                    .any(|e| matches!(e, pane::Event::Exited { .. }))
            })
            .map(|(&id, _)| id)
            .collect();
        for pane in self.panes.values().chain(self.retiring.values()) {
            drop(pane.kill_by(cutoff));
        }
        let deadline = tokio::time::sleep_until(tokio::time::Instant::from_std(
            started + Duration::from_millis(2500),
        ));
        tokio::pin!(deadline);
        while !running.is_empty() || !self.pending.is_empty() {
            tokio::select! {
                () = &mut deadline => break,
                Some((id, event)) = self.event_receiver.recv() => {
                    if matches!(event, pane::Event::Exited { .. }) {
                        running.remove(&id);
                        exited.insert(id);
                        self.retiring.remove(&id);
                    }
                }
                message = self.receiver.recv() => match message {
                    Some(Message::Spawned { pane: id, result }) => {
                        self.pending.remove(&id);
                        if let Ok((pane, _)) = result {
                            drop(pane.kill_by(cutoff));
                            if !exited.contains(&id) { running.insert(id); }
                            self.panes.insert(id, pane);
                        }
                    }
                    Some(Message::Stop(other)) => waiters.push(other),
                    Some(_) => {}
                    None => break,
                },
            }
        }
        self.panes.clear();
        self.retiring.clear();
        self.pending.clear();
        for reply in waiters {
            let _ = reply.send(());
        }
    }
}
