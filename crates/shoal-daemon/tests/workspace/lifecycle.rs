use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    time::Duration,
};

use serde_json::{Value, json};
use shoal_proto::Frame;
use shoal_testkit::eventually;

use crate::{
    create,
    support::{Rpc, SHOALD, fixture},
};

fn frame(id: u64, method: &str, params: Value) -> Vec<u8> {
    let Value::Object(object) =
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
    else {
        unreachable!()
    };
    Frame::Json(object).encode().unwrap()
}

fn exchange(stream: &mut UnixStream, id: u64, method: &str, params: Value) {
    stream.write_all(&frame(id, method, params)).unwrap();
    let mut length = [0; 4];
    stream.read_exact(&mut length).unwrap();
    let mut payload = vec![0; u32::from_le_bytes(length) as usize];
    stream.read_exact(&mut payload).unwrap();
    assert_eq!(payload[0], 1);
    let response: Value = serde_json::from_slice(&payload[1..]).unwrap();
    assert_eq!(response["id"], id);
    assert!(response.get("result").is_some(), "{response}");
}

#[test]
fn an_empty_workspace_shutdown_delivers_success_before_closing() {
    let env = fixture(false);
    let daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    assert_eq!(rpc.ok("daemon.shutdown", json!({})), json!({}));
    assert!(daemon.wait().status.success());
    assert!(!env.socket().exists());
}

#[test]
fn a_saturated_control_connection_is_closed_even_while_its_writer_is_blocked() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut healthy = Rpc::new(&env);
    let (session, _, _) = create(&mut healthy);
    // Valid large state makes a non-reading peer fill its kernel socket buffer
    // before the bounded control queue overflows. No pane-stream behavior here.
    healthy.ok(
        "session.rename",
        json!({"session": session, "name": "x".repeat(128 * 1024)}),
    );
    let mut stalled = UnixStream::connect(env.socket()).unwrap();
    stalled
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stalled
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    exchange(
        &mut stalled,
        1,
        "hello",
        json!({"protocol": 1, "client": "cli", "version": "0.1.0", "env": {}}),
    );
    exchange(
        &mut stalled,
        2,
        "window.show_session",
        json!({"session": session}),
    );
    assert_eq!(healthy.workspace()["sessions"][0]["windows"], 1);
    let requests: Vec<u8> = (3..259)
        .flat_map(|id| frame(id, "workspace.get", json!({})))
        .collect();
    if let Err(error) = stalled.write_all(&requests) {
        assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
            ),
            "{error}"
        );
    }
    eventually("overflow releases the window", || {
        (healthy.workspace()["sessions"][0]["windows"] == 0).then_some(())
    });
    // POLLHUP is visible without draining readable bytes. Reading the peer here
    // would unblock the defective writer and conceal the resource leak.
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    eventually("overflow closes the blocked socket", || {
        let mut descriptors = [PollFd::new(&stalled, PollFlags::IN)];
        poll(
            &mut descriptors,
            Some(&Timespec::try_from(Duration::ZERO).unwrap()),
        )
        .unwrap();
        descriptors[0]
            .revents()
            .contains(PollFlags::HUP)
            .then_some(())
    });
    assert_eq!(healthy.ok("daemon.status", json!({}))["sessions"], 1);
}
