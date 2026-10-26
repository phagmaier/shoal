//! T22: `shoal daemon start | stop | status | persist`, auto-start and the
//! protocol-mismatch stop, through the real binaries in a temporary
//! environment whose `systemctl` is a logging fake (`TempEnv`).

use std::{
    fs,
    io::{Read as _, Write as _},
    os::unix::{fs::DirBuilderExt as _, net::UnixListener},
    process::{Command, Output, Stdio},
};

use serde_json::json;
use shoal_client::proto::{Frame, Message, decode};
use shoal_testkit::{TempEnv, eventually, shoald};

const SHOAL: &str = env!("CARGO_BIN_EXE_shoal");

fn shoal(env: &TempEnv, args: &[&str]) -> Output {
    env.command(SHOAL).args(args).output().unwrap()
}

fn ok(env: &TempEnv, args: &[&str]) -> String {
    let output = shoal(env, args);
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn failed(env: &TempEnv, args: &[&str], code: i32) -> String {
    let output = shoal(env, args);
    assert_eq!(output.status.code(), Some(code), "{args:?}: {output:?}");
    String::from_utf8(output.stderr).unwrap()
}

/// Stops a daemon the CLI started, even when a test fails; nothing else owns it.
struct Stop<'a>(&'a TempEnv);

impl Drop for Stop<'_> {
    fn drop(&mut self) {
        let _ = shoal(self.0, &["daemon", "stop"]);
    }
}

fn pid(started: &str) -> u32 {
    let digits: String = started
        .split("(pid ")
        .nth(1)
        .unwrap()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().unwrap()
}

/// The session id from `/proc/<pid>/stat`.
fn session_of(pid: u32) -> u32 {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let after = &stat[stat.rfind(')').unwrap() + 2..];
    after.split(' ').nth(3).unwrap().parse().unwrap()
}

#[test]
fn without_systemd_start_spawns_a_detached_daemon() {
    let env = TempEnv::new();
    let _stop = Stop(&env);
    assert_eq!(
        failed(&env, &["daemon", "status"], 3),
        "shoal: can't reach shoald: it isn't running\n"
    );
    assert_eq!(
        failed(&env, &["daemon", "persist"], 3),
        "shoal: can't reach shoald: it isn't running\n"
    );
    assert!(env.systemctl_calls().is_empty(), "only start starts");

    let started = ok(&env, &["daemon", "start"]);
    assert!(started.starts_with("Started shoald (pid "), "{started}");
    assert_eq!(env.systemctl_calls(), ["--user start shoald.service"]);
    let pid = pid(&started);
    // Its own session: closing the terminal that started it can't hang it up.
    assert_eq!(session_of(pid), pid);
    assert_eq!(
        ok(&env, &["daemon", "start"]),
        format!("shoald is already running (pid {pid}).\n")
    );

    let status = ok(&env, &["daemon", "status"]);
    assert!(
        status.starts_with(&format!(
            "shoald {} is running (pid {pid}, up ",
            env!("CARGO_PKG_VERSION")
        )),
        "{status}"
    );
    assert!(
        status.ends_with("\n0 sessions, 0 panes. Not saved yet.\n"),
        "{status}"
    );
    assert_eq!(ok(&env, &["daemon", "persist"]), "Saved.\n");
    assert!(ok(&env, &["daemon", "status"]).contains(". Last saved "));

    assert_eq!(ok(&env, &["daemon", "stop"]), "Stopped shoald.\n");
    assert!(!env.socket().exists());
    eventually("the daemon to exit", || {
        (!std::path::Path::new(&format!("/proc/{pid}")).exists()
            || fs::read_to_string(format!("/proc/{pid}/stat"))
                .is_ok_and(|stat| stat.contains(") Z ")))
        .then_some(())
    });
    assert_eq!(ok(&env, &["daemon", "stop"]), "shoald isn't running.\n");
    assert_eq!(env.systemctl_calls().len(), 1);
}

#[test]
fn workspace_commands_start_the_daemon_when_needed() {
    let env = TempEnv::new();
    let _stop = Stop(&env);
    assert_eq!(ok(&env, &["ls"]), "No sessions.\n");
    assert_eq!(env.systemctl_calls(), ["--user start shoald.service"]);
    ok(&env, &["new", "-n", "work"]);
    assert_eq!(env.systemctl_calls().len(), 1, "already running");
}

#[test]
fn systemd_starts_it_when_it_can() {
    let env = TempEnv::new();
    let _stop = Stop(&env);
    // A systemd stand-in that starts the daemon itself.
    env.fake_systemctl(&format!(
        "[ \"$*\" = '--user start shoald.service' ] && setsid -f '{}' </dev/null >/dev/null 2>&1",
        shoald().display()
    ));
    let started = ok(&env, &["daemon", "start"]);
    assert!(started.starts_with("Started shoald (pid "), "{started}");
    assert_eq!(env.systemctl_calls(), ["--user start shoald.service"]);
    // The CLI didn't also spawn one, which would have found it running.
    assert!(!env.log().contains("already running"), "{}", env.log());
}

#[test]
fn a_different_protocol_is_reported_and_stopped_with_sigterm() {
    let env = TempEnv::new();
    fs::DirBuilder::new()
        .mode(0o700)
        .create(env.runtime_dir())
        .unwrap();
    // A stand-in "old daemon": a process to signal, and a socket that refuses
    // hello with its pid, as a shoald of another protocol would.
    let mut old = Command::new("sleep").arg("30").spawn().unwrap();
    let listener = UnixListener::bind(env.socket()).unwrap();
    let old_pid = old.id();
    let server = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut buffer = Vec::new();
            let mut chunk = [0; 4096];
            let request = loop {
                if let Some((Frame::Json(object), _)) = decode(&buffer).unwrap() {
                    break Message::from_object(object).unwrap();
                }
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => {
                        break Message::Notification(shoal_client::proto::RawNotification {
                            method: String::new(),
                            params: json!({}),
                        });
                    }
                    Ok(read) => buffer.extend_from_slice(&chunk[..read]),
                }
            };
            let Message::Request(request) = request else {
                continue;
            };
            let refusal = json!({"jsonrpc": "2.0", "id": request.id, "error": {
                "code": 1003, "message": "protocol mismatch",
                "data": {"daemon_protocol": 2, "pid": old_pid}}});
            let serde_json::Value::Object(object) = refusal else {
                unreachable!()
            };
            let _ = stream.write_all(&Frame::Json(object).encode().unwrap());
        }
    });

    assert_eq!(
        failed(&env, &["ls"], 1),
        "shoal: shoald is from a different build. Restart it with `shoal daemon stop`, then try again.\n"
    );
    // Stopping signals the old pid, then waits for its socket to go.
    let stop = env
        .command(SHOAL)
        .args(["daemon", "stop"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let status = old.wait().unwrap();
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&status),
        Some(15)
    );
    fs::remove_file(env.socket()).unwrap();
    let output = stop.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "Stopped shoald.\n"
    );
    // systemd was asked whether it runs that pid, and doesn't.
    assert_eq!(
        env.systemctl_calls(),
        ["--user show -p MainPID --value shoald.service"]
    );
    drop(server);
}
