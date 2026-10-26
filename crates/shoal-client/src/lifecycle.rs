//! Starting, stopping and restarting `shoald` (M1 spec §4.1, §5.3), shared
//! by the CLI and the window's restart button. Everything here blocks.

use std::{
    io,
    os::unix::process::CommandExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use shoal_proto::{Call, CoreMismatchData, Empty, HelloResult, ProtocolMismatchData, error_code};

use crate::{AutoStart, Client, Connected, Error, NoAutoStart, Options, wait_for_socket};

const UNIT: &str = "shoald.service";
/// How long `systemctl --user start` has to bring up the socket.
const SYSTEMD_WAIT: Duration = Duration::from_secs(2);
/// How long a stopping daemon has to remove its socket (§5.3 step 2).
const STOP_WAIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(10);

/// The §4.1 strategy: `systemctl --user start shoald.service` and up to 2 s
/// for the socket; if that fails or systemd is absent, `shoald` started
/// detached (its own session, stdio to `/dev/null`).
#[derive(Debug, Clone)]
pub struct SystemdOrSpawn {
    shoald: PathBuf,
}

impl SystemdOrSpawn {
    /// Spawn this `shoald` when systemd can't start the unit.
    pub fn with_shoald(shoald: impl Into<PathBuf>) -> Self {
        Self {
            shoald: shoald.into(),
        }
    }

    /// Spawn the `shoald` next to the running binary, else the one on `PATH`.
    pub fn new() -> Self {
        let beside = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.join("shoald")))
            .filter(|path| path.is_file());
        Self {
            shoald: beside.unwrap_or_else(|| "shoald".into()),
        }
    }
}

