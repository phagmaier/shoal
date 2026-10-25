//! The pane thread: its `poll` loop, the PTY, the shell's exit and kill.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    ffi::OsString,
    os::fd::{AsFd, BorrowedFd, OwnedFd, RawFd},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize};
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
    io::Errno,
    process::{
        Pid, PidfdFlags, Signal, WaitId, WaitIdOptions, kill_process_group, pidfd_open, waitid,
    },
};
use shoal_model::{GridSize, PaneId};
use shoal_term::{PaneSetup, Terminal};

use super::{
    Command, Event, Events, PaneSettings, PaneSpec, Snapshot, SpawnError, spawn, stream::Stream,
};

/// The most output read at once.
const READ_BYTES: usize = 64 * 1024;
/// How long a hung-up process group has before `SIGKILL`.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// A non-zero exit sooner than this after starting keeps the pane open.
const EARLY_EXIT: Duration = Duration::from_secs(1);
const XTVERSION: &str = concat!("shoal ", env!("CARGO_PKG_VERSION"));

/// Everything a pane thread needs to start, resolved before it runs.
pub struct Plan {
    id: PaneId,
    size: GridSize,
    argv: Vec<OsString>,
    env: BTreeMap<OsString, OsString>,
    cwd: PathBuf,
    scrollback: u32,
    theme: shoal_term::Theme,
}

impl Plan {
    pub fn new(spec: PaneSpec, settings: &PaneSettings) -> Self {
        let argv = spawn::program(
            &spec.argv,
            settings.shell_program.as_deref(),
            &settings.base_env,
            spawn::passwd_shell,
        );
        Self {
            id: spec.id,
            size: spec.size,
            argv,
            env: spawn::environment(
                &settings.base_env,
                &spec.window_env,
                spec.id,
                &settings.socket,
            ),
            cwd: spawn::directory(spec.cwd.as_deref(), &settings.home),
            scrollback: settings.scrollback,
            theme: settings.theme,
        }
    }
}

/// The shell side while it runs: the PTY's master end and the child.
struct Pty {
    master: Box<dyn MasterPty + Send>,
    fd: RawFd,
    // Held so the child is never reaped elsewhere; `pidfd` collects its status.
    _child: Box<dyn Child + Send + Sync>,
    pid: Pid,
    pidfd: OwnedFd,
    /// The PTY read end of file: every holder of the terminal closed it.
    eof: bool,
}

impl Pty {
    fn fd(&self) -> BorrowedFd<'_> {
        // SAFETY: `fd` is the master's own descriptor, which `master` keeps
        // open for as long as this borrow of `self` lasts.
        unsafe { BorrowedFd::borrow_raw(self.fd) }
    }

    fn signal(&self, signal: Signal) {
        // A hangup signals the session's leader and the foreground group;
        // `pid` leads both the session and its own process group.
        let foreground = self.master.process_group_leader().and_then(Pid::from_raw);
        for group in std::iter::once(self.pid).chain(foreground.filter(|&group| group != self.pid))
        {
            match kill_process_group(group, signal) {
                Ok(()) | Err(Errno::SRCH) => {}
                Err(error) => {
                    tracing::warn!(%error, ?signal, "could not signal a pane's processes")
                }
            }
        }
    }
}

pub struct Runtime {
    id: PaneId,
    terminal: Terminal,
    /// `None` once the shell has exited.
    pty: Option<Pty>,
    stream: Stream,
    commands: mpsc::Receiver<Command>,
    wake: Arc<OwnedFd>,
    /// Filled by the terminal's reply callback while it is fed.
    replies: Rc<RefCell<Vec<u8>>>,
    /// Bytes waiting for the PTY: input and replies, in order.
    input: Vec<u8>,
    events: Events,
    started: Instant,
    kill_deadline: Option<Instant>,
    /// The handle was dropped: end once the shell has.
    orphaned: bool,
}

