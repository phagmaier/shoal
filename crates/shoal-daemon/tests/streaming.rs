//! T13 contracts through the real daemon, socket and PTY.

#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use shoal_model::PaneId;
use shoal_proto::{Frame, Input, Message, Response};
use shoal_term::{CaptureFormat, Terminal};
use shoal_testkit::{Connection, Received, TempEnv, eventually};
use support::{Rpc, SHOALD, fixture};

fn pane(rpc: &mut Rpc) -> PaneId {
    rpc.ok("session.create", json!({}));
    serde_json::from_value(rpc.workspace()["sessions"][0]["tabs"][0]["focused"].clone()).unwrap()
}

fn window(env: &TempEnv) -> Connection {
    let mut connection = env.connect();
    let response = connection.request(
        0,
        "hello",
        json!({
            "protocol": 1, "client": "gui", "version": "0.1.0",
            "core": shoal_term::core_id(), "env": {}
        }),
    );
    assert!(matches!(response, Response::Success { .. }));
    connection
}

fn response(connection: &mut Connection, id: u64) -> Result<Value, shoal_proto::RpcError> {
    loop {
        match connection.recv() {
            Received::Json(Message::Response(Response::Success { id: got, result })) => {
                assert_eq!(got, id.into());
                return Ok(result);
            }
            Received::Json(Message::Response(Response::Error { id: got, error })) => {
                assert_eq!(got, Some(id.into()));
                return Err(error);
            }
            Received::Json(Message::Notification(_)) => {}
            other => panic!("expected response, got {other:?}"),
        }
    }
}

fn binary(connection: &mut Connection) -> Vec<u8> {
    loop {
        match connection.recv() {
            Received::Binary(bytes) => return bytes,
            Received::Json(Message::Notification(_)) => {}
            other => panic!("expected binary frame, got {other:?}"),
        }
    }
}

struct Mirror {
    pane: PaneId,
    seq: u64,
    terminal: Terminal,
    output: Vec<u8>,
    replacements: usize,
    resizes: usize,
    scrollback: u32,
}

impl Mirror {
    fn attach(connection: &mut Connection, pane: PaneId, id: u64, scrollback: u32) -> Self {
        connection.send_request(id, "pane.attach", json!({"pane":pane}));
        let bytes = binary(connection);
        let Frame::Snapshot(snapshot) = shoal_proto::decode_exact(&bytes).unwrap() else {
            panic!("snapshot first");
        };
        assert_eq!(snapshot.pane, pane);
        let result = response(connection, id).unwrap();
        assert_eq!(
            result,
            json!({"seq":snapshot.seq,"cols":snapshot.size.cols,"rows":snapshot.size.rows})
        );
        Self {
            pane,
            seq: snapshot.seq,
            terminal: Terminal::restore(
                snapshot.bytes,
                snapshot.size.cols,
                snapshot.size.rows,
                scrollback,
            )
            .unwrap(),
            output: Vec::new(),
            replacements: 0,
            resizes: 0,
            scrollback,
        }
    }

