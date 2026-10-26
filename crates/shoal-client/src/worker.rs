//! The client's background thread: one tokio runtime that owns the socket,
//! matches responses to requests, routes pane frames to their streams and
//! reconnects. Timers exist only while a request waits or a retry is due.

use std::{
    collections::BTreeMap,
    os::unix::net::UnixStream as StdStream,
    sync::mpsc::{self, Receiver, SyncSender},
    time::Duration,
};

use serde_json::Value;
use shoal_model::PaneId;
use shoal_proto::{
    Call, Empty, Frame, HelloResult, Input, Notification, PROTOCOL_VERSION, Request, RequestId,
};
use tokio::{
    io::AsyncWriteExt as _,
    net::{UnixStream, unix::OwnedReadHalf},
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    time::Instant,
};

use crate::{
    AutoStart, Error, Event, Options, StreamItem,
    backoff::Backoff,
    client::Command,
    options::{START_WAIT, connect_within},
    stream::Tracker,
};

mod incoming;

type Started = (UnboundedSender<Command>, HelloResult, Receiver<Event>);

/// Start the thread and wait for the first `hello`.
pub(crate) fn spawn(options: Options, auto_start: Box<dyn AutoStart>) -> Result<Started, Error> {
    let (commands, receiver) = unbounded_channel();
    let (events, events_receiver) = mpsc::channel();
    let (ready, started) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("shoal-client".into())
        .spawn(move || run(options, auto_start, receiver, events, ready))
        .map_err(|error| Error::Connect(error.to_string()))?;
    let hello = started.recv().unwrap_or(Err(Error::Stopped))?;
    Ok((commands, hello, events_receiver))
}

fn run(
    options: Options,
    auto_start: Box<dyn AutoStart>,
    commands: UnboundedReceiver<Command>,
    events: mpsc::Sender<Event>,
    ready: SyncSender<Result<HelloResult, Error>>,
) {
    let stream = match connect_first(&options, auto_start) {
        Ok(stream) => stream,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(Error::Connect(error.to_string())));
            return;
        }
    };
    runtime.block_on(async move {
        let mut worker = Worker {
            options,
            commands,
            events,
            ready: Some(ready),
            finished: false,
            link: None,
            retry_at: None,
            backoff: Backoff::default(),
            next_id: 1,
            pending: BTreeMap::new(),
            attachments: BTreeMap::new(),
            subscribed: false,
        };
        match stream
            .set_nonblocking(true)
            .and_then(|()| UnixStream::from_std(stream))
        {
            Ok(stream) => worker.establish(stream),
            Err(error) => worker.lose(Error::Connect(error.to_string())),
        }
        worker.run().await;
    });
}

/// The first connection: auto-start once if nothing is listening.
fn connect_first(
    options: &Options,
    mut auto_start: Box<dyn AutoStart>,
) -> Result<StdStream, Error> {
    let first = match StdStream::connect(&options.socket) {
        Ok(stream) => return Ok(stream),
        Err(error) => error,
    };
    tracing::info!(socket = %options.socket.display(), error = %first, "starting shoald");
    auto_start
        .start(&options.socket)
        .map_err(|error| Error::Connect(format!("{first}; auto-start failed: {error}")))?;
    connect_within(&options.socket, START_WAIT).map_err(|error| Error::Connect(error.to_string()))
}

struct Worker {
    options: Options,
    commands: UnboundedReceiver<Command>,
    events: mpsc::Sender<Event>,
    /// Until the first `hello` is answered: where its result goes.
    ready: Option<SyncSender<Result<HelloResult, Error>>>,
    /// The first connection failed; the thread ends.
    finished: bool,
    link: Option<Link>,
    retry_at: Option<Instant>,
    backoff: Backoff,
    next_id: u64,
    pending: BTreeMap<u64, Pending>,
    attachments: BTreeMap<PaneId, Attachment>,
    subscribed: bool,
}