impl Runtime {
    /// Make the terminal, then open the PTY and start the shell, on the
    /// thread that will own them.
    pub fn start(
        plan: Plan,
        commands: mpsc::Receiver<Command>,
        wake: Arc<OwnedFd>,
        events: Events,
    ) -> Result<Self, SpawnError> {
        let replies = Rc::new(RefCell::new(Vec::new()));
        let sink = replies.clone();
        let (id, term_events) = (plan.id, events.clone());
        let terminal = Terminal::pane(
            PaneSetup {
                cols: plan.size.cols,
                rows: plan.size.rows,
                scrollback: plan.scrollback,
                theme: plan.theme,
                version: XTVERSION,
            },
            Box::new(move |bytes| sink.borrow_mut().extend_from_slice(bytes)),
            Box::new(move |event| {
                let _ = term_events.send((id, Event::Terminal(event)));
            }),
        )?;
        let pty = open(&plan)?;
        Ok(Self {
            id,
            terminal,
            pty: Some(pty),
            stream: Stream::default(),
            commands,
            wake,
            replies,
            input: Vec::new(),
            events,
            started: Instant::now(),
            kill_deadline: None,
            orphaned: false,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pty
            .as_ref()
            .map_or(0, |pty| pty.pid.as_raw_nonzero().get().cast_unsigned())
    }

    pub fn run(mut self) {
        let mut buffer = vec![0; READ_BYTES];
        loop {
            let now = Instant::now();
            if self.kill_deadline.is_some_and(|deadline| now >= deadline) {
                self.kill_deadline = None;
                if let Some(pty) = &self.pty {
                    pty.signal(Signal::KILL);
                }
            }
            if self
                .stream
                .deadline()
                .is_some_and(|deadline| now >= deadline)
            {
                self.stream.flush(now);
            }
            if self.orphaned && self.pty.is_none() {
                return;
            }
            let ready = match self.wait(now) {
                Ok(ready) => ready,
                Err(error) => {
                    tracing::error!(pane = %self.id, %error, "pane thread stopped: poll failed");
                    return;
                }
            };
            if ready.wake {
                self.commands();
            }
            if ready.output {
                self.read(&mut buffer);
            }
            if ready.input {
                self.write_input();
            }
            if ready.exited {
                self.reap();
            }
        }
    }

    /// Sleep until something is ready or a deadline passes.
    fn wait(&self, now: Instant) -> rustix::io::Result<Ready> {
        let deadline = [self.stream.deadline(), self.kill_deadline]
            .into_iter()
            .flatten()
            .min();
        let timeout = deadline.map(|deadline| {
            Timespec::try_from(deadline.saturating_duration_since(now))
                .expect("invariant: a deadline within seconds fits a timespec")
        });
        let mut fds = vec![PollFd::new(&*self.wake, PollFlags::IN)];
        let (mut pty_at, mut pidfd_at) = (None, None);
        if let Some(pty) = &self.pty {
            if pty.eof {
                // Everything is read; wait for the shell's status.
                pidfd_at = Some(fds.len());
                fds.push(PollFd::new(&pty.pidfd, PollFlags::IN));
            } else {
                let mut flags = PollFlags::IN;
                if !self.input.is_empty() {
                    flags |= PollFlags::OUT;
                }
                pty_at = Some(fds.len());
                fds.push(PollFd::from_borrowed_fd(pty.fd(), flags));
            }
        }
        match poll(&mut fds, timeout.as_ref()) {
            Ok(_) => {}
            Err(Errno::INTR) => return Ok(Ready::default()),
            Err(error) => return Err(error),
        }
        let events = |at: Option<usize>| at.map_or(PollFlags::empty(), |at| fds[at].revents());
        let pty = events(pty_at);
        Ok(Ready {
            wake: !fds[0].revents().is_empty(),
            // A hangup or error still leaves output to read before the end.
            output: pty.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR),
            input: pty.contains(PollFlags::OUT),
            exited: !events(pidfd_at).is_empty(),
        })
    }

    fn commands(&mut self) {
        // Reset the eventfd before draining, so a command sent meanwhile
        // wakes the next poll.
        let mut counter = [0; 8];
        let _ = rustix::io::read(&*self.wake, &mut counter);
        loop {
            match self.commands.try_recv() {
                Ok(command) => self.command(command),
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.orphaned = true;
                    self.kill();
                    return;
                }
            }
        }
    }

    fn command(&mut self, command: Command) {
        let now = Instant::now();
        match command {
            Command::Write(bytes) => {
                if self.pty.as_ref().is_some_and(|pty| !pty.eof) {
                    self.input.extend_from_slice(&bytes);
                    self.write_input();
                }
            }
            Command::Resize(size) => self.resize(size, now),
            Command::Snapshot(reply) => {
                // Everything fed so far gets numbered, so the snapshot holds
                // exactly the items up to `seq`.
                self.stream.flush(now);
                let snapshot = self.terminal.snapshot().map(|bytes| Snapshot {
                    seq: self.stream.seq(),
                    size: self.size(),
                    bytes,
                });
                let _ = reply.send(snapshot);
            }
            Command::Subscribe(sink, reply) => {
                let _ = reply.send(self.stream.subscribe(sink, now));
            }
            Command::Unsubscribe(id) => self.stream.unsubscribe(id),
            Command::Capture {
                format,
                scrollback,
                reply,
            } => {
                let _ = reply.send(self.terminal.capture(format, scrollback));
            }
            Command::Kill => self.kill(),
        }
    }

    fn size(&self) -> GridSize {
        let size =
            |dimension: Result<u16, _>| dimension.expect("invariant: a live terminal has a size");
        GridSize {
            cols: size(self.terminal.cols()),
            rows: size(self.terminal.rows()),
        }
    }

    fn resize(&mut self, size: GridSize, now: Instant) {
        if size == self.size() {
            return;
        }
        // The terminal first, so the program's redraw after SIGWINCH lands
        // in a terminal of the new size.
        if let Err(error) = self.terminal.resize(size.cols, size.rows) {
            tracing::warn!(pane = %self.id, %error, cols = size.cols, rows = size.rows, "resize refused");
            return;
        }
        // After the end of output nothing is left to tell.
        if let Some(pty) = self.pty.as_ref().filter(|pty| !pty.eof)
            && let Err(error) = pty.master.resize(pty_size(size))
        {
            tracing::warn!(pane = %self.id, error = format!("{error:#}"), "pty resize failed");
        }
        self.stream.resized(size, now);
    }

