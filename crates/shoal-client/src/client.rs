//! The public handle: blocking calls and `std` channels over the background
//! worker.

use std::{
    sync::mpsc::{self, Receiver},
    time::Duration,
};

use serde_json::Value;
use shoal_model::PaneId;
use shoal_proto::{Call, HelloResult, Notification};
use tokio::sync::mpsc::UnboundedSender;

use crate::{AttachStream, AutoStart, Detach, Error, Options, StreamItem, worker};

/// What the connection did, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A reconnect's `hello` succeeded (the first one is in [`Connected`]).
    /// Workspace subscriptions and attach streams resume by themselves.
    Connected(HelloResult),
    /// The connection ended. With `reconnect` on, the client is retrying;
    /// requests fail with [`Error::NotConnected`] until the next
    /// [`Event::Connected`]. After a refused `hello` it stops retrying.
    Disconnected(Error),
    /// `workspace.changed` and `daemon.notice`. `pane.detached` goes to its
    /// stream instead.
    Notification(Notification),
}

/// A client connected for the first time.
#[derive(Debug)]
pub struct Connected {
    pub client: Client,
    pub hello: HelloResult,
    /// Every [`Event`] after the first `hello`.
    pub events: Receiver<Event>,
}

/// A connection to `shoald`. The background thread ends once the client and
/// every [`AttachStream`] are dropped.
#[derive(Debug)]
pub struct Client {
    commands: UnboundedSender<Command>,
    timeout: Duration,
}

pub(crate) enum Command {
    Request {
        call: Call,
        timeout: Duration,
        reply: mpsc::Sender<Result<Value, Error>>,
    },
    Subscribe {
        reply: mpsc::Sender<Result<Value, Error>>,
    },
    Input {
        pane: PaneId,
        bytes: Vec<u8>,
    },
    Attach {
        pane: PaneId,
        stream: u64,
        items: mpsc::Sender<StreamItem>,
    },
    Detach {
        pane: PaneId,
        stream: u64,
    },
}

impl Client {
    /// Connect and say `hello`, blocking until it is answered. If the socket
    /// is not there, `auto_start` runs once and the client waits up to 2 s
    /// for the daemon. A refused `hello` is an [`Error::Rpc`].
    pub fn connect(options: Options, auto_start: impl AutoStart) -> Result<Connected, Error> {
        let timeout = options.request_timeout;
        let (commands, hello, events) = worker::spawn(options, Box::new(auto_start))?;
        Ok(Connected {
            client: Self { commands, timeout },
            hello,
            events,
        })
    }

    /// Send `call` and wait for its result.
    pub fn request(&self, call: Call) -> Result<Value, Error> {
        self.send(call).recv().unwrap_or(Err(Error::Stopped))
    }

    /// Send `call` without waiting. Exactly one result arrives on the
    /// channel: the answer, [`Error::Timeout`] after the request timeout, or
    /// [`Error::NotConnected`].
    pub fn send(&self, call: Call) -> Receiver<Result<Value, Error>> {
        self.send_with_timeout(call, self.timeout)
    }

    pub fn send_with_timeout(
        &self,
        call: Call,
        timeout: Duration,
    ) -> Receiver<Result<Value, Error>> {
        let (reply, result) = mpsc::channel();
        self.command(
            Command::Request {
                call,
                timeout,
                reply: reply.clone(),
            },
            &reply,
        );
        result
    }

    /// `workspace.subscribe`: `workspace.changed` arrives as events, again
    /// after every reconnect.
    pub fn subscribe_workspace(&self) -> Result<(), Error> {
        let (reply, result) = mpsc::channel();
        self.command(
            Command::Subscribe {
                reply: reply.clone(),
            },
            &reply,
        );
        result.recv().unwrap_or(Err(Error::Stopped)).map(drop)
    }

    /// Keystrokes or a paste for a pane, already encoded (§5.6). Dropped
    /// while disconnected.
    pub fn input(&self, pane: PaneId, bytes: Vec<u8>) {
        let _ = self.commands.send(Command::Input { pane, bytes });
    }

    /// Stream a pane. A second stream for the same pane replaces the first,
    /// which ends.
    pub fn attach(&self, pane: PaneId) -> AttachStream {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (items, receiver) = mpsc::channel();
        let _ = self.commands.send(Command::Attach {
            pane,
            stream: id,
            items,
        });
        AttachStream {
            items: receiver,
            detach: Detach {
                pane,
                id,
                commands: self.commands.clone(),
            },
        }
    }

    fn command(&self, command: Command, reply: &mpsc::Sender<Result<Value, Error>>) {
        if self.commands.send(command).is_err() {
            let _ = reply.send(Err(Error::Stopped));
        }
    }
}