    fn step(&mut self, connection: &mut Connection) {
        let bytes = binary(connection);
        match shoal_proto::decode_exact(&bytes).unwrap() {
            Frame::Output(output) => {
                assert_eq!(output.pane, self.pane);
                assert_eq!(output.seq, self.seq + 1);
                self.seq = output.seq;
                self.terminal.feed(output.bytes);
                self.output.extend_from_slice(output.bytes);
            }
            Frame::Resized(resize) => {
                assert_eq!(resize.pane, self.pane);
                assert_eq!(resize.seq, self.seq + 1);
                self.seq = resize.seq;
                self.resizes += 1;
                self.terminal
                    .resize(resize.size.cols, resize.size.rows)
                    .unwrap();
            }
            Frame::Snapshot(snapshot) => {
                assert_eq!(snapshot.pane, self.pane);
                assert!(snapshot.seq >= self.seq);
                self.seq = snapshot.seq;
                self.replacements += 1;
                self.terminal = Terminal::restore(
                    snapshot.bytes,
                    snapshot.size.cols,
                    snapshot.size.rows,
                    self.scrollback,
                )
                .unwrap();
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    fn until(&mut self, connection: &mut Connection, marker: &str) {
        let deadline = std::time::Instant::now() + shoal_testkit::TIMEOUT;
        while !String::from_utf8(self.terminal.capture(CaptureFormat::Text, false).unwrap())
            .unwrap()
            .contains(marker)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "stream did not reach {marker}"
            );
            self.step(connection);
        }
    }

    fn equals_capture(&self, rpc: &mut Rpc) {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let capture = rpc.ok(
            "pane.capture",
            json!({"pane":self.pane,"format":"vt","scrollback":true}),
        );
        let got = self.terminal.capture(CaptureFormat::Vt, true).unwrap();
        let expected = STANDARD.decode(capture["data"].as_str().unwrap()).unwrap();
        assert!(
            got == expected,
            "history: mirror {} bytes, daemon {} bytes; common suffix {}; starts {:?} / {:?}",
            got.len(),
            expected.len(),
            got.iter()
                .rev()
                .zip(expected.iter().rev())
                .take_while(|(a, b)| a == b)
                .count(),
            String::from_utf8_lossy(&got[..got.len().min(60)]),
            String::from_utf8_lossy(&expected[..expected.len().min(60)])
        );
    }
}

fn script_pane(rpc: &mut Rpc, script: &str) -> PaneId {
    let session = rpc.ok("session.create", json!({}))["session"].clone();
    let created = rpc.ok(
        "tab.create",
        json!({"session":session,"argv":["bash","-c",script]}),
    );
    let pane: PaneId = serde_json::from_value(created["pane"].clone()).unwrap();
    eventually("script ready", || {
        rpc.ok(
            "pane.capture",
            json!({"pane":pane,"format":"text","scrollback":false}),
        )["data"]
            .as_str()
            .unwrap()
            .contains("ready")
            .then_some(())
    });
    pane
}

fn input(connection: &mut Connection, pane: PaneId, bytes: &[u8]) {
    connection.send_raw(&Frame::Input(Input { pane, bytes }).encode().unwrap());
}

#[test]
fn a_hundred_thousand_lines_and_resizes_stream_without_gaps_and_mid_output_attach_matches() {
    let env = fixture(false);
    std::fs::write(
        env.config_home().join("shoal/config.toml"),
        format!("{}[scrollback]\nlines = 100000\n", support::config(false)),
    )
    .unwrap();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let pane = script_pane(
        &mut rpc,
        "stty -echo; printf 'ready\\n'; read -r gate; seq -f 'line-%06g' 1 50000; read -r gate; seq -f 'line-%06g' 50001 100000; printf 'stream-finished\\n'; read -r gate; printf 'after-detach\\n'; read -r gate",
    );
    let mut first = window(&env);
    let mut full = Mirror::attach(&mut first, pane, 1, 100000);
    rpc.ok("window.set_focused", json!({"focused":true}));
    rpc.ok("pane.set_size", json!({"pane":pane,"cols":100,"rows":30}));
    input(&mut first, pane, b"go\n");
    // Begin a second attach after streaming has started, before the second half.
    while full.output.is_empty() {
        full.step(&mut first);
    }
    let mut second = window(&env);
    let mut partial = Mirror::attach(&mut second, pane, 1, 100000);
    input(&mut first, pane, b"continue\n");
    full.until(&mut first, "stream-finished");
    partial.until(&mut second, "stream-finished");
    let expected: Vec<u8> = (1..=100000)
        .flat_map(|line| format!("line-{line:06}\r\n").into_bytes())
        .chain(b"stream-finished\r\n".iter().copied())
        .collect();
    assert_eq!(full.output, expected);
    assert_eq!(full.replacements, 0);
    assert_eq!(partial.replacements, 0);
    assert_eq!(full.resizes, 1);
    full.equals_capture(&mut rpc);
    partial.equals_capture(&mut rpc);
    let replaced = Mirror::attach(&mut first, pane, 2, 100000);
    replaced.equals_capture(&mut rpc);
    // Explicit detach prevents further bytes even when another subscriber lives.
    first.send_request(3, "pane.detach", json!({"pane":pane}));
    response(&mut first, 3).unwrap();
    rpc.ok("pane.send_text", json!({"pane":pane,"text":"continue\n"}));
    partial.until(&mut second, "after-detach");
    first.send_request(4, "daemon.status", json!({}));
    response(&mut first, 4).unwrap();
}

#[test]
fn a_stalled_socket_resynchronizes_without_slowing_another_window_or_the_shell() {
    let env = fixture(false);
    // Large-output test needs screen state, not megabytes of retained history.
    std::fs::write(
        env.config_home().join("shoal/config.toml"),
        format!("{}[scrollback]\nlines = 0\n", support::config(false)),
    )
    .unwrap();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let pane = script_pane(
        &mut rpc,
        "stty -echo; printf 'ready\\n'; read -r gate; yes '0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ' | head -n 400000; printf 'flood-finished\\n'; : > flood-finished; read -r gate",
    );
    let mut slow = window(&env);
    let mut stale = Mirror::attach(&mut slow, pane, 1, 0);
    let mut fast = window(&env);
    let mut live = Mirror::attach(&mut fast, pane, 1, 0);
    input(&mut fast, pane, b"go\n");
    live.until(&mut fast, "flood-finished");
    assert_eq!(
        live.replacements, 0,
        "the actively reading client has no gap"
    );
    assert_eq!(
        live.output.iter().filter(|&&byte| byte == b'\n').count(),
        400001
    );
    eventually("shell progress with a stalled reader", || {
        env.home().join("flood-finished").exists().then_some(())
    });
    stale.until(&mut slow, "flood-finished");
    assert!(
        stale.replacements > 0,
        "the unread socket overflowed its 8 MiB queue"
    );
    stale.equals_capture(&mut rpc);
    live.equals_capture(&mut rpc);
    // Control remains usable on the resynchronized connection.
    slow.send_request(2, "pane.detach", json!({"pane":pane}));
    response(&mut slow, 2).unwrap();
    slow.send_request(3, "daemon.status", json!({}));
    response(&mut slow, 3).unwrap();
}

#[test]
fn a_mid_sequence_attach_returns_the_specified_timeout_and_can_be_retried() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let pane = script_pane(
        &mut rpc,
        "stty -echo; printf 'ready\\n'; read -r gate; printf '\\033]0;'; printf '%05000d' 0; read -r gate; : > kept-running; read -r gate; printf '\\007done\\n'; read -r gate",
    );
    let mut observer = window(&env);
    let mut live = Mirror::attach(&mut observer, pane, 1, 10000);
    input(&mut observer, pane, b"go\n");
    while live.output.len() < 5004 {
        live.step(&mut observer);
    }
    let mut waiting = window(&env);
    let started = std::time::Instant::now();
    waiting.send_request(1, "pane.attach", json!({"pane":pane}));
    rpc.ok("pane.send_text", json!({"pane":pane,"text":"go\n"}));
    eventually("shell progress before attach timeout", || {
        env.home().join("kept-running").exists().then_some(())
    });
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    let error = response(&mut waiting, 1).unwrap_err();
    assert_eq!(error.code, 1002);
    assert_eq!(error.data, Some(json!({"reason":"mid_sequence"})));
    assert!(started.elapsed() >= std::time::Duration::from_millis(1900));
    assert!(started.elapsed() < std::time::Duration::from_millis(3500));
    input(&mut observer, pane, b"go\n");
    live.until(&mut observer, "done");
    let fresh = Mirror::attach(&mut waiting, pane, 2, 10000);
    fresh.equals_capture(&mut rpc);
}

