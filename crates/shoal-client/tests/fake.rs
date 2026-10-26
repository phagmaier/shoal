//! T20: what a correct `shoald` never does, from a scripted fake daemon on a
//! temporary socket: a sequence gap, a refused `hello`, notifications ahead of
//! `hello`'s answer, a protocol error, and reconnect backoff.

use std::{
    io::{Read as _, Write as _},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use shoal_client::{Client, Connected, Error, Event, NoAutoStart, Options, StreamItem};
use shoal_model::{GridSize, PaneId};
use shoal_proto::{
    ClientKind, DetachReason, Frame, Input, Message, NoticeLevel, Notification, Output, Snapshot,
    decode, error_code,
};
use shoal_testkit::TIMEOUT;

struct Fake {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    listener: UnixListener,
}

impl Fake {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("shoald.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        Self {
            _dir: dir,
            socket,
            listener,
        }
    }

    fn options(&self) -> Options {
        let mut options = Options::new(&self.socket, ClientKind::Gui);
        options.core = Some("fake-core".into());
        options
    }

    fn accept(&self) -> Peer {
        let (stream, _) = self.listener.accept().unwrap();
        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
        Peer {
            stream,
            buffer: Vec::new(),
        }
    }
}

struct Peer {
    stream: UnixStream,
    buffer: Vec<u8>,
}

impl Peer {
    /// The next request's id, method and parameters.
    fn request(&mut self) -> (Value, String, Value) {
        loop {
            if let Some((frame, used)) = decode(&self.buffer).unwrap() {
                let Frame::Json(object) = frame else {
                    panic!("expected a request, got {frame:?}")
                };
                self.buffer.drain(..used);
                let Message::Request(request) = Message::from_object(object).unwrap() else {
                    panic!("expected a request")
                };
                return (
                    serde_json::to_value(&request.id).unwrap(),
                    request.method,
                    request.params,
                );
            }
            let mut chunk = [0; 4096];
            let read = self.stream.read(&mut chunk).unwrap();
            assert!(read > 0, "the client closed the connection");
            self.buffer.extend_from_slice(&chunk[..read]);
        }
    }

    fn expect(&mut self, method: &str) -> Value {
        let (id, got, _) = self.request();
        assert_eq!(got, method);
        id
    }

    fn json(&mut self, message: Value) {
        let Value::Object(object) = message else {
            panic!("an object")
        };
        self.frame(Frame::Json(object));
    }

    fn frame(&mut self, frame: Frame<'_>) {
        self.stream.write_all(&frame.encode().unwrap()).unwrap();
    }