    fn read(&mut self, buffer: &mut [u8]) {
        let Some(pty) = &self.pty else { return };
        match rustix::io::read(pty.fd(), &mut *buffer) {
            // Every terminal end is closed: Linux reports EIO, not 0.
            Ok(0) | Err(Errno::IO) => self.end_of_output(),
            Ok(read) => {
                let output = &buffer[..read];
                self.terminal.feed(output);
                self.stream.push(output, Instant::now());
                let replies = self.replies.take();
                if !replies.is_empty() {
                    self.input.extend_from_slice(&replies);
                    self.write_input();
                }
            }
            Err(Errno::AGAIN | Errno::INTR) => {}
            Err(error) => {
                tracing::warn!(pane = %self.id, %error, "pty read failed");
                self.end_of_output();
            }
        }
    }

    fn write_input(&mut self) {
        let Some(pty) = &self.pty else { return };
        while !self.input.is_empty() {
            match rustix::io::write(pty.fd(), &self.input) {
                Ok(written) => {
                    self.input.drain(..written);
                }
                // The shell isn't reading; poll says when it is.
                Err(Errno::AGAIN) => return,
                Err(Errno::INTR) => {}
                Err(error) => {
                    tracing::debug!(pane = %self.id, %error, "pty write failed");
                    self.input.clear();
                    return;
                }
            }
        }
    }

    fn end_of_output(&mut self) {
        self.stream.flush(Instant::now());
        self.input.clear();
        if let Some(pty) = &mut self.pty {
            pty.eof = true;
        }
        // The status is usually ready already; if not, poll the pidfd.
        self.reap();
    }

    fn reap(&mut self) {
        let Some(pty) = &self.pty else { return };
        let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG;
        let (code, signal) = match waitid(WaitId::PidFd(pty.pidfd.as_fd()), options) {
            Ok(Some(status)) => (status.exit_status(), status.terminating_signal()),
            Ok(None) | Err(Errno::INTR) => return,
            Err(error) => {
                tracing::warn!(pane = %self.id, %error, "could not collect a shell's status");
                (None, None)
            }
        };
        let early_failure =
            self.started.elapsed() < EARLY_EXIT && code.is_some_and(|code| code != 0);
        tracing::info!(pane = %self.id, ?code, ?signal, early_failure, "shell exited");
        self.pty = None;
        self.kill_deadline = None;
        let _ = self.events.send((
            self.id,
            Event::Exited {
                code,
                signal,
                early_failure,
            },
        ));
    }

    fn kill(&mut self) {
        let Some(pty) = &self.pty else { return };
        if self.kill_deadline.is_none() {
            pty.signal(Signal::HUP);
            self.kill_deadline = Some(Instant::now() + KILL_GRACE);
        }
    }
}

#[derive(Debug, Default)]
struct Ready {
    wake: bool,
    output: bool,
    input: bool,
    exited: bool,
}

/// Wake a pane thread's `poll`.
pub fn wake(eventfd: &OwnedFd) {
    // Only a counter overflow could fail, and the thread resets it.
    let _ = rustix::io::write(eventfd, &1u64.to_ne_bytes());
}

fn pty_size(size: GridSize) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Open the PTY at the pane's size and start the shell on it. portable-pty
/// makes the new session and controlling terminal and marks both PTY ends
/// close-on-exec.
fn open(plan: &Plan) -> Result<Pty, SpawnError> {
    let pair = portable_pty::native_pty_system()
        .openpty(pty_size(plan.size))
        .map_err(|error| SpawnError::Pty(format!("{error:#}")))?;
    let mut command = CommandBuilder::from_argv(plan.argv.clone());
    command.env_clear();
    for (key, value) in &plan.env {
        command.env(key, value);
    }
    command.cwd(&plan.cwd);
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| SpawnError::Program {
            program: plan.argv[0].to_string_lossy().into_owned(),
            reason: format!("{error:#}"),
        })?;
    // Only the shell keeps the terminal end open, so its exit ends output.
    drop(pair.slave);
    let raw = child
        .process_id()
        .and_then(|pid| Pid::from_raw(pid.cast_signed()))
        .ok_or_else(|| SpawnError::Pty("the shell has no process id".into()))?;
    let pidfd = pidfd_open(raw, PidfdFlags::empty()).map_err(std::io::Error::from)?;
    let fd = pair
        .master
        .as_raw_fd()
        .ok_or_else(|| SpawnError::Pty("the pty has no descriptor".into()))?;
    let pty = Pty {
        master: pair.master,
        fd,
        _child: child,
        pid: raw,
        pidfd,
        eof: false,
    };
    let flags = fcntl_getfl(pty.fd()).map_err(std::io::Error::from)?;
    fcntl_setfl(pty.fd(), flags | OFlags::NONBLOCK).map_err(std::io::Error::from)?;
    Ok(pty)
}