#[test]
fn a_stalled_mid_sequence_resync_sends_pane_detached_and_keeps_control_alive() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let pane = script_pane(
        &mut rpc,
        "stty -echo; printf 'ready\\n'; read -r gate; printf '\\033]0;'; head -c 10000000 /dev/zero | tr '\\0' x; : > flooded; read -r gate; printf '\\007done\\n'; read -r gate",
    );
    let mut slow = window(&env);
    let _stale = Mirror::attach(&mut slow, pane, 1, 10000);
    let mut fast = window(&env);
    let mut live = Mirror::attach(&mut fast, pane, 1, 10000);
    input(&mut fast, pane, b"go\n");
    while live.output.len() < 10000004 {
        live.step(&mut fast);
    }
    assert_eq!(live.replacements, 0);
    eventually("flooded shell", || {
        env.home().join("flooded").exists().then_some(())
    });
    let deadline = std::time::Instant::now() + shoal_testkit::TIMEOUT;
    loop {
        assert!(std::time::Instant::now() < deadline);
        match slow.recv() {
            Received::Binary(_) => {} // a frame already in the kernel cannot be retracted
            Received::Json(Message::Notification(notification))
                if notification.method == "pane.detached" =>
            {
                assert_eq!(
                    notification.params,
                    json!({"pane":pane,"reason":"mid_sequence"})
                );
                break;
            }
            Received::Json(Message::Notification(_)) => {}
            other => panic!("expected detach, got {other:?}"),
        }
    }
    slow.send_request(2, "daemon.status", json!({}));
    response(&mut slow, 2).unwrap();
    input(&mut fast, pane, b"go\n");
    live.until(&mut fast, "done");
    live.equals_capture(&mut rpc);
    let retried = Mirror::attach(&mut slow, pane, 3, 10000);
    retried.equals_capture(&mut rpc);
}