struct Link {
    reader: OwnedReadHalf,
    buffer: Vec<u8>,
    writer: UnboundedSender<Vec<u8>>,
    /// The `hello` still unanswered; nothing else is sent until it is.
    hello: Option<u64>,
    /// Notifications that arrived before `hello`'s answer.
    held: Vec<Notification>,
}

struct Pending {
    deadline: Instant,
    timeout: Duration,
    reply: Reply,
}

enum Reply {
    Caller(mpsc::Sender<Result<Value, Error>>),
    Hello,
    /// `workspace.subscribe`, from the caller or resumed after a reconnect.
    Subscribe(Option<mpsc::Sender<Result<Value, Error>>>),
    Attach {
        pane: PaneId,
        stream: u64,
    },
    Ignore,
}

struct Attachment {
    stream: u64,
    items: mpsc::Sender<StreamItem>,
    tracker: Tracker,
}

impl Worker {
    async fn run(&mut self) {
        while !self.finished {
            let deadline = self.pending.values().map(|pending| pending.deadline).min();
            let retry_at = self.retry_at;
            tokio::select! {
                command = self.commands.recv() => match command {
                    Some(command) => self.command(command),
                    None => return,
                },
                read = incoming::read(&mut self.link), if self.link.is_some() => self.read(read),
                () = sleep_until(deadline), if deadline.is_some() => self.expire(),
                () = sleep_until(retry_at), if retry_at.is_some() => self.retry().await,
            }
        }
    }

