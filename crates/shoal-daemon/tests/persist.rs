//! T15: the persistence writer (M1 spec §6.1–6.3, §4.5) through the real
//! daemon, in a temporary HOME and XDG environment.

#[allow(dead_code)]
mod support;

use std::{
    fs,
    io::Write as _,
    os::unix::{fs::PermissionsExt as _, process::ExitStatusExt as _},
    path::Path,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use shoal_daemon::persist::write_atomic;
use shoal_testkit::{Signal, eventually};
use support::{
    Rpc, SHOALD, config, fixture, print, saved_text, saved_workspace, screen, scrollback, session,
    state, wait_for,
};

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn session_with(rpc: &mut Rpc, marker: &str) -> Value {
    let pane = session(rpc);
    print(rpc, &pane, marker);
    pane
}

#[test]
fn workspace_json_matches_format_one_and_closed_panes_are_cleaned_up() {
    let env = fixture(false);
    let project = env.home().join("project");
    fs::create_dir(&project).unwrap();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    assert!(rpc.ok("daemon.status", json!({}))["last_persist_at"].is_null());
    let session = rpc.ok("session.create", json!({"cwd": project}))["session"].clone();
    let first = rpc.workspace()["sessions"][0]["tabs"][0]["focused"].clone();
    let second = rpc.ok("pane.split", json!({"pane": first, "direction": "right"}))["pane"].clone();
    let split = rpc.workspace()["sessions"][0]["tabs"][0]["layout"]["split"]["id"].clone();
    rpc.ok("split.set_ratio", json!({"split": split, "ratio": 0.6}));
    rpc.ok("pane.zoom", json!({"pane": second, "zoomed": true}));
    let tab = rpc.ok(
        "tab.create",
        json!({"session": session, "argv": ["sleep", "1000"]}),
    )["tab"]
        .clone();
    rpc.ok("tab.rename", json!({"tab": tab, "name": "waiting"}));
    assert_eq!(rpc.ok("daemon.persist", json!({})), json!({}));
    assert!(rpc.ok("daemon.status", json!({}))["last_persist_at"].is_string());

    // The file as written, field order included, with times and HOME redacted.
    let document = saved_workspace(&env);
    let mut text = fs::read_to_string(state(&env).join("workspace.json")).unwrap();
    let session = &document["sessions"][0];
    for time in [
        &document["saved_at"],
        &session["created_at"],
        &session["last_active_at"],
    ] {
        text = text.replace(time.as_str().unwrap(), "[time]");
    }
    insta::assert_snapshot!(text.replace(env.home().to_str().unwrap(), "~"));

    assert_eq!(mode(&state(&env)), 0o700);
    assert_eq!(mode(&state(&env).join("workspace.json")), 0o600);
    let pane_folder = state(&env).join(format!("panes/{}", second.as_str().unwrap()));
    assert_eq!(mode(&pane_folder), 0o700);
    assert_eq!(mode(&scrollback(&env, &second)), 0o600);
    rpc.ok("pane.close", json!({"pane": second}));
    rpc.ok("daemon.persist", json!({}));
    assert!(!pane_folder.exists());
    assert!(scrollback(&env, &first).exists());
}

#[test]
fn scrollback_is_saved_after_two_quiet_seconds() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let pane = session_with(&mut rpc, "marker-quiet");
    let shown = Instant::now();
    let file = scrollback(&env, &pane);
    eventually("the scrollback file", || file.exists().then_some(()));
    // Output ended no earlier than the marker appeared, less a polling step.
    assert!(
        shown.elapsed() >= Duration::from_millis(1800),
        "{:?}",
        shown.elapsed()
    );
    assert!(saved_text(&file).contains("marker-quiet"));
    eventually("the layout to reference it", || {
        let saved = saved_workspace(&env);
        (saved["panes"][pane.as_str().unwrap()]["scrollback"]
            == json!(format!(
                "panes/{}/scrollback.vt.zst",
                pane.as_str().unwrap()
            )))
        .then_some(())
    });
}

