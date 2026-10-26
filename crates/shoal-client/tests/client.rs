//! T20: every client feature against the real `shoald` in a temporary HOME
//! and XDG environment. Auto-start strategies here spawn the test daemon;
//! nothing calls `systemctl`.

use std::{
    fs,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc::Receiver,
    },
    time::{Duration, Instant},
};

use serde_json::json;
use shoal_client::{
    AttachStream, Client, Connected, Error, Event, NoAutoStart, Options, StreamItem,
};
use shoal_model::{GridSize, PaneId, SessionId};
use shoal_proto::{
    Call, CaptureFormat, ClientKind, Empty, NoticeLevel, Notification, WorkspaceState, error_code,
};
use shoal_testkit::{Daemon, Signal, TIMEOUT, TempEnv, shoald};

fn fixture() -> TempEnv {
    let env = TempEnv::new();
    fs::create_dir_all(env.config_home().join("shoal")).unwrap();
    fs::write(env.config_home().join("shoal/config.toml"), CONFIG).unwrap();
    fs::write(
        env.home().join(".bashrc"),
        "PS1='fixture> '; unset PROMPT_COMMAND\n",
    )
    .unwrap();
    env
}

const CONFIG: &str =
    "[shell]\nprogram = '/bin/bash'\nintegration = 'off'\n[prompt]\nmode = 'shell'\n";

fn options(env: &TempEnv, client: ClientKind, core: Option<String>) -> Options {
    let mut options = Options::new(env.socket(), client);
    options.core = core;
    options
}

fn cli(env: &TempEnv) -> Connected {
    Client::connect(options(env, ClientKind::Cli, None), NoAutoStart).unwrap()
}

/// A window client with the daemon's own core, so it can attach.
fn gui(env: &TempEnv) -> Connected {
    let core = cli(env).hello.core;
    Client::connect(options(env, ClientKind::Gui, Some(core)), NoAutoStart).unwrap()
}

fn workspace(client: &Client) -> WorkspaceState {
    serde_json::from_value(client.request(Call::WorkspaceGet(Empty {})).unwrap()).unwrap()
}

/// A new session's only pane.
fn new_pane(client: &Client) -> PaneId {
    let created = client
        .request(Call::SessionCreate {
            name: None,
            cwd: None,
        })
        .unwrap();
    let session: SessionId = serde_json::from_value(created["session"].clone()).unwrap();
    let state = workspace(client);
    let session = state
        .workspace
        .sessions
        .iter()
        .find(|item| item.id == session)
        .unwrap();
    session.tabs[0].focused
}

fn screen(client: &Client, pane: PaneId) -> String {
    let captured = client
        .request(Call::PaneCapture {
            pane,
            format: CaptureFormat::Text,
            scrollback: true,
        })
        .unwrap();
    captured["data"].as_str().unwrap().to_owned()
}

fn next_event(
    events: &Receiver<Event>,
    what: &str,
    mut wanted: impl FnMut(&Event) -> bool,
) -> Event {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let event = events
            .recv_timeout(left)
            .unwrap_or_else(|error| panic!("waiting for {what}: {error}"));
        if wanted(&event) {
            return event;
        }
    }
}

fn next_item(stream: &AttachStream) -> StreamItem {
    stream
        .recv_timeout(TIMEOUT)
        .unwrap()
        .expect("a stream item in time")
}

/// The first item must be a snapshot; returns its sequence number.
fn snapshot(stream: &AttachStream) -> u64 {
    match next_item(stream) {
        StreamItem::Snapshot { seq, .. } => seq,
        other => panic!("expected a snapshot, got {other:?}"),
    }
}

/// Collect output after `seq` until it contains `needle`, checking that the
/// items are numbered without gaps. Returns the last sequence number.
fn output_until(stream: &AttachStream, mut seq: u64, needle: &str) -> u64 {
    let mut text = Vec::new();
    while !String::from_utf8_lossy(&text).contains(needle) {
        match next_item(stream) {
            StreamItem::Output { seq: got, bytes } => {
                assert_eq!(got, seq + 1, "in order");
                seq = got;
                text.extend_from_slice(&bytes);
            }
            StreamItem::Resized { seq: got, .. } => {
                assert_eq!(got, seq + 1, "in order");
                seq = got;
            }
            other => panic!("expected output, got {other:?}"),
        }
    }
    seq
}

#[test]
fn auto_start_runs_once_when_nothing_is_listening() {
    let env = Arc::new(fixture());
    let started: Arc<Mutex<Option<Daemon>>> = Arc::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let strategy = {
        let (env, started, calls) = (env.clone(), started.clone(), calls.clone());
        move |requested: &Path| {
            assert_eq!(requested, env.socket());
            calls.fetch_add(1, Ordering::SeqCst);
            *started.lock().unwrap() = Some(env.start_daemon(shoald()));
            Ok(())
        }
    };
    let connected = Client::connect(options(&env, ClientKind::Cli, None), strategy).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let pid = started.lock().unwrap().as_ref().unwrap().pid();
    assert_eq!(connected.hello.pid, pid);
    assert_eq!(connected.hello.protocol, 1);

    // A running daemon needs no start.
    let never = |_: &Path| -> std::io::Result<()> { panic!("not needed") };
    let again = Client::connect(options(&env, ClientKind::Cli, None), never).unwrap();
    assert_eq!(again.hello.pid, pid);
}