    fn connected(&self) -> bool {
        self.link.as_ref().is_some_and(|link| link.hello.is_none())
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Request {
                call,
                timeout,
                reply,
            } => {
                if self.connected() {
                    self.send(&call, timeout, Reply::Caller(reply));
                } else {
                    let _ = reply.send(Err(Error::NotConnected));
                }
            }
            Command::Subscribe { reply } => {
                if self.connected() {
                    let call = Call::WorkspaceSubscribe(Empty {});
                    let timeout = self.options.request_timeout;
                    self.send(&call, timeout, Reply::Subscribe(Some(reply)));
                } else {
                    let _ = reply.send(Err(Error::NotConnected));
                }
            }
            Command::Input { pane, bytes } => {
                if self.connected() {
                    match (Frame::Input(Input {
                        pane,
                        bytes: &bytes,
                    }))
                    .encode()
                    {
                        Ok(frame) => self.write(frame),
                        Err(error) => tracing::warn!(%pane, %error, "input not sent"),
                    }
                }
            }
            Command::Attach {
                pane,
                stream,
                items,
            } => {
                // Without a connection or a retry coming, it can never attach.
                if !self.connected() && self.link.is_none() && self.retry_at.is_none() {
                    let _ = items.send(StreamItem::Failed(Error::NotConnected));
                    return;
                }
                let attachment = Attachment {
                    stream,
                    items,
                    tracker: Tracker::default(),
                };
                // Replacing an attachment drops its sender, which ends it.
                self.attachments.insert(pane, attachment);
                if self.connected() {
                    self.attach(pane, stream);
                }
            }
            Command::Detach { pane, stream } => {
                if self
                    .attachments
                    .get(&pane)
                    .is_some_and(|attachment| attachment.stream == stream)
                {
                    self.attachments.remove(&pane);
                    if self.connected() {
                        let timeout = self.options.request_timeout;
                        self.send(&Call::PaneDetach { pane }, timeout, Reply::Ignore);
                    }
                }
            }
        }
    }

    fn attach(&mut self, pane: PaneId, stream: u64) {
        let timeout = self.options.request_timeout;
        self.send(
            &Call::PaneAttach { pane },
            timeout,
            Reply::Attach { pane, stream },
        );
    }

    /// Number, encode and write a request, and remember who wants the answer.
    fn send(&mut self, call: &Call, timeout: Duration, reply: Reply) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let frame = Request::from_call(RequestId::from(id), call)
            .and_then(|request| serde_json::to_value(&request))
            .map_err(|error| error.to_string())
            .and_then(|value| match value {
                Value::Object(object) => Frame::Json(object).encode().map_err(|e| e.to_string()),
                _ => Err("a request must be an object".into()),
            });
        match frame {
            Ok(frame) => self.write(frame),
            Err(reason) => {
                self.resolve(
                    reply,
                    Err(Error::Protocol(format!("cannot send: {reason}"))),
                );
                return id;
            }
        }
        let deadline = Instant::now() + timeout;
        self.pending.insert(
            id,
            Pending {
                deadline,
                timeout,
                reply,
            },
        );
        id
    }

    fn write(&mut self, frame: Vec<u8>) {
        if let Some(link) = &self.link {
            let _ = link.writer.send(frame);
        }
    }

    /// A new socket: start the writer and say `hello`.
    fn establish(&mut self, stream: UnixStream) {
        let (reader, mut writer) = stream.into_split();
        let (frames, mut queue) = unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            while let Some(frame) = queue.recv().await {
                if writer.write_all(&frame).await.is_err() {
                    return;
                }
            }
        });
        self.link = Some(Link {
            reader,
            buffer: Vec::new(),
            writer: frames,
            hello: None,
            held: Vec::new(),
        });
        let hello = Call::Hello {
            protocol: PROTOCOL_VERSION,
            client: self.options.client,
            version: self.options.version.clone(),
            core: self.options.core.clone(),
            env: self.options.env.clone(),
        };
        let id = self.send(&hello, self.options.request_timeout, Reply::Hello);
        if let Some(link) = &mut self.link {
            link.hello = Some(id);
        }
    }

    async fn retry(&mut self) {
        self.retry_at = None;
        match UnixStream::connect(&self.options.socket).await {
            Ok(stream) => self.establish(stream),
            Err(error) => {
                tracing::debug!(%error, "shoald is not back yet");
                self.retry_at = Some(Instant::now() + self.backoff.next());
            }
        }
    }

    /// The connection is gone. Answer everything waiting on it, then retry
    /// unless reconnecting is off or `hello` was refused.
    fn lose(&mut self, reason: Error) {
        let refused = matches!(reason, Error::Rpc(_));
        self.link = None;
        for (_, pending) in std::mem::take(&mut self.pending) {
            match pending.reply {
                // Streams wait for the next connection instead.
                Reply::Attach { .. } | Reply::Hello => {}
                reply => self.resolve(reply, Err(Error::NotConnected)),
            }
        }
        for attachment in self.attachments.values_mut() {
            attachment.tracker.reset();
        }
        if let Some(ready) = self.ready.take() {
            let _ = ready.send(Err(reason));
            self.finished = true;
            return;
        }
        tracing::info!(%reason, "lost shoald");
        let _ = self.events.send(Event::Disconnected(reason));
        if self.options.reconnect && !refused {
            self.retry_at = Some(Instant::now() + self.backoff.next());
        } else {
            for (_, attachment) in std::mem::take(&mut self.attachments) {
                let _ = attachment
                    .items
                    .send(StreamItem::Failed(Error::NotConnected));
            }
        }
    }

    fn resolve(&mut self, reply: Reply, result: Result<Value, Error>) {
        match reply {
            Reply::Caller(reply) | Reply::Subscribe(Some(reply)) => {
                let _ = reply.send(result);
            }
            Reply::Attach { pane, stream } => {
                if let Err(error) = result {
                    self.attach_failed(pane, stream, error);
                }
            }
            Reply::Hello | Reply::Subscribe(None) | Reply::Ignore => {}
        }
    }

    fn expire(&mut self) {
        let now = Instant::now();
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.deadline <= now)
            .map(|(&id, _)| id)
            .collect();
        for id in expired {
            let Some(pending) = self.pending.remove(&id) else {
                continue;
            };
            let error = Error::Timeout(pending.timeout);
            match pending.reply {
                Reply::Hello => return self.lose(error),
                Reply::Attach { pane, stream } => {
                    self.attach_failed(pane, stream, error);
                    let timeout = self.options.request_timeout;
                    self.send(&Call::PaneDetach { pane }, timeout, Reply::Ignore);
                }
                reply => self.resolve(reply, Err(error)),
            }
        }
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}