#[test]
fn a_full_screen_program_saves_the_shell_screen_beneath_it() {
    let env = fixture(false);
    fs::write(env.home().join("notes.txt"), "only-in-nvim-77\n").unwrap();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let pane = session_with(&mut rpc, "marker-under");
    rpc.ok(
        "pane.send_text",
        json!({"pane": pane, "text": "nvim -u NONE -i NONE -n notes.txt\n"}),
    );
    wait_for(&mut rpc, &pane, "only-in-nvim-77");
    rpc.ok("daemon.persist", json!({}));
    let saved = saved_text(&scrollback(&env, &pane));
    assert!(saved.contains("marker-under"), "{saved:?}");
    assert!(
        saved.contains("nvim -u NONE -i NONE -n notes.txt"),
        "{saved:?}"
    );
    assert!(!saved.contains("only-in-nvim-77"), "{saved:?}");
    // The live terminal is untouched.
    assert!(screen(&mut rpc, &pane).contains("only-in-nvim-77"));
}

#[test]
fn disabling_persistence_deletes_scrollback_but_keeps_saving_the_layout() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let pane = session_with(&mut rpc, "marker-private");
    rpc.ok("daemon.persist", json!({}));
    let file = scrollback(&env, &pane);
    assert!(saved_text(&file).contains("marker-private"));
    fs::write(
        env.config_home().join("shoal/config.toml"),
        format!("{}[scrollback]\npersist = false\n", config(false)),
    )
    .unwrap();
    eventually("the scrollback file to be deleted", || {
        (!file.exists()).then_some(())
    });
    rpc.ok(
        "session.rename",
        json!({"session": "s1", "name": "renamed"}),
    );
    rpc.ok(
        "pane.send_text",
        json!({"pane": pane, "text": "printf 'more-%s\\n' output\n"}),
    );
    wait_for(&mut rpc, &pane, "more-output\n");
    rpc.ok("daemon.persist", json!({}));
    let saved = saved_workspace(&env);
    assert_eq!(saved["sessions"][0]["name"], "renamed");
    assert!(saved["panes"][pane.as_str().unwrap()]["scrollback"].is_null());
    assert!(!file.exists());
}

#[test]
fn shutdown_saves_every_pane_before_ending_shells() {
    for signal in [None, Some(Signal::TERM), Some(Signal::INT)] {
        let env = fixture(false);
        let daemon = env.spawn_daemon(SHOALD);
        let mut rpc = Rpc::new(&env);
        let first = session(&mut rpc);
        let second = session(&mut rpc);
        print(&mut rpc, &first, "marker-first");
        print(&mut rpc, &second, "marker-second");
        // Still within the quiet period: only the shutdown save writes these.
        assert!(!scrollback(&env, &first).exists());
        match signal {
            Some(signal) => daemon.signal(signal),
            None => {
                rpc.ok("daemon.shutdown", json!({}));
            }
        }
        assert!(daemon.wait().status.success());
        assert!(!env.socket().exists());
        let saved = saved_workspace(&env);
        assert_eq!(saved["sessions"].as_array().unwrap().len(), 2, "{signal:?}");
        for (pane, marker) in [(first, "marker-first"), (second, "marker-second")] {
            let reference = &saved["panes"][pane.as_str().unwrap()]["scrollback"];
            assert!(reference.is_string(), "{signal:?}: {saved}");
            assert!(
                saved_text(&state(&env).join(reference.as_str().unwrap())).contains(marker),
                "{signal:?}"
            );
        }
    }
}

const CRASH_TARGET: &str = "SHOAL_TEST_CRASH_TARGET";

#[test]
fn a_crash_during_a_write_leaves_the_previous_file_intact() {
    if let Some(target) = std::env::var_os(CRASH_TARGET) {
        // The child: die partway through filling the temporary file.
        let _ = write_atomic(Path::new(&target), |file| {
            file.write_all(b"half of the new")?;
            file.sync_all()?;
            std::process::abort()
        });
        unreachable!("the write aborts");
    }
    let folder = tempfile::tempdir().unwrap();
    let target = folder.path().join("workspace.json");
    write_atomic(&target, |file| file.write_all(b"previous")).unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_crash_during_a_write_leaves_the_previous_file_intact",
            "--nocapture",
        ])
        .env(CRASH_TARGET, &target)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(6), "SIGABRT: {status:?}");
    assert_eq!(fs::read(&target).unwrap(), b"previous");
    // The next write replaces both the leftover temporary file and the old one.
    write_atomic(&target, |file| file.write_all(b"next")).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"next");
    assert_eq!(mode(&target), 0o600);
}