    fn result(&mut self, id: &Value, result: Value) {
        self.json(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    fn hello(&mut self, pid: u32) {
        let id = self.expect("hello");
        self.result(
            &id,
            json!({"protocol": 1, "daemon_version": "0.1.0", "core": "fake-core", "pid": pid}),
        );
    }

    fn snapshot(&mut self, seq: u64, bytes: &[u8]) {
        self.frame(Frame::Snapshot(Snapshot {
            pane: PANE,
            seq,
            size: SIZE,
            bytes,
        }));
    }

    fn output(&mut self, seq: u64, bytes: &[u8]) {
        self.frame(Frame::Output(Output {
            pane: PANE,
            seq,
            bytes,
        }));
    }
}

const PANE: PaneId = PaneId::new(3);
const SIZE: GridSize = GridSize { cols: 80, rows: 24 };

/// Connect on another thread while `serve` plays the daemon's part.
fn connect(fake: &Fake, serve: impl FnOnce(&mut Peer)) -> (Result<Connected, Error>, Peer) {
    let options = fake.options();
    let client = std::thread::spawn(move || Client::connect(options, NoAutoStart));
    let mut peer = fake.accept();
    serve(&mut peer);
    (client.join().unwrap(), peer)
}

fn item(stream: &shoal_client::AttachStream) -> StreamItem {
    stream
        .recv_timeout(TIMEOUT)
        .unwrap()
        .expect("an item in time")
}

#[test]
fn a_gap_drops_items_until_a_fresh_snapshot_from_reattaching() {
    let fake = Fake::new();
    let (connected, mut peer) = connect(&fake, |peer| {
        // Notices may come ahead of hello's answer; they wait for it.
        let id = peer.expect("hello");
        peer.json(json!({"jsonrpc": "2.0", "method": "daemon.notice", "params": {"level": "warn", "message": "early"}}));
        peer.result(
            &id,
            json!({"protocol": 1, "daemon_version": "0.1.0", "core": "fake-core", "pid": 7}),
        );
    });
    let Connected {
        client,
        events,
        hello,
    } = connected.unwrap();
    assert_eq!(hello.pid, 7);
    assert_eq!(
        events.recv_timeout(TIMEOUT).unwrap(),
        Event::Notification(Notification::DaemonNotice {
            level: NoticeLevel::Warn,
            message: "early".into(),
            key: None
        })
    );

    let stream = client.attach(PANE);
    let (id, method, params) = peer.request();
    assert_eq!(
        (method.as_str(), params),
        ("pane.attach", json!({"pane": "p3"}))
    );
    peer.snapshot(10, b"first");
    peer.result(&id, json!({"seq": 10, "cols": 80, "rows": 24}));
    peer.output(11, b"a");
    peer.output(13, b"c");
    peer.output(14, b"d");
    // The gap makes the client attach again.
    let id = peer.expect("pane.attach");
    peer.snapshot(20, b"second");
    peer.result(&id, json!({"seq": 20, "cols": 80, "rows": 24}));
    peer.output(21, b"e");

    let snapshot = |seq, bytes: &[u8]| StreamItem::Snapshot {
        seq,
        size: SIZE,
        bytes: bytes.to_vec(),
    };
    let output = |seq, bytes: &[u8]| StreamItem::Output {
        seq,
        bytes: bytes.to_vec(),
    };
    assert_eq!(item(&stream), snapshot(10, b"first"));
    assert_eq!(item(&stream), output(11, b"a"));
    assert_eq!(item(&stream), snapshot(20, b"second"));
    assert_eq!(item(&stream), output(21, b"e"));

    peer.json(json!({"jsonrpc": "2.0", "method": "pane.detached", "params": {"pane": "p3", "reason": "mid_sequence"}}));
    assert_eq!(
        item(&stream),
        StreamItem::Detached(DetachReason::MidSequence)
    );
    assert!(stream.recv_timeout(TIMEOUT).is_err(), "the stream ended");
    // pane.detached goes to the stream, not the event channel.
    assert!(events.try_recv().is_err());
}

#[test]
fn a_refused_hello_fails_the_connect() {
    let fake = Fake::new();
    let (connected, _peer) = connect(&fake, |peer| {
        let id = peer.expect("hello");
        peer.json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": 1003, "message": "protocol mismatch", "data": {"daemon_protocol": 2, "pid": 9}}}));
    });
    let error = connected.unwrap_err();
    assert_eq!(
        error.code(),
        Some(error_code::VERSION_MISMATCH),
        "{error:?}"
    );
}

#[test]
fn a_protocol_error_disconnects_and_the_client_reconnects_after_backing_off() {
    let fake = Fake::new();
    let (connected, mut peer) = connect(&fake, |peer| peer.hello(1));
    let Connected { client, events, .. } = connected.unwrap();
    std::thread::scope(|scope| {
        let subscribed = scope.spawn(|| client.subscribe_workspace());
        let id = peer.expect("workspace.subscribe");
        peer.result(&id, json!({}));
        subscribed.join().unwrap().unwrap();
    });
    // Now break the protocol.
    peer.frame(Frame::Input(Input {
        pane: PANE,
        bytes: b"x",
    }));
    let lost = Instant::now();
    match events.recv_timeout(TIMEOUT).unwrap() {
        Event::Disconnected(Error::Protocol(_)) => {}
        other => panic!("expected a protocol error, got {other:?}"),
    }
    // The first retry waits 100 ms.
    let mut peer = fake.accept();
    assert!(
        lost.elapsed() >= Duration::from_millis(90),
        "{:?}",
        lost.elapsed()
    );
    peer.hello(2);
    match events.recv_timeout(TIMEOUT).unwrap() {
        Event::Connected(hello) => assert_eq!(hello.pid, 2),
        other => panic!("expected the reconnect, got {other:?}"),
    }
    // The subscription resumes by itself.
    peer.expect("workspace.subscribe");
}
