//! `shoald` startup, handshake and stop (M1 spec §4.1, §4.5, §4.6, §5.1–5.3),
//! run as the real binary in a temporary environment.

use std::{
    fs,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::Path,
    process::Stdio,
};

use serde_json::{Value, json};
use shoal_proto::{Frame, Response, RpcError};
use shoal_testkit::{Signal, TempEnv, eventually};

const SHOALD: &str = env!("CARGO_BIN_EXE_shoald");

/// INPUT (§5.1): len 12, kind 0x20, pane 3, then `ls\r`.
const INPUT: [u8; 16] = [12, 0, 0, 0, 0x20, 3, 0, 0, 0, 0, 0, 0, 0, b'l', b's', b'\r'];
/// OUTPUT, which only the daemon sends: len 18, kind 0x10, pane 3, seq 1, `x`.
const OUTPUT: [u8; 22] = [
    18, 0, 0, 0, 0x10, 3, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, b'x',
];

fn mode(path: &Path) -> u32 {
    fs::metadata(path).expect("metadata").permissions().mode() & 0o777
}

/// The error in `response`, which must answer request `id` (`None` for null).
fn error(response: Response<Value>, id: Option<u64>) -> RpcError {
    match response {
        Response::Error { id: got, error } => {
            assert_eq!(got, id.map(Into::into), "{error:?}");
            error
        }
        Response::Success { result, .. } => panic!("expected an error, got {result}"),
    }
}

#[test]
fn hello_reports_the_daemon() {
    let env = TempEnv::new();
    let daemon = env.spawn_daemon(SHOALD);
    let mut connection = env.connect();
    assert_eq!(
        connection.hello(),
        json!({
            "protocol": 1,
            "daemon_version": "0.1.0",
            "core": shoal_term::core_id(),
            "pid": daemon.pid(),
        })
    );
}

#[test]
fn requests_after_hello_are_answered_in_order() {
    let env = TempEnv::new();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut connection = env.connect();
    connection.hello();
    connection.send_request(1, "no.such_method", json!({}));
    connection.send_request(2, "session.close", json!({}));
    let hello = json!({"protocol": 1, "client": "cli", "version": "0.1.0", "env": {}});
    connection.send_request(3, "hello", hello);
    connection.send_json(json!({"jsonrpc": "1.0", "id": 4, "method": "x"}));
    // Clients send no notifications in M1; one is ignored, not answered.
    connection.send_json(json!({"jsonrpc": "2.0", "method": "no.such_notice", "params": {}}));
    connection.send_request(5, "workspace.get", json!({}));

    let unknown = error(connection.response(), Some(1));
    assert_eq!(
        (unknown.code, unknown.message.as_str()),
        (-32601, "unknown method")
    );
    let bad = error(connection.response(), Some(2));
    assert_eq!(bad.code, 1004, "{bad:?}");
    assert_eq!(error(connection.response(), Some(3)).code, 1002);
    assert_eq!(error(connection.response(), None).code, -32600);
    match connection.response() {
        Response::Success { id, result } => {
            assert_eq!(id, 5_u64.into());
            assert_eq!(result["rev"], 0);
            assert_eq!(result["workspace"]["sessions"], json!([]));
        }
        other => panic!("workspace.get failed: {other:?}"),
    }
}

#[test]
fn anything_but_hello_first_gets_an_error_and_closes_the_connection() {
    let env = TempEnv::new();
    let _daemon = env.spawn_daemon(SHOALD);
    let json = |message: Value| {
        let Value::Object(object) = message else {
            panic!("JSON frames are objects");
        };
        Frame::Json(object).encode().expect("encode")
    };
    let firsts = [
        (
            "a request",
            json(json!({"jsonrpc": "2.0", "id": 7, "method": "workspace.get", "params": {}})),
            Some(7),
        ),
        (
            "a notification",
            json(json!({"jsonrpc": "2.0", "method": "hello", "params": {}})),
            None,
        ),
        (
            "an invalid envelope",
            json(json!({"id": 1, "method": "hello"})),
            None,
        ),
        ("an input frame", INPUT.to_vec(), None),
    ];
    for (what, frame, id) in firsts {
        let mut connection = env.connect();
        connection.send_raw(&frame);
        let refused = error(connection.response(), id);
        assert_eq!(
            (refused.code, refused.message.as_str()),
            (1002, "hello must come first"),
            "{what}"
        );
        connection.assert_closed();
    }
}

