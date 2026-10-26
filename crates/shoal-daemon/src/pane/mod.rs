//! Pane runtime (M1 spec §4.2): one OS thread per pane owns its PTY and its
//! `shoal-term` terminal, and takes commands from a [`Pane`] handle.
//!
//! The thread sleeps in `poll` on the PTY and an eventfd for commands, so a
//! quiet pane uses no CPU. Timeouts serve pending output, metadata, kill,
//! attach and resync work only; none recur while idle.

mod metadata;
mod runtime;
pub mod spawn;
pub mod stream;
pub mod subscriber;

use std::{
    collections::BTreeMap,
    ffi::OsString,
    io,
    os::fd::OwnedFd,
    path::PathBuf,
    sync::{Arc, atomic::Ordering, mpsc},
};

use rustix::event::{EventfdFlags, eventfd};
use shoal_model::{GridSize, PaneId};
use shoal_term::{CaptureFormat, SnapshotError, TermError, Theme};
use tokio::sync::{Notify, mpsc::UnboundedSender, oneshot};

pub use stream::{Item, Subscribed, SubscriberId};
pub use subscriber::{Delivery, Receiver};

/// What every pane is started with, from config and the daemon's startup.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneSettings {
    /// `shell.program`; `None` when the config leaves it empty.
    pub shell_program: Option<String>,
    pub shell_integration: shoal_config::schema::ShellIntegration,
    pub prompt_mode: shoal_config::schema::PromptMode,
    pub scrollback: u32,
    pub theme: Theme,
    /// `SHOAL_SOCKET`.
    pub socket: PathBuf,
    pub home: PathBuf,
    /// The daemon's own environment, the base of every shell's.
    pub base_env: Vec<(OsString, OsString)>,
}

/// One pane to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneSpec {
    pub id: PaneId,
    pub size: GridSize,
    /// Empty for the user's shell.
    pub argv: Vec<String>,
    /// The requested directory, or the source pane's for splits and new tabs.
    pub cwd: Option<PathBuf>,
    /// The latest connected window's values; only [`spawn::WINDOW_ENV`] apply.
    pub window_env: BTreeMap<String, String>,
}

/// What a pane reports to its owner, tagged with its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The shell ended. `early_failure` is the early-exit guard: a non-zero
    /// exit within 1 s of starting, so the pane should stay open and show
    /// what went wrong. The thread keeps serving snapshots and captures.
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
        early_failure: bool,
    },
    Metadata(Metadata),
}

/// Coalesced terminal reports and foreground-process information (§4.3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    pub cwd: Option<PathBuf>,
    pub program: Option<String>,
    pub osc_title: Option<String>,
    /// The latest bell since the previous update. Its time lets actor focus
    /// changes clear a bell that is still waiting for the metadata throttle.
    pub bell: Option<std::time::Instant>,
}

pub type Events = UnboundedSender<(PaneId, Event)>;

/// The terminal state at a point in the stream: everything up to `seq`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub seq: u64,
    pub size: GridSize,
    pub bytes: Vec<u8>,
}

/// The initial snapshot and the subscriber registered immediately after it.
#[derive(Debug)]
pub struct Attachment {
    pub snapshot: Snapshot,
    pub subscribed: Subscribed,
    pub items: Receiver,
}

#[derive(Debug, thiserror::Error)]
pub enum AttachError {
    #[error("the terminal is in the middle of an escape sequence")]
    MidSequence,
    #[error("snapshot exceeds the protocol frame limit")]
    TooLarge,
    #[error(transparent)]
    Encode(#[from] TermError),
}

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("cannot set up the pane: {0}")]
    Io(#[from] io::Error),
    #[error("cannot set up the pane's terminal: {0}")]
    Terminal(#[from] TermError),
    #[error("cannot open a pty: {0}")]
    Pty(String),
    #[error("cannot start {program}: {reason}")]
    Program { program: String, reason: String },
}

enum Command {
    Write(Vec<u8>),
    Resize(GridSize),
    SetSize {
        size: Option<GridSize>,
        reply: oneshot::Sender<(u64, GridSize)>,
    },
    ApplySettings {
        theme: Theme,
        scrollback: u32,
    },
    Snapshot(oneshot::Sender<Result<Snapshot, SnapshotError>>),
    Attach {
        notify: Arc<Notify>,
        deadline: std::time::Instant,
        reply: oneshot::Sender<Result<Attachment, AttachError>>,
    },
    Subscribe(subscriber::Sender, oneshot::Sender<Subscribed>),
    Unsubscribe(SubscriberId),
    Capture {
        format: CaptureFormat,
        scrollback: bool,
        reply: oneshot::Sender<Result<Vec<u8>, TermError>>,
    },
    Kill,
    KillBy(std::time::Instant, oneshot::Sender<()>),
}

/// The handle to a running pane thread. Dropping it ends the thread, killing
/// the shell first if it is still running.
#[derive(Debug)]
pub struct Pane {
    id: PaneId,
    pid: u32,
    commands: Option<mpsc::Sender<Command>>,
    wake: Arc<OwnedFd>,
    metadata_activity: Arc<metadata::Activity>,
}

/// Start a pane's thread, PTY and shell. Blocks until the shell is started,
/// about as long as a fork; call it where blocking is allowed.
pub fn spawn(spec: PaneSpec, settings: &PaneSettings, events: Events) -> Result<Pane, SpawnError> {
    let wake = Arc::new(
        eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK).map_err(io::Error::from)?,
    );
    let (commands, receiver) = mpsc::channel();
    let (started, result) = mpsc::sync_channel(1);
    let id = spec.id;
    let plan = runtime::Plan::new(spec, settings)?;
    let thread_wake = wake.clone();
    let metadata_activity = Arc::new(metadata::Activity::default());
    let thread_activity = metadata_activity.clone();
    std::thread::Builder::new()
        .name(format!("pane-{}", id.get()))
        .spawn(move || {
            match runtime::Runtime::start(plan, receiver, thread_wake, events, thread_activity) {
                Ok(runtime) => {
                    let _ = started.send(Ok(runtime.pid()));
                    runtime.run();
                }
                Err(error) => {
                    let _ = started.send(Err(error));
                }
            }
        })?;
    let pid = result
        .recv()
        .map_err(|_| io::Error::other("the pane thread ended while starting"))??;
    tracing::info!(pane = %id, pid, "pane started");
    Ok(Pane {
        id,
        pid,
        commands: Some(commands),
        wake,
        metadata_activity,
    })
}