impl Default for SystemdOrSpawn {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoStart for SystemdOrSpawn {
    fn start(&mut self, socket: &Path) -> io::Result<()> {
        match systemctl(&["start", UNIT]) {
            Ok(true) if wait_for_socket(socket, SYSTEMD_WAIT) => return Ok(()),
            Ok(true) => tracing::warn!("{UNIT} started but its socket did not appear"),
            Ok(false) => tracing::info!("systemctl could not start {UNIT}"),
            Err(error) => tracing::info!(%error, "no systemctl"),
        }
        spawn_detached(&self.shoald)
    }
}

fn spawn_detached(shoald: &Path) -> io::Result<()> {
    let mut command = Command::new(shoald);
    command
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and touches no memory of the
    // parent, so it may run between fork and exec.
    unsafe {
        command.pre_exec(|| {
            rustix::process::setsid().map_err(io::Error::from)?;
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    // Reap it whenever it exits, so a long-lived window keeps no zombie.
    std::thread::Builder::new()
        .name("shoald-reaper".into())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(())
}

/// `systemctl --user <args>`, quietly: whether it succeeded.
fn systemctl(args: &[&str]) -> io::Result<bool> {
    Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
}

/// How the daemon differs from this program (§5.3, §7.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    /// `hello` failed: nothing works until a restart.
    Protocol { daemon_protocol: u32, pid: u32 },
    /// Everything but attaching works.
    Core { daemon_core: String },
    /// Everything works; a restart brings the new version.
    Version { daemon_version: String },
}

impl Mismatch {
    /// A refused `hello` (protocol) or `pane.attach` (core).
    pub fn of_error(error: &Error) -> Option<Self> {
        let Error::Rpc(error) = error else {
            return None;
        };
        if error.code != error_code::VERSION_MISMATCH {
            return None;
        }
        let data = error.data.clone()?;
        if let Ok(data) = serde_json::from_value::<ProtocolMismatchData>(data.clone()) {
            return Some(Self::Protocol {
                daemon_protocol: data.daemon_protocol,
                pid: data.pid,
            });
        }
        serde_json::from_value::<CoreMismatchData>(data)
            .ok()
            .map(|data| Self::Core {
                daemon_core: data.daemon_core,
            })
    }

    /// A successful `hello` from a daemon of another core or version. The core
    /// counts only for a client that decodes snapshots (sends a core).
    pub fn of_hello(options: &Options, hello: &HelloResult) -> Option<Self> {
        if options
            .core
            .as_ref()
            .is_some_and(|core| *core != hello.core)
        {
            return Some(Self::Core {
                daemon_core: hello.core.clone(),
            });
        }
        (hello.daemon_version != options.version).then(|| Self::Version {
            daemon_version: hello.daemon_version.clone(),
        })
    }
}

/// What [`stop`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    Stopped,
    NotRunning,
}

/// Stop the daemon through its normal shutdown, which saves everything first
/// (§4.5), and wait up to 5 s for its socket to go. With the same protocol
/// that is `daemon.shutdown`; with another, `systemctl --user stop` if systemd
/// runs it, else `SIGTERM`. Never `SIGKILL`.
pub fn stop(options: &Options) -> Result<Stopped, Error> {
    let mut quiet = options.clone();
    quiet.reconnect = false;
    match Client::connect(quiet, NoAutoStart) {
        Err(Error::Connect(_)) => return Ok(Stopped::NotRunning),
        Err(error) => match Mismatch::of_error(&error) {
            Some(Mismatch::Protocol { pid, .. }) => terminate(pid)?,
            _ => return Err(error),
        },
        Ok(Connected { client, .. }) => match client.request(Call::DaemonShutdown(Empty {})) {
            // The daemon may close the connection as it answers.
            Ok(_) | Err(Error::NotConnected) => {}
            Err(error) => return Err(error),
        },
    }
    wait_gone(&options.socket, STOP_WAIT)?;
    Ok(Stopped::Stopped)
}

/// §5.3: stop, then start as in §4.1 (the window passes
/// [`SystemdOrSpawn`]) and connect again.
pub fn restart(options: Options, auto_start: impl AutoStart) -> Result<Connected, Error> {
    stop(&options)?;
    Client::connect(options, auto_start)
}

/// End a daemon whose protocol differs, as its manager would.
fn terminate(pid: u32) -> Result<(), Error> {
    let main_pid = Command::new("systemctl")
        .args(["--user", "show", "-p", "MainPID", "--value", UNIT])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned());
    if main_pid.as_deref().is_some_and(|main| managed(main, pid)) {
        return match systemctl(&["stop", UNIT]) {
            Ok(true) => Ok(()),
            _ => Err(Error::Connect(format!("systemctl could not stop {UNIT}"))),
        };
    }
    signal_stop(pid)
}

/// Whether `systemctl show -p MainPID --value` output names `pid`.
fn managed(main_pid: &str, pid: u32) -> bool {
    main_pid.trim().parse::<u32>() == Ok(pid)
}

/// `SIGTERM`: the daemon saves and exits as for `daemon.shutdown`.
pub fn signal_stop(pid: u32) -> Result<(), Error> {
    let pid = rustix::process::Pid::from_raw(pid.cast_signed())
        .ok_or_else(|| Error::Connect(format!("shoald reported pid {pid}")))?;
    rustix::process::kill_process(pid, rustix::process::Signal::TERM)
        .map_err(|error| Error::Connect(format!("cannot signal shoald: {error}")))
}

fn wait_gone(socket: &Path, timeout: Duration) -> Result<(), Error> {
    let deadline = Instant::now() + timeout;
    while socket.exists() {
        if Instant::now() >= deadline {
            return Err(Error::DidNotStop);
        }
        std::thread::sleep(POLL);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shoal_proto::{ClientKind, RpcError};

    #[test]
    fn mismatches_come_from_refusals_and_hello() {
        let refused = |data| {
            Error::Rpc(RpcError {
                code: error_code::VERSION_MISMATCH,
                message: "mismatch".into(),
                data: Some(data),
            })
        };
        assert_eq!(
            Mismatch::of_error(&refused(
                serde_json::json!({"daemon_protocol": 2, "pid": 9})
            )),
            Some(Mismatch::Protocol {
                daemon_protocol: 2,
                pid: 9
            })
        );
        assert_eq!(
            Mismatch::of_error(&refused(serde_json::json!({"daemon_core": "other"}))),
            Some(Mismatch::Core {
                daemon_core: "other".into()
            })
        );
        assert_eq!(Mismatch::of_error(&Error::NotConnected), None);

        let mut options = Options::new("/s", ClientKind::Gui);
        options.version = "0.2.0".into();
        let hello = |core: &str, version: &str| HelloResult {
            protocol: 1,
            daemon_version: version.into(),
            core: core.into(),
            pid: 1,
        };
        assert_eq!(Mismatch::of_hello(&options, &hello("a", "0.2.0")), None);
        assert_eq!(
            Mismatch::of_hello(&options, &hello("a", "0.1.0")),
            Some(Mismatch::Version {
                daemon_version: "0.1.0".into()
            })
        );
        options.core = Some("a".into());
        assert_eq!(
            Mismatch::of_hello(&options, &hello("b", "0.1.0")),
            Some(Mismatch::Core {
                daemon_core: "b".into()
            }),
            "the core matters more than the version"
        );
    }

    #[test]
    fn systemd_manages_only_its_own_main_pid() {
        assert!(managed("4312\n", 4312));
        assert!(!managed("0\n", 4312), "the unit is not running");
        assert!(!managed("99\n", 4312));
        assert!(!managed("", 4312));
    }
}