#[test]
fn without_a_daemon_connecting_fails_after_at_most_two_seconds_of_waiting() {
    let env = fixture();
    let error = Client::connect(options(&env, ClientKind::Cli, None), NoAutoStart).unwrap_err();
    assert!(matches!(error, Error::Connect(_)), "{error:?}");
    let started = Instant::now();
    let error =
        Client::connect(options(&env, ClientKind::Cli, None), |_: &Path| Ok(())).unwrap_err();
    assert!(matches!(error, Error::Connect(_)), "{error:?}");
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_secs(2) && waited < TIMEOUT,
        "{waited:?}"
    );
}

#[test]
fn requests_from_many_threads_get_their_own_answers() {
    let env = fixture();
    let _daemon = env.spawn_daemon(shoald());
    let client = Arc::new(cli(&env).client);
    let threads: Vec<_> = (0..8)
        .map(|i| {
            let client = client.clone();
            std::thread::spawn(move || {
                let name = format!("thread-{i}");
                let created = client
                    .request(Call::SessionCreate {
                        name: Some(name.clone()),
                        cwd: None,
                    })
                    .unwrap();
                let session: SessionId =
                    serde_json::from_value(created["session"].clone()).unwrap();
                (name, session)
            })
        })
        .collect();
    let answers: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    let state = workspace(&client);
    for (name, session) in answers {
        // Each answer named the session its own request created.
        let found = state
            .workspace
            .sessions
            .iter()
            .find(|item| item.id == session);
        assert_eq!(found.map(|item| item.name.clone()), Some(name));
    }
    let error = client
        .request(Call::PaneClose {
            pane: PaneId::new(999),
        })
        .unwrap_err();
    assert_eq!(error.code(), Some(error_code::NOT_FOUND), "{error:?}");
}

#[test]
fn a_request_times_out_and_its_late_answer_goes_nowhere() {
    let env = fixture();
    let daemon = env.spawn_daemon(shoald());
    let client = cli(&env).client;
    daemon.signal(Signal::STOP);
    let started = Instant::now();
    let pending = client.send_with_timeout(
        Call::SessionCreate {
            name: Some("late".into()),
            cwd: None,
        },
        Duration::from_millis(300),
    );
    let result = pending.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(result, Err(Error::Timeout(Duration::from_millis(300))));
    assert!(started.elapsed() >= Duration::from_millis(300));
    daemon.signal(Signal::CONT);
    // The late session answer must not be taken for this one.
    let status = client.request(Call::DaemonStatus(Empty {})).unwrap();
    assert_eq!(status["pid"], json!(daemon.pid()));
}

#[test]
fn notifications_arrive_on_the_event_channel() {
    let env = fixture();
    let _daemon = env.spawn_daemon(shoald());
    let Connected { client, events, .. } = cli(&env);
    client.subscribe_workspace().unwrap();
    next_event(&events, "the initial workspace", |event| {
        matches!(
            event,
            Event::Notification(Notification::WorkspaceChanged(_))
        )
    });
    new_pane(&client);
    next_event(&events, "the new session", |event| {
        matches!(event, Event::Notification(Notification::WorkspaceChanged(state))
            if state.workspace.sessions.len() == 1)
    });
    fs::write(
        env.config_home().join("shoal/config.toml"),
        format!("{CONFIG}[scrollback]\nlines = -1\n"),
    )
    .unwrap();
    next_event(&events, "the config notice", |event| {
        matches!(
            event,
            Event::Notification(Notification::DaemonNotice {
                level: NoticeLevel::Error,
                ..
            })
        )
    });
}

#[test]
fn attach_streams_a_snapshot_then_numbered_output_and_resizes() {
    let env = fixture();
    let _daemon = env.spawn_daemon(shoald());
    let Connected { client, .. } = gui(&env);
    let pane = new_pane(&client);
    let stream = client.attach(pane);
    assert_eq!(stream.pane(), pane);
    let seq = snapshot(&stream);
    client.input(pane, b"printf 'typed-%s\\n' input\r".to_vec());
    let seq = output_until(&stream, seq, "typed-input");
    client
        .request(Call::WindowSetFocused { focused: true })
        .unwrap();
    client
        .request(Call::PaneSetSize {
            pane,
            cols: 100,
            rows: 30,
        })
        .unwrap();
    let mut last = seq;
    loop {
        match next_item(&stream) {
            StreamItem::Resized { seq, size } => {
                assert_eq!(seq, last + 1);
                assert_eq!(
                    size,
                    GridSize {
                        cols: 100,
                        rows: 30
                    }
                );
                break;
            }
            StreamItem::Output { seq, .. } => {
                assert_eq!(seq, last + 1);
                last = seq;
            }
            other => panic!("expected the resize, got {other:?}"),
        }
    }
    // Attaching again replaces the first stream, which ends.
    let second = client.attach(pane);
    snapshot(&second);
    // Items already delivered to it can still be read; then it ends.
    loop {
        match stream.recv_timeout(TIMEOUT) {
            Err(Error::Stopped) => break,
            Ok(Some(StreamItem::Output { .. } | StreamItem::Resized { .. })) => {}
            other => panic!("expected the first stream to end, got {other:?}"),
        }
    }
}