impl Pane {
    pub fn id(&self) -> PaneId {
        self.id
    }

    /// The shell's process id, which is also its process group.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Internal diagnostic: actual metadata collections, including unchanged
    /// results. Reading this never wakes the pane or collects metadata.
    pub fn metadata_samples(&self) -> u64 {
        self.metadata_activity.samples.load(Ordering::Relaxed)
    }

    /// Whether event-triggered metadata work is still pending. Like the
    /// sample counter, this diagnostic does not wake the thread.
    pub fn metadata_pending(&self) -> bool {
        self.metadata_activity.pending.load(Ordering::Acquire)
    }

    /// Bytes for the shell, in order. Ignored once it has exited.
    pub fn write(&self, bytes: Vec<u8>) {
        self.send(Command::Write(bytes));
    }

    /// Resize the terminal and the PTY; subscribers get a numbered resize.
    pub fn resize(&self, size: GridSize) {
        self.send(Command::Resize(size));
    }

    /// Apply an authorized size (or query it) and acknowledge the actual grid
    /// on-thread, after the resize item is numbered.
    pub fn set_size(&self, size: Option<GridSize>) -> oneshot::Receiver<(u64, GridSize)> {
        let (reply, result) = oneshot::channel();
        self.send(Command::SetSize { size, reply });
        result
    }

    /// Change terminal defaults and the history limit on the pane's thread.
    /// Application-set colors stay in force until the application resets them.
    /// Shell and prompt settings only affect subsequently spawned panes.
    pub fn apply_settings(&self, theme: Theme, scrollback: u32) {
        self.send(Command::ApplySettings { theme, scrollback });
    }

    pub fn snapshot(&self) -> oneshot::Receiver<Result<Snapshot, SnapshotError>> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Snapshot(reply));
        receiver
    }

    /// Capture and subscribe atomically on the pane thread. If continuation
    /// tracking overflowed, wait at most two seconds for the first ground point.
    pub fn attach(
        &self,
        notify: Arc<Notify>,
    ) -> oneshot::Receiver<Result<Attachment, AttachError>> {
        let (reply, result) = oneshot::channel();
        self.send(Command::Attach {
            notify,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(2),
            reply,
        });
        result
    }

    /// Subscribe without a snapshot (internal runtime fixtures).
    pub fn subscribe(&self) -> (oneshot::Receiver<Subscribed>, Receiver) {
        let (sink, items) = subscriber::channel(Arc::new(Notify::new()));
        let (reply, result) = oneshot::channel();
        self.send(Command::Subscribe(sink, reply));
        (result, items)
    }

    pub fn unsubscribe(&self, id: SubscriberId) {
        self.send(Command::Unsubscribe(id));
    }

    /// The screen as text or terminal bytes, with history when `scrollback`.
    pub fn capture(
        &self,
        format: CaptureFormat,
        scrollback: bool,
    ) -> oneshot::Receiver<Result<Vec<u8>, TermError>> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::Capture {
            format,
            scrollback,
            reply,
        });
        receiver
    }

    /// `SIGHUP`, as a hangup sends it, then `SIGKILL` after 2 s. The shell's
    /// end arrives as [`Event::Exited`].
    pub fn kill(&self) {
        self.send(Command::Kill);
    }

    /// Hang up now and escalate no later than this shared shutdown deadline.
    /// The acknowledgement confirms the signal/deadline was applied on-thread.
    pub fn kill_by(&self, deadline: std::time::Instant) -> oneshot::Receiver<()> {
        let (reply, receiver) = oneshot::channel();
        self.send(Command::KillBy(deadline, reply));
        receiver
    }

    fn send(&self, command: Command) {
        // A thread that has ended drops the command and its reply channel,
        // which the caller sees as a closed receiver.
        if let Some(commands) = &self.commands
            && commands.send(command).is_ok()
        {
            runtime::wake(&self.wake);
        }
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        self.commands = None;
        runtime::wake(&self.wake);
    }
}