#[test]
fn a_failed_hello_closes_the_connection() {
    let env = TempEnv::new();
    let daemon = env.spawn_daemon(SHOALD);

    let mut connection = env.connect();
    let params = json!({"protocol": 2, "client": "gui", "version": "9.0.0", "env": {}});
    let mismatch = error(connection.request(1, "hello", params), Some(1));
    assert_eq!(mismatch.code, 1003);
    assert_eq!(
        mismatch.data,
        Some(json!({"daemon_protocol": 1, "pid": daemon.pid()}))
    );
    connection.assert_closed();

    let mut connection = env.connect();
    let params = json!({"protocol": 1, "client": "toaster", "version": "0.1.0", "env": {}});
    assert_eq!(
        error(connection.request(1, "hello", params), Some(1)).code,
        1004
    );
    connection.assert_closed();
}

#[test]
fn a_protocol_error_closes_only_that_connection() {
    let env = TempEnv::new();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut broken = env.connect();
    broken.hello();
    // Length 1, kind 0x7f: an unknown frame kind.
    broken.send_raw(&[1, 0, 0, 0, 0x7f]);
    broken.assert_closed();

    let mut daemon_only = env.connect();
    daemon_only.hello();
    daemon_only.send_raw(&OUTPUT);
    daemon_only.assert_closed();

    env.connect().hello();
    assert!(env.log().contains("protocol error"), "{}", env.log());
}

#[test]
fn a_second_daemon_exits_0_with_already_running() {
    let env = TempEnv::new();
    let first = env.spawn_daemon(SHOALD);
    let second = env.start_daemon(SHOALD).wait();
    assert!(second.status.success(), "{second:?}");
    assert!(env.log().contains("already running"), "{}", env.log());
    let mut connection = env.connect();
    assert_eq!(connection.hello()["pid"], first.pid());
}

#[test]
fn a_stale_socket_file_is_recovered() {
    let env = TempEnv::new();
    fs::create_dir(env.runtime_dir()).expect("runtime dir");
    // A listener dropped without unlinking leaves the file, as a crash does.
    drop(UnixListener::bind(env.socket()).expect("stale socket"));
    assert!(env.socket().exists());
    let _daemon = env.spawn_daemon(SHOALD);
    env.connect().hello();
}

#[test]
fn stop_signals_exit_0_and_remove_the_socket() {
    for signal in [Signal::TERM, Signal::INT] {
        let env = TempEnv::new();
        let daemon = env.spawn_daemon(SHOALD);
        let mut connection = env.connect();
        connection.hello();
        daemon.signal(signal);
        let exited = daemon.wait();
        assert!(exited.status.success(), "{signal:?}: {exited:?}");
        assert!(!env.socket().exists(), "{signal:?} left the socket");
        connection.assert_closed();
        assert!(env.log().contains("shoald stopped"), "{}", env.log());
    }
}

#[test]
fn the_runtime_folder_and_socket_are_private_and_logs_go_to_state() {
    let env = TempEnv::new();
    fs::create_dir(env.runtime_dir()).expect("runtime dir");
    fs::set_permissions(env.runtime_dir(), fs::Permissions::from_mode(0o755)).expect("chmod");
    let _daemon = env.spawn_daemon(SHOALD);
    assert_eq!(mode(&env.runtime_dir()), 0o700);
    assert_eq!(mode(&env.socket()), 0o600);
    assert_eq!(mode(&env.runtime_dir().join("shoald.lock")), 0o600);
    // The socket is bound just before the line is written.
    eventually("the listening line", || {
        env.log().contains("shoald listening").then_some(())
    });
}

#[test]
fn without_a_runtime_folder_it_fails_with_a_message() {
    let env = TempEnv::new();
    let output = env
        .command(SHOALD)
        .env_remove("XDG_RUNTIME_DIR")
        .stdin(Stdio::null())
        .output()
        .expect("run shoald");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "shoald: XDG_RUNTIME_DIR must be an absolute path\n"
    );
}

#[test]
fn input_after_hello_is_accepted_quietly() {
    let env = TempEnv::new();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut connection = env.connect();
    connection.hello();
    connection.send_raw(&INPUT);
    let next = connection.request(9, "no.such_method", json!({}));
    assert_eq!(error(next, Some(9)).code, -32601);
}
