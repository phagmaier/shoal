//! How a client connects, and the auto-start strategy it calls when the
//! socket is not there (M1 spec §4.1).

use std::{
    collections::BTreeMap,
    io,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use shoal_proto::ClientKind;

/// How long the client waits for the socket after auto-start returns.
pub(crate) const START_WAIT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(10);

#[derive(Debug, Clone)]
pub struct Options {
    pub socket: PathBuf,
    pub client: ClientKind,
    /// This program's version, sent in `hello`.
    pub version: String,
    /// The terminal core this program decodes snapshots with; `None` for
    /// the CLI, which never attaches (§5.3).
    pub core: Option<String>,
    /// The window's allowlisted environment (§4.2), sent in `hello`.
    pub env: BTreeMap<String, String>,
    /// How long any request may wait for its answer.
    pub request_timeout: Duration,
    /// Reconnect with backoff after losing the daemon, then re-subscribe and
    /// re-attach. Off for one-shot CLI commands.
    pub reconnect: bool,
}

impl Options {
    /// Defaults: a 5 s request timeout, reconnecting, an empty environment.
    pub fn new(socket: impl Into<PathBuf>, client: ClientKind) -> Self {
        Self {
            socket: socket.into(),
            client,
            version: env!("CARGO_PKG_VERSION").into(),
            core: None,
            env: BTreeMap::new(),
            request_timeout: Duration::from_secs(5),
            reconnect: true,
        }
    }
}

/// Starts the daemon when the first connection attempt finds no socket. It
/// runs on the client's thread and may block; the client then waits up to
/// 2 s for the socket. Reconnecting never calls it.
pub trait AutoStart: Send + 'static {
    fn start(&mut self, socket: &Path) -> io::Result<()>;
}

impl<F: FnMut(&Path) -> io::Result<()> + Send + 'static> AutoStart for F {
    fn start(&mut self, socket: &Path) -> io::Result<()> {
        self(socket)
    }
}

/// Never start anything: connecting fails if the daemon is not running.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoAutoStart;

impl AutoStart for NoAutoStart {
    fn start(&mut self, _socket: &Path) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::NotFound, "auto-start is off"))
    }
}

/// Poll until `socket` accepts a connection, for at most `timeout`. For
/// strategies that try one way to start the daemon, then another.
pub fn wait_for_socket(socket: &Path, timeout: Duration) -> bool {
    connect_within(socket, timeout).is_ok()
}

pub(crate) fn connect_within(socket: &Path, timeout: Duration) -> io::Result<UnixStream> {
    let deadline = Instant::now() + timeout;
    loop {
        match UnixStream::connect(socket) {
            Ok(stream) => return Ok(stream),
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => std::thread::sleep(POLL),
        }
    }
}