#[test]
fn dropping_a_split_streams_detach_ends_its_items() {
    let env = fixture();
    let _daemon = env.spawn_daemon(shoald());
    let Connected { client, .. } = gui(&env);
    let pane = new_pane(&client);
    let (items, detach) = client.attach(pane).split();
    assert_eq!(detach.pane(), pane);
    let (forward, forwarded) = std::sync::mpsc::channel();
    // Blocks on the items, with nothing else to wake it, until they end.
    std::thread::spawn(move || {
        while let Ok(item) = items.recv() {
            let _ = forward.send(item);
        }
    });
    assert!(matches!(
        forwarded.recv_timeout(TIMEOUT),
        Ok(StreamItem::Snapshot { .. })
    ));
    drop(detach);
    loop {
        match forwarded.recv_timeout(TIMEOUT) {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(StreamItem::Output { .. }) => {}
            other => panic!("expected the items to end, got {other:?}"),
        }
    }
}

#[test]
fn attach_failures_end_the_stream() {
    let env = fixture();
    let _daemon = env.spawn_daemon(shoald());
    let Connected { client, .. } = cli(&env);
    let pane = new_pane(&client);
    // The CLI sends no core, so it may not decode snapshots.
    match next_item(&client.attach(pane)) {
        StreamItem::Failed(error) => {
            assert_eq!(
                error.code(),
                Some(error_code::VERSION_MISMATCH),
                "{error:?}"
            );
        }
        other => panic!("expected a failure, got {other:?}"),
    }
    let Connected { client, .. } = gui(&env);
    let stream = client.attach(PaneId::new(999));
    match next_item(&stream) {
        StreamItem::Failed(error) => assert_eq!(error.code(), Some(error_code::NOT_FOUND)),
        other => panic!("expected a failure, got {other:?}"),
    }
    assert!(stream.recv_timeout(TIMEOUT).is_err(), "nothing follows");
}

#[test]
fn after_the_daemon_is_killed_the_client_reconnects_and_reattaches() {
    let env = fixture();
    let daemon = env.spawn_daemon(shoald());
    let Connected {
        client,
        events,
        hello,
    } = gui(&env);
    client.subscribe_workspace().unwrap();
    let pane = new_pane(&client);
    let stream = client.attach(pane);
    let seq = snapshot(&stream);
    client.input(pane, b"printf 'before-%s\\n' kill\r".to_vec());
    output_until(&stream, seq, "before-kill");
    client.request(Call::DaemonPersist(Empty {})).unwrap();

    daemon.signal(Signal::KILL);
    drop(daemon.wait());
    next_event(&events, "the disconnect", |event| {
        matches!(event, Event::Disconnected(_))
    });
    assert_eq!(
        client.request(Call::WorkspaceGet(Empty {})),
        Err(Error::NotConnected)
    );

    let _daemon = env.spawn_daemon(shoald());
    let Event::Connected(again) = next_event(&events, "the reconnect", |event| {
        matches!(event, Event::Connected(_))
    }) else {
        unreachable!()
    };
    assert_ne!(again.pid, hello.pid);
    // The subscription resumes, with the restored session.
    next_event(&events, "the restored workspace", |event| {
        matches!(event, Event::Notification(Notification::WorkspaceChanged(state))
            if state.workspace.panes.contains_key(&pane))
    });
    // The stream resumes from a fresh snapshot of the restored pane.
    let seq = snapshot(&stream);
    assert!(screen(&client, pane).contains("before-kill"));
    client.input(pane, b"printf 'after-%s\\n' restart\r".to_vec());
    output_until(&stream, seq, "after-restart");
}

#[test]
fn without_reconnecting_a_lost_daemon_ends_streams() {
    let env = fixture();
    let daemon = env.spawn_daemon(shoald());
    let core = cli(&env).hello.core;
    let mut options = options(&env, ClientKind::Gui, Some(core));
    options.reconnect = false;
    let Connected { client, events, .. } = Client::connect(options, NoAutoStart).unwrap();
    let pane = new_pane(&client);
    let stream = client.attach(pane);
    snapshot(&stream);
    client.request(Call::DaemonShutdown(Empty {})).unwrap();
    assert!(daemon.wait().status.success());
    next_event(&events, "the disconnect", |event| {
        matches!(event, Event::Disconnected(_))
    });
    assert_eq!(next_item(&stream), StreamItem::Failed(Error::NotConnected));
    assert_eq!(
        client.request(Call::WorkspaceGet(Empty {})),
        Err(Error::NotConnected)
    );
    assert!(matches!(
        client.attach(pane).recv(),
        Some(StreamItem::Failed(Error::NotConnected))
    ));
}