#[test]
fn attach_rejects_a_different_core_but_control_still_works() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let pane = pane(&mut rpc);
    let id = rpc.send("pane.attach", json!({"pane": pane}));
    let error = rpc.response(id).unwrap_err();
    assert_eq!(error.code, 1003);
    assert_eq!(
        error.data,
        Some(json!({"daemon_core": shoal_term::core_id()}))
    );
    assert_eq!(rpc.ok("daemon.status", json!({}))["panes"], 1);
}

#[test]
fn only_the_most_recent_focus_report_controls_size_and_input_reaches_the_shell() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut first = Rpc::new(&env);
    let pane = pane(&mut first);
    let mut second = Rpc::new(&env);
    let size = json!({"pane": pane, "cols": 100, "rows": 30});
    assert_eq!(
        first.ok("pane.set_size", size.clone()),
        json!({"cols":80,"rows":24})
    );
    first.ok("window.set_focused", json!({"focused":true}));
    assert_eq!(
        first.ok("pane.set_size", size.clone()),
        json!({"cols":100,"rows":30})
    );
    second.ok("window.set_focused", json!({"focused":true}));
    assert_eq!(
        first.ok("pane.set_size", json!({"pane":pane,"cols":60,"rows":10})),
        json!({"cols":100,"rows":30})
    );
    assert_eq!(
        second.ok("pane.set_size", json!({"pane":pane,"cols":90,"rows":28})),
        json!({"cols":90,"rows":28})
    );
    first.ok("window.set_focused", json!({"focused":true}));
    assert_eq!(
        first.ok("pane.set_size", json!({"pane":pane,"cols":95,"rows":29})),
        json!({"cols":95,"rows":29})
    );
    assert_eq!(
        second.ok("pane.set_size", json!({"pane":pane,"cols":60,"rows":10})),
        json!({"cols":95,"rows":29})
    );
    first.ok("pane.set_size", json!({"pane":pane,"cols":90,"rows":28}));
    first.error(
        "pane.set_size",
        json!({"pane":pane,"cols":0,"rows":10}),
        1004,
    );
    second.error(
        "pane.set_size",
        json!({"pane":pane,"cols":1001,"rows":10}),
        1004,
    );
    let mut input = window(&env);
    input.send_raw(
        &Frame::Input(Input {
            pane,
            bytes: b"printf 'binary-%s\\n' $((6*7)); stty size\r",
        })
        .encode()
        .unwrap(),
    );
    eventually("binary input and PTY resize", || {
        let capture = first.ok(
            "pane.capture",
            json!({"pane":pane,"format":"text","scrollback":true}),
        );
        let text = capture["data"].as_str().unwrap();
        (text.contains("binary-42") && text.contains("28 90")).then_some(())
    });
}
