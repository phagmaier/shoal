//! The single owner of the workspace and all runtime pane handles.

mod driver;
mod geometry;
mod metadata;
mod methods;
mod persistence;
mod spawning;
mod streaming;
mod workspace;

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, OnceLock},
    time::Duration,
};

use serde_json::{Value, json};
use shoal_config::XdgPaths;
use shoal_daemon::{
    pane,
    settings::{self, Settings},
};
use shoal_model::{PaneId, SessionId, Workspace};
use shoal_proto::{
    ClientKind, Frame, Notification, RawNotification, Request, RequestId, Response, RpcError,
    error_code,
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    time::Instant,
};

pub type ConnectionId = u64;
pub struct Outgoing {
    pub frames: mpsc::Sender<OutgoingFrame>,
    pub streams: Arc<tokio::sync::Notify>,
    pub close: watch::Sender<bool>,
}

pub enum OutgoingFrame {
    Wire(Arc<[u8]>),
    Attach {
        pane: PaneId,
        snapshot: Arc<[u8]>,
        response: Arc<[u8]>,
        items: pane::Receiver,
    },
    Detach(PaneId),
}

#[derive(Clone)]
pub struct Handle {
    sender: mpsc::UnboundedSender<Message>,
}

impl Handle {
    pub async fn connect(
        &self,
        client: ClientKind,
        env: BTreeMap<String, String>,
        core: Option<String>,
        outgoing: Outgoing,
    ) -> ConnectionId {
        let (reply, result) = oneshot::channel();
        let _ = self.sender.send(Message::Connect {
            client,
            env,
            core,
            outgoing,
            reply,
        });
        match result.await {
            Ok(mut registration) => {
                registration.active = false;
                registration.id
            }
            Err(_) => 0,
        }
    }

    pub async fn request(&self, connection: ConnectionId, request: Request) {
        let (done, result) = oneshot::channel();
        let _ = self.sender.send(Message::Request {
            connection,
            request,
            done,
        });
        let _ = result.await;
    }

    pub async fn input(&self, connection: ConnectionId, pane: PaneId, bytes: Vec<u8>) {
        let (done, result) = oneshot::channel();
        let _ = self.sender.send(Message::Input {
            connection,
            pane,
            bytes,
            done,
        });
        let _ = result.await;
    }

    pub fn disconnect(&self, connection: ConnectionId) {
        let _ = self.sender.send(Message::Disconnect(connection));
    }

    pub async fn stop(&self) {
        let (reply, result) = oneshot::channel();
        let _ = self.sender.send(Message::Stop(reply));
        let _ = result.await;
    }
}

pub fn start(
    paths: XdgPaths,
    settings: Settings,
    updates: mpsc::UnboundedReceiver<settings::Update>,
    shutdown: watch::Sender<bool>,
) -> Handle {
    let (sender, receiver) = mpsc::unbounded_channel();
    let (events, event_receiver) = mpsc::unbounded_channel();
    let actor = Actor {
        workspace: Workspace {
            rev: 0,
            next_id: 1,
            sessions: Vec::new(),
            panes: BTreeMap::new(),
        },
        next_id: 1,
        next_connection: 1,
        size_owner: None,
        size_sequences: BTreeMap::new(),
        connections: BTreeMap::new(),
        ready: VecDeque::new(),
        panes: BTreeMap::new(),
        retiring: BTreeMap::new(),
        pending: BTreeMap::new(),
        shutdown_cutoff: Arc::new(OnceLock::new()),
        early_events: BTreeMap::new(),
        osc_titles: BTreeMap::new(),
        focused_panes: BTreeSet::new(),
        last_pane_focus: BTreeMap::new(),
        window_env: BTreeMap::new(),
        persistence: persistence::Persistence::new(
            shoal_daemon::persist::Store::new(&paths),
            settings.config.scrollback.persist,
        ),
        pane_settings: settings.pane(
            &paths,
            paths
                .socket_file()
                .expect("invariant: startup validated runtime path"),
            std::env::vars_os().collect(),
        ),
        settings,
        updates,
        updates_open: true,
        notices: Vec::new(),
        events,
        event_receiver,
        sender: sender.clone(),
        receiver,
        shutdown,
        started: Instant::now(),
    };
    tokio::spawn(actor.run());
    Handle { sender }
}

