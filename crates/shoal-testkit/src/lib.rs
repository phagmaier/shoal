//! Test-only fixtures: an isolated environment, the real `shoald`, raw protocol
//! connections and [`eventually`]. Helpers panic with a description instead of
//! returning errors, since every caller is a test.
//!
//! Paths are written out here rather than taken from `shoal-config`, so tests
//! check that the daemon uses the locations M1 spec §4.1 and §4.6 name.

use std::{
    ffi::{OsStr, OsString},
    io::{ErrorKind, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

use rustix::process::Pid;
pub use rustix::process::Signal;
use serde_json::{Value, json};
use shoal_proto::{Frame, Message, Response};

/// How long anything in a test may wait.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// Poll `check` until it returns `Some`, panicking after [`TIMEOUT`].
pub fn eventually<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    let mut pause = Duration::from_millis(1);
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "timed out after {TIMEOUT:?} waiting for {what}"
        );
        std::thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(50));
    }
}

/// `HOME` and every XDG folder under one temporary directory, removed on drop.
pub struct TempEnv {
    root: tempfile::TempDir,
}

impl Default for TempEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl TempEnv {
    pub fn new() -> Self {
        let root = tempfile::tempdir().expect("create the temporary environment");
        for dir in ["home", "config", "state", "cache", "data", "runtime"] {
            std::fs::create_dir(root.path().join(dir)).expect("create an XDG folder");
        }
        Self { root }
    }

    pub fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    pub fn config_home(&self) -> PathBuf {
        self.root.path().join("config")
    }

    pub fn state_home(&self) -> PathBuf {
        self.root.path().join("state")
    }

    pub fn cache_home(&self) -> PathBuf {
        self.root.path().join("cache")
    }

    pub fn data_home(&self) -> PathBuf {
        self.root.path().join("data")
    }

    /// `XDG_RUNTIME_DIR`.
    pub fn runtime_home(&self) -> PathBuf {
        self.root.path().join("runtime")
    }

    /// `$XDG_RUNTIME_DIR/shoal/`.
    pub fn runtime_dir(&self) -> PathBuf {
        self.runtime_home().join("shoal")
    }

    pub fn socket(&self) -> PathBuf {
        self.runtime_dir().join("shoald.sock")
    }

    pub fn log_file(&self) -> PathBuf {
        self.state_home().join("shoal/logs/shoald.log")
    }

    /// The daemon's log so far; empty before it exists.
    pub fn log(&self) -> String {
        std::fs::read_to_string(self.log_file()).unwrap_or_default()
    }

    /// This environment's variables: `PATH` and the locale are kept from the
    /// test's, everything else, `SHOAL_*` included, is left out. Logging is at
    /// debug.
    pub fn vars(&self) -> Vec<(OsString, OsString)> {
        let mut vars: Vec<(OsString, OsString)> = ["PATH", "LANG", "LC_ALL"]
            .into_iter()
            .filter_map(|kept| Some((kept.into(), std::env::var_os(kept)?)))
            .collect();
        let dirs = [
            ("HOME", self.home()),
            ("XDG_CONFIG_HOME", self.config_home()),
            ("XDG_STATE_HOME", self.state_home()),
            ("XDG_CACHE_HOME", self.cache_home()),
            ("XDG_DATA_HOME", self.data_home()),
            ("XDG_RUNTIME_DIR", self.runtime_home()),
        ];
        vars.extend(dirs.map(|(key, dir)| (key.into(), dir.into())));
        vars.push(("SHOAL_LOG".into(), "debug".into()));
        vars
    }

    /// `program` with only [`TempEnv::vars`], started in `HOME`.
    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .envs(self.vars())
            .current_dir(self.home());
        command
    }

    /// Start the daemon binary at `program` and wait until it accepts connections.
    pub fn spawn_daemon(&self, program: impl AsRef<Path>) -> Daemon {
        let daemon = self.start_daemon(program);
        eventually("the daemon to accept connections", || {
            UnixStream::connect(self.socket()).ok()
        });
        daemon
    }

    /// Start the daemon binary at `program` without waiting for it.
    pub fn start_daemon(&self, program: impl AsRef<Path>) -> Daemon {
        let child = self
            .command(program.as_ref())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start the daemon");
        Daemon { child: Some(child) }
    }

    /// A raw connection to the daemon's socket, before `hello`.
    pub fn connect(&self) -> Connection {
        let stream = UnixStream::connect(self.socket()).expect("connect to the daemon");
        stream
            .set_read_timeout(Some(TIMEOUT))
            .expect("set a read timeout");
        Connection {
            stream,
            buffer: Vec::new(),
        }
    }
}

