//! T22: stopping, restarting and mismatch detection against the real
//! `shoald`. Strategies here spawn the test daemon; nothing in this process
//! runs `systemctl` (the CLI's tests cover that path with a fake).

use std::sync::{Arc, Mutex};

use shoal_client::{
    Client, NoAutoStart, Options,
    lifecycle::{self, Mismatch, Stopped},
    proto::{Call, ClientKind, Empty, WorkspaceState},
};
use shoal_testkit::{Daemon, TempEnv, eventually, shoald};

fn options(env: &TempEnv) -> Options {
    let mut options = Options::new(env.socket(), ClientKind::Cli);
    options.reconnect = false;
    options
}

fn sessions(client: &Client) -> Vec<String> {
    let state: WorkspaceState =
        serde_json::from_value(client.request(Call::WorkspaceGet(Empty {})).unwrap()).unwrap();
    state
        .workspace
        .sessions
        .into_iter()
        .map(|session| session.name)
        .collect()
}

fn create(client: &Client, name: &str) {
    client
        .request(Call::SessionCreate {
            name: Some(name.into()),
            cwd: None,
        })
        .unwrap();
}

#[test]
fn restart_saves_stops_and_starts_a_daemon_that_restores() {
    let env = Arc::new(TempEnv::new());
    let first = env.spawn_daemon(shoald());
    let connected = Client::connect(options(&env), NoAutoStart).unwrap();
    create(&connected.client, "kept");
    let started: Arc<Mutex<Option<Daemon>>> = Arc::default();
    let strategy = {
        let (env, started) = (env.clone(), started.clone());
        move |_: &std::path::Path| {
            *started.lock().unwrap() = Some(env.start_daemon(shoald()));
            Ok(())
        }
    };
    let again = lifecycle::restart(options(&env), strategy).unwrap();
    assert!(
        first.wait().status.success(),
        "stopped through its shutdown"
    );
    assert_ne!(again.hello.pid, connected.hello.pid);
    assert_eq!(
        again.hello.pid,
        started.lock().unwrap().as_ref().unwrap().pid()
    );
    assert_eq!(sessions(&again.client), ["kept"]);
}

#[test]
fn stopping_reports_whether_anything_ran() {
    let env = TempEnv::new();
    assert_eq!(lifecycle::stop(&options(&env)), Ok(Stopped::NotRunning));
    let daemon = env.spawn_daemon(shoald());
    assert_eq!(lifecycle::stop(&options(&env)), Ok(Stopped::Stopped));
    assert!(!env.socket().exists());
    assert!(daemon.wait().status.success());
}

#[test]
fn sigterm_saves_like_a_shutdown() {
    let env = TempEnv::new();
    let daemon = env.spawn_daemon(shoald());
    let connected = Client::connect(options(&env), NoAutoStart).unwrap();
    create(&connected.client, "saved");
    lifecycle::signal_stop(connected.hello.pid).unwrap();
    assert!(daemon.wait().status.success());
    eventually("the socket to go", || {
        (!env.socket().exists()).then_some(())
    });
    let saved = std::fs::read_to_string(env.state_home().join("shoal/workspace.json")).unwrap();
    assert!(saved.contains("\"saved\""), "{saved}");
}

#[test]
fn hello_shows_core_and_version_mismatches() {
    let env = TempEnv::new();
    let _daemon = env.spawn_daemon(shoald());
    let mut newer = options(&env);
    newer.version = "9.9.9".into();
    let connected = Client::connect(newer.clone(), NoAutoStart).unwrap();
    assert_eq!(
        Mismatch::of_hello(&newer, &connected.hello),
        Some(Mismatch::Version {
            daemon_version: env!("CARGO_PKG_VERSION").into()
        })
    );
    let mut window = options(&env);
    window.client = ClientKind::Gui;
    window.core = Some("another-build".into());
    let connected = Client::connect(window.clone(), NoAutoStart).unwrap();
    let daemon_core = connected.hello.core.clone();
    assert_eq!(
        Mismatch::of_hello(&window, &connected.hello),
        Some(Mismatch::Core {
            daemon_core: daemon_core.clone()
        })
    );
    // Attaching is what the daemon refuses, with the same answer.
    let stream = connected.client.attach(shoal_client::model::PaneId::new(1));
    match stream.recv() {
        Some(shoal_client::StreamItem::Failed(error)) => {
            assert_eq!(
                Mismatch::of_error(&error),
                Some(Mismatch::Core { daemon_core })
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}