struct Connection {
    outgoing: Outgoing,
    core: Option<String>,
    attachments: BTreeMap<PaneId, pane::SubscriberId>,
    shown: Option<SessionId>,
    focused: bool,
    reservations: BTreeSet<SessionId>,
    subscribed: bool,
    dirty: bool,
    last_notification: Option<Instant>,
    busy: bool,
    requests: VecDeque<(Request, oneshot::Sender<()>)>,
}

// An id may be delivered just before its connect future is cancelled by EOF.
// Carry cleanup with the delivery itself until the caller takes ownership.
struct Registration {
    id: ConnectionId,
    sender: mpsc::UnboundedSender<Message>,
    active: bool,
}

impl Drop for Registration {
    fn drop(&mut self) {
        if self.active {
            let _ = self.sender.send(Message::Disconnect(self.id));
        }
    }
}

struct Reply {
    connection: ConnectionId,
    id: RequestId,
    done: oneshot::Sender<()>,
}

enum Message {
    Connect {
        client: ClientKind,
        env: BTreeMap<String, String>,
        core: Option<String>,
        outgoing: Outgoing,
        reply: oneshot::Sender<Registration>,
    },
    Request {
        connection: ConnectionId,
        request: Request,
        done: oneshot::Sender<()>,
    },
    Disconnect(ConnectionId),
    Input {
        connection: ConnectionId,
        pane: PaneId,
        bytes: Vec<u8>,
        done: oneshot::Sender<()>,
    },
    Sized {
        reply: Reply,
        pane: PaneId,
        result: Result<(u64, shoal_model::GridSize), String>,
    },
    Attached {
        reply: Reply,
        pane: PaneId,
        result: Result<pane::Attachment, RpcError>,
    },
    Spawned {
        pane: PaneId,
        result: Result<(pane::Pane, std::path::PathBuf), String>,
    },
    Captured {
        reply: Reply,
        result: Result<Value, RpcError>,
    },
    Persistence(persistence::Done),
    Stop(oneshot::Sender<()>),
}

struct Actor {
    workspace: Workspace,
    next_id: u64,
    next_connection: u64,
    size_owner: Option<ConnectionId>,
    size_sequences: BTreeMap<PaneId, u64>,
    connections: BTreeMap<ConnectionId, Connection>,
    ready: VecDeque<ConnectionId>,
    panes: BTreeMap<PaneId, pane::Pane>,
    retiring: BTreeMap<PaneId, pane::Pane>,
    pending: BTreeMap<PaneId, spawning::Spawn>,
    shutdown_cutoff: Arc<OnceLock<std::time::Instant>>,
    early_events: BTreeMap<PaneId, Vec<pane::Event>>,
    osc_titles: BTreeMap<PaneId, String>,
    focused_panes: BTreeSet<PaneId>,
    last_pane_focus: BTreeMap<PaneId, std::time::Instant>,
    window_env: BTreeMap<String, String>,
    persistence: persistence::Persistence,
    pane_settings: pane::PaneSettings,
    settings: Settings,
    updates: mpsc::UnboundedReceiver<settings::Update>,
    updates_open: bool,
    notices: Vec<Notification>,
    events: pane::Events,
    event_receiver: mpsc::UnboundedReceiver<(PaneId, pane::Event)>,
    sender: mpsc::UnboundedSender<Message>,
    receiver: mpsc::UnboundedReceiver<Message>,
    shutdown: watch::Sender<bool>,
    started: Instant,
}

impl Actor {
    fn finish(&mut self, reply: Reply, result: Result<Value, RpcError>) -> bool {
        let response = match result {
            Ok(result) => Response::Success {
                id: reply.id.clone(),
                result,
            },
            Err(error) => Response::Error {
                id: Some(reply.id.clone()),
                error,
            },
        };
        let frame = encode(&response).or_else(|_| {
            encode(&Response::<Value>::Error {
                id: match &response {
                    Response::Success { id, .. } => Some(id.clone()),
                    Response::Error { id, .. } => id.clone(),
                },
                error: error(
                    error_code::INTERNAL,
                    "response exceeds protocol frame limit",
                ),
            })
        });
        let enqueued = if let Ok(frame) = frame {
            self.send(reply.connection, frame)
        } else {
            self.disconnect(reply.connection);
            false
        };
        self.release(reply);
        enqueued
    }

    fn release(&mut self, reply: Reply) {
        let _ = reply.done.send(());
        if let Some(connection) = self.connections.get_mut(&reply.connection) {
            connection.busy = false;
            if !connection.requests.is_empty() {
                self.ready.push_back(reply.connection);
            }
        }
    }