/// A running daemon, killed on drop if a test did not stop it.
pub struct Daemon {
    child: Option<Child>,
}

impl Daemon {
    pub fn pid(&self) -> u32 {
        self.child().id()
    }

    pub fn signal(&self, signal: Signal) {
        let pid = Pid::from_child(self.child());
        rustix::process::kill_process(pid, signal).expect("signal the daemon");
    }

    /// Wait for the daemon to exit by itself.
    pub fn wait(mut self) -> Exited {
        let mut child = self
            .child
            .take()
            .expect("invariant: the child is present until waited");
        let status = eventually("the daemon to exit", || {
            child.try_wait().expect("wait for the daemon")
        });
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            pipe.read_to_string(&mut stderr)
                .expect("read the daemon's stderr");
        }
        Exited { status, stderr }
    }

    fn child(&self) -> &Child {
        self.child
            .as_ref()
            .expect("invariant: the child is present until waited")
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Debug)]
pub struct Exited {
    pub status: ExitStatus,
    pub stderr: String,
}

/// What [`Connection::recv`] read.
#[derive(Debug)]
pub enum Received {
    Json(Message),
    /// A binary frame's full wire bytes, for `shoal_proto::decode_exact`.
    Binary(Vec<u8>),
    /// The daemon closed the connection.
    Closed,
}

/// Frames over a blocking socket; every read gives up after [`TIMEOUT`].
pub struct Connection {
    stream: UnixStream,
    buffer: Vec<u8>,
}

impl Connection {
    pub fn send_raw(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).expect("write to the daemon");
    }

    pub fn send_json(&mut self, message: Value) {
        let Value::Object(object) = message else {
            panic!("a JSON frame must be an object");
        };
        self.send_raw(&Frame::Json(object).encode().expect("encode a frame"));
    }

    pub fn send_request(&mut self, id: u64, method: &str, params: Value) {
        self.send_json(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
    }

    /// The next frame, or `Closed`. Panics if nothing arrives within [`TIMEOUT`].
    pub fn recv(&mut self) -> Received {
        loop {
            match shoal_proto::decode(&self.buffer).expect("the daemon sent a valid frame") {
                Some((Frame::Json(object), used)) => {
                    self.buffer.drain(..used);
                    let message = Message::from_object(object).expect("a valid JSON-RPC message");
                    return Received::Json(message);
                }
                Some((_, used)) => return Received::Binary(self.buffer.drain(..used).collect()),
                None => {}
            }
            let mut chunk = [0; 64 * 1024];
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    assert!(
                        self.buffer.is_empty(),
                        "the daemon closed partway through a frame"
                    );
                    return Received::Closed;
                }
                Ok(read) => self.buffer.extend_from_slice(&chunk[..read]),
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    panic!("nothing from the daemon within {TIMEOUT:?}")
                }
                Err(error) if error.kind() == ErrorKind::ConnectionReset => {
                    return Received::Closed;
                }
                Err(error) => panic!("read from the daemon: {error}"),
            }
        }
    }

    /// The next frame, which must be a response.
    pub fn response(&mut self) -> Response<Value> {
        match self.recv() {
            Received::Json(Message::Response(response)) => response,
            other => panic!("expected a response, got {other:?}"),
        }
    }

    /// Send a request and read its response.
    pub fn request(&mut self, id: u64, method: &str, params: Value) -> Response<Value> {
        self.send_request(id, method, params);
        self.response()
    }

    /// A CLI `hello` for the current protocol; returns its result.
    pub fn hello(&mut self) -> Value {
        let params = json!({"protocol": shoal_proto::PROTOCOL_VERSION, "client": "cli", "version": "0.1.0", "env": {}});
        match self.request(0, "hello", params) {
            Response::Success { result, .. } => result,
            Response::Error { error, .. } => panic!("hello failed: {error:?}"),
        }
    }

    /// Assert the daemon closes the connection next.
    pub fn assert_closed(&mut self) {
        match self.recv() {
            Received::Closed => {}
            other => panic!("expected the connection to close, got {other:?}"),
        }
    }
}