    fn send(&mut self, id: ConnectionId, frame: Arc<[u8]>) -> bool {
        self.send_frame(id, OutgoingFrame::Wire(frame))
    }

    fn send_frame(&mut self, id: ConnectionId, frame: OutgoingFrame) -> bool {
        let enqueued = self
            .connections
            .get(&id)
            .is_some_and(|state| state.outgoing.frames.try_send(frame).is_ok());
        if !enqueued {
            self.disconnect(id);
        }
        enqueued
    }

    fn notify(&mut self, id: ConnectionId, notification: &Notification) {
        match RawNotification::from_notification(notification)
            .map_err(|e| e.to_string())
            .and_then(|raw| encode(&raw))
        {
            Ok(frame) => {
                self.send(id, frame);
            }
            Err(reason) => {
                tracing::warn!(%reason, "cannot encode notification; disconnecting client");
                self.disconnect(id);
            }
        }
    }

    fn changed(&mut self) {
        self.refresh_metadata_titles();
        self.clear_focused_bells();
        self.workspace.next_id = self.next_id;
        self.workspace.rev += 1;
        debug_assert!(
            shoal_model::validate(&self.workspace).is_ok(),
            "invalid workspace after actor change"
        );
        self.observe_persistence();
        for connection in self.connections.values_mut() {
            connection.dirty |= connection.subscribed;
        }
    }

    fn notification_deadline(&self) -> Option<Instant> {
        self.connections
            .values()
            .filter(|connection| connection.dirty)
            .map(|connection| {
                connection
                    .last_notification
                    .map_or_else(Instant::now, |last| last + Duration::from_millis(16))
            })
            .min()
    }

    fn flush_notifications(&mut self) {
        let now = Instant::now();
        let ids: Vec<_> = self
            .connections
            .iter()
            .filter(|(_, c)| {
                c.dirty
                    && c.last_notification
                        .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(16))
            })
            .map(|(&id, _)| id)
            .collect();
        let frame = encode(&WorkspaceNotification {
            jsonrpc: "2.0",
            method: "workspace.changed",
            params: WorkspaceView {
                rev: self.workspace.rev,
                workspace: &self.workspace,
            },
        });
        for &id in &ids {
            if let Some(connection) = self.connections.get_mut(&id) {
                connection.dirty = false;
            }
        }
        // A failed send can disconnect a window and dirty every subscriber
        // again. Do not clear those newer changes while sending this revision.
        for id in ids {
            if let Some(connection) = self.connections.get_mut(&id) {
                connection.last_notification = Some(Instant::now());
            }
            match &frame {
                Ok(frame) => {
                    self.send(id, frame.clone());
                }
                Err(reason) => {
                    tracing::warn!(%reason, "workspace notification exceeds protocol limits");
                    self.disconnect(id);
                }
            }
        }
    }

    fn disconnect(&mut self, id: ConnectionId) {
        if self.size_owner == Some(id) {
            self.size_owner = None;
        }
        if let Some(connection) = self.connections.remove(&id) {
            let _ = connection.outgoing.close.send(true);
            for (pane, subscriber) in connection.attachments {
                if let Some(runtime) = self.panes.get(&pane) {
                    runtime.unsubscribe(subscriber);
                }
            }
            if let Some(session) = connection.shown
                && let Some(index) = self.session_index(session)
            {
                self.workspace.sessions[index].windows -= 1;
                self.changed();
            }
        }
    }
}

#[derive(serde::Serialize)]
struct WorkspaceNotification<'a> {
    jsonrpc: &'static str,
    method: &'static str,
    params: WorkspaceView<'a>,
}

#[derive(serde::Serialize)]
struct WorkspaceView<'a> {
    rev: u64,
    workspace: &'a Workspace,
}

fn error(code: i32, message: impl Into<String>) -> RpcError {
    RpcError {
        code,
        message: message.into(),
        data: None,
    }
}

fn encode(value: &impl serde::Serialize) -> Result<Arc<[u8]>, String> {
    let Value::Object(object) = serde_json::to_value(value).map_err(|e| e.to_string())? else {
        return Err("protocol envelope must be an object".into());
    };
    Frame::Json(object)
        .encode()
        .map(Arc::from)
        .map_err(|e| e.to_string())
}
