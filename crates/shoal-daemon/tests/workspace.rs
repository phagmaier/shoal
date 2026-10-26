//! T12 contracts exercised through the real socket, actor and shell processes.

#[path = "workspace/lifecycle.rs"]
mod lifecycle;
#[path = "workspace/live_settings.rs"]
mod live_settings;
mod support;

use std::{
    fs,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use shoal_testkit::{Signal, eventually};
use support::{Rpc, SHOALD, fixture};

fn create(rpc: &mut Rpc) -> (Value, Value, Value) {
    let session = rpc.ok("session.create", json!({}))["session"].clone();
    let workspace = rpc.workspace();
    let item = workspace["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == session)
        .unwrap();
    let tab = item["tabs"][0]["id"].clone();
    let pane = item["tabs"][0]["focused"].clone();
    (session, tab, pane)
}

#[test]
fn session_tab_and_pane_methods_update_the_shared_workspace() {
    let env = fixture(false);
    let directory = env.home().join("project");
    fs::create_dir(&directory).unwrap();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    assert_eq!(rpc.workspace()["sessions"], json!([]));
    let session = rpc.ok("session.create", json!({"cwd": directory}))["session"].clone();
    let first = rpc.workspace();
    assert_eq!(first["sessions"][0]["name"], "main");
    let original_tab = first["sessions"][0]["active_tab"].clone();
    let original_pane = first["sessions"][0]["tabs"][0]["focused"].clone();
    assert_eq!(
        first["panes"][original_pane.as_str().unwrap()]["cwd"],
        json!(directory)
    );
    let second = rpc.ok("session.create", json!({}))["session"].clone();
    assert_eq!(rpc.workspace()["sessions"][1]["name"], "2");
    rpc.ok("session.rename", json!({"session": second, "name": "work"}));
    assert_eq!(rpc.workspace()["sessions"][1]["name"], "work");

    let created = rpc.ok("tab.create", json!({"session": session, "index": 0}));
    let tab = created["tab"].clone();
    let pane = created["pane"].clone();
    let state = rpc.workspace();
    assert_eq!(state["sessions"][0]["tabs"][0]["id"], tab);
    assert_eq!(
        state["panes"][pane.as_str().unwrap()]["cwd"],
        json!(directory)
    );
    rpc.ok("tab.rename", json!({"tab": tab, "name": "build"}));
    assert_eq!(rpc.workspace()["sessions"][0]["tabs"][0]["name"], "build");
    rpc.ok("tab.rename", json!({"tab": tab, "name": null}));
    assert!(rpc.workspace()["sessions"][0]["tabs"][0]["name"].is_null());
    rpc.ok("tab.move", json!({"tab": tab, "index": 1}));
    assert_eq!(rpc.workspace()["sessions"][0]["tabs"][1]["id"], tab);
    rpc.ok("tab.activate", json!({"tab": original_tab}));
    assert_eq!(rpc.workspace()["sessions"][0]["active_tab"], original_tab);
    rpc.ok("pane.focus", json!({"pane": pane}));
    assert_eq!(rpc.workspace()["sessions"][0]["active_tab"], tab);

    let split_pane =
        rpc.ok("pane.split", json!({"pane": pane, "direction": "right"}))["pane"].clone();
    let state = rpc.workspace();
    let split_id = state["sessions"][0]["tabs"][1]["layout"]["split"]["id"].clone();
    assert_eq!(state["sessions"][0]["tabs"][1]["focused"], split_pane);
    rpc.ok("split.set_ratio", json!({"split": split_id, "ratio": 0.5}));
    rpc.ok(
        "pane.resize",
        json!({"pane": pane, "direction": "right", "cells": 16}),
    );
    let state = rpc.workspace();
    let ratio = state["sessions"][0]["tabs"][1]["layout"]["split"]["ratio"]
        .as_f64()
        .unwrap();
    assert!((ratio - 0.6).abs() < 0.00001, "ratio {ratio}");
    rpc.ok("split.set_ratio", json!({"split": split_id, "ratio": 5.0}));
    let ratio = rpc.workspace()["sessions"][0]["tabs"][1]["layout"]["split"]["ratio"]
        .as_f64()
        .unwrap();
    assert!((ratio - 0.9).abs() < 0.00001);
    let lower = rpc.ok(
        "pane.split",
        json!({"pane": split_pane, "direction": "down"}),
    )["pane"]
        .clone();
    rpc.ok("pane.zoom", json!({"pane": lower, "zoomed": true}));
    rpc.ok("pane.zoom", json!({"pane": lower, "zoomed": true}));
    assert_eq!(rpc.workspace()["sessions"][0]["tabs"][1]["zoomed"], lower);
    rpc.ok("pane.focus", json!({"pane": pane}));
    let focused = rpc.workspace();
    assert_eq!(focused["sessions"][0]["tabs"][1]["focused"], pane);
    assert_eq!(focused["sessions"][0]["tabs"][1]["zoomed"], pane);
    rpc.ok("pane.focus", json!({"pane": lower}));
    assert_eq!(rpc.workspace()["sessions"][0]["tabs"][1]["zoomed"], lower);
    rpc.ok("pane.zoom", json!({"pane": lower, "zoomed": false}));
    assert!(rpc.workspace()["sessions"][0]["tabs"][1]["zoomed"].is_null());

    rpc.ok(
        "pane.send_text",
        json!({"pane": lower, "text": "printf 'answer-%s\\n' \"$((6 * 7))\"\n"}),
    );
    eventually("shell command output", || {
        let data = rpc.ok(
            "pane.capture",
            json!({"pane": lower, "format": "text", "scrollback": true}),
        );
        data["data"]
            .as_str()
            .unwrap()
            .contains("answer-42")
            .then_some(())
    });
    let vt = rpc.ok(
        "pane.capture",
        json!({"pane": lower, "format": "vt", "scrollback": false}),
    );
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let bytes = STANDARD.decode(vt["data"].as_str().unwrap()).unwrap();
    let mut terminal = shoal_term::Terminal::new(80, 24, 100).unwrap();
    terminal.feed(&bytes);
    assert!(
        String::from_utf8(
            terminal
                .capture(shoal_term::CaptureFormat::Text, false)
                .unwrap()
        )
        .unwrap()
        .contains("answer-42")
    );

    rpc.ok("pane.close", json!({"pane": lower}));
    rpc.ok("pane.close", json!({"pane": split_pane}));
    assert_eq!(
        rpc.workspace()["sessions"][0]["tabs"][1]["layout"],
        json!({"pane": pane})
    );
    rpc.ok("tab.close", json!({"tab": tab}));
    assert_eq!(rpc.workspace()["sessions"][0]["active_tab"], original_tab);
    rpc.ok("pane.close", json!({"pane": original_pane}));
    assert_eq!(rpc.workspace()["sessions"].as_array().unwrap().len(), 1);
    rpc.ok("session.close", json!({"session": second}));
    assert_eq!(rpc.workspace()["panes"], json!({}));
    let status = rpc.ok("daemon.status", json!({}));
    assert_eq!(status["sessions"], 0);
    assert_eq!(status["panes"], 0);
    assert_eq!(status["pid"], _daemon.pid());
    assert_eq!(status["version"], "0.1.0");
    assert!(status["uptime_s"].is_u64());
    // The layout was saved after the first change (§6.3).
    assert!(status["last_persist_at"].is_string(), "{status}");
}

#[test]
fn missing_targets_and_rejected_edits_leave_existing_structure_intact() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let (session, tab, pane) = create(&mut rpc);
    for (method, params) in [
        ("session.rename", json!({"session": "s99999", "name": "x"})),
        ("session.close", json!({"session": "s99999"})),
        ("window.show_session", json!({"session": "s99999"})),
        ("tab.create", json!({"session": "s99999"})),
        ("tab.close", json!({"tab": "t99999"})),
        ("tab.rename", json!({"tab": "t99999", "name": "x"})),
        ("tab.move", json!({"tab": "t99999", "index": 0})),
        ("tab.activate", json!({"tab": "t99999"})),
        (
            "pane.split",
            json!({"pane": "p99999", "direction": "right"}),
        ),
        ("pane.close", json!({"pane": "p99999"})),
        ("pane.focus", json!({"pane": "p99999"})),
        (
            "pane.resize",
            json!({"pane": "p99999", "direction": "left", "cells": 2}),
        ),
        ("split.set_ratio", json!({"split": "x99999", "ratio": 0.5})),
        ("pane.zoom", json!({"pane": "p99999", "zoomed": true})),
        ("pane.send_text", json!({"pane": "p99999", "text": "x"})),
        (
            "pane.capture",
            json!({"pane": "p99999", "format": "text", "scrollback": false}),
        ),
    ] {
        rpc.error(method, params, 1001);
    }
    rpc.error("session.create", json!({"name": "main"}), 1002);
    let other = rpc.ok("session.create", json!({"name": "other"}))["session"].clone();
    rpc.error(
        "session.rename",
        json!({"session": other, "name": "main"}),
        1002,
    );
    rpc.error("tab.move", json!({"tab": tab, "index": 2}), 1002);
    rpc.error("tab.create", json!({"session": session, "index": 2}), 1002);
    rpc.error(
        "tab.create",
        json!({"session": session, "argv": ["/missing/shoal-program"]}),
        1005,
    );
    for (method, params) in [
        (
            "pane.resize",
            json!({"pane": pane, "direction": "right", "cells": -1}),
        ),
        (
            "split.set_ratio",
            json!({"split": "x99999", "ratio": 1e300}),
        ),
        (
            "pane.capture",
            json!({"pane": pane, "format": "html", "scrollback": false}),
        ),
    ] {
        rpc.error(method, params, 1004);
    }
    rpc.ok(
        "pane.resize",
        json!({"pane": pane, "direction": "right", "cells": 0}),
    );
    rpc.ok(
        "pane.resize",
        json!({"pane": pane, "direction": "left", "cells": 10}),
    );
    let state = rpc.workspace();
    assert_eq!(state["sessions"][0]["tabs"].as_array().unwrap().len(), 1);
    assert_eq!(
        state["sessions"][0]["tabs"][0]["layout"],
        json!({"pane": pane})
    );
    assert_eq!(state["sessions"][1]["name"], "other");
    assert_eq!(state["panes"].as_object().unwrap().len(), 2);
    assert_eq!(rpc.ok("daemon.persist", json!({})), json!({}));
}

#[test]
fn simultaneous_smart_opens_reserve_distinct_sessions_and_disconnect_releases_windows() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut first = Rpc::hello(&env, "gui", json!({}));
    let mut second = Rpc::hello(&env, "gui", json!({}));
    let a = first.send("session.open_smart", json!({}));
    let b = second.send("session.open_smart", json!({}));
    let a = first.response(a).unwrap()["session"].clone();
    let b = second.response(b).unwrap()["session"].clone();
    assert_ne!(a, b);
    assert_eq!(first.ok("session.open_smart", json!({}))["session"], a);
    first.ok("window.show_session", json!({"session": a}));
    second.ok("window.show_session", json!({"session": b}));
    second.ok("window.show_session", json!({"session": b}));
    second.ok("window.set_focused", json!({"focused": true}));
    second.ok("window.set_focused", json!({"focused": false}));
    let state = second.workspace();
    assert_eq!(state["sessions"][0]["windows"], 1);
    assert_eq!(state["sessions"][1]["windows"], 1);
    drop(first);
    eventually("disconnected window count", || {
        let workspace = second.workspace();
        let session = workspace["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|session| session["id"] == a)
            .unwrap();
        (session["windows"] == 0).then_some(())
    });
    let mut third = Rpc::hello(&env, "gui", json!({}));
    assert_eq!(third.ok("session.open_smart", json!({}))["session"], a);
    third.ok("window.show_session", json!({"session": b}));
    let workspace = third.workspace();
    let session = workspace["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| session["id"] == b)
        .unwrap();
    assert_eq!(session["windows"], 2);
    third.ok("session.close", json!({"session": b}));
    assert_eq!(third.workspace()["sessions"][0]["id"], a);
    assert_eq!(third.workspace()["sessions"][0]["windows"], 0);
}

#[test]
fn subscriptions_send_initial_and_coalesced_latest_state_in_order() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut observer = Rpc::new(&env);
    observer.ok("workspace.subscribe", json!({}));
    let initial = observer.notification("workspace.changed");
    assert_eq!(initial["workspace"]["sessions"], json!([]));
    let mut editor = Rpc::new(&env);
    let began = Instant::now();
    let (session, _, _) = create(&mut editor);
    let mut last = 0;
    for index in 0..24 {
        last = editor.send(
            "session.rename",
            json!({"session": session, "name": format!("name-{index}")}),
        );
    }
    for id in (last - 23)..=last {
        editor.response(id).unwrap();
    }
    let latest = editor.workspace();
    let editing_ms = began.elapsed().as_millis();
    let mut revisions = vec![initial["rev"].as_u64().unwrap()];
    loop {
        let update = observer.notification("workspace.changed");
        revisions.push(update["rev"].as_u64().unwrap());
        if update["rev"] == latest["rev"] {
            assert_eq!(update["workspace"], latest);
            break;
        }
    }
    assert!(revisions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(
        revisions.len() < 26,
        "updates were not coalesced: {revisions:?}"
    );
    assert!(
        revisions.len() as u128 <= 3 + editing_ms / 16,
        "more than one update per 16 ms: {revisions:?} in {editing_ms} ms"
    );
}

#[test]
fn smart_open_chooses_the_most_recent_hidden_session_and_the_lowest_free_name() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut editor = Rpc::new(&env);
    let (main, _, _) = create(&mut editor);
    let (two, _, _) = create(&mut editor);
    let (three, _, _) = create(&mut editor);
    let mut window = Rpc::hello(&env, "gui", json!({}));
    window.ok("window.show_session", json!({"session": main}));
    window.ok("window.show_session", json!({"session": three}));
    let mut next = Rpc::hello(&env, "gui", json!({}));
    assert_eq!(next.ok("session.open_smart", json!({}))["session"], main);
    next.ok("window.show_session", json!({"session": main}));
    let mut last = Rpc::hello(&env, "gui", json!({}));
    assert_eq!(last.ok("session.open_smart", json!({}))["session"], two);
    last.ok("window.show_session", json!({"session": two}));
    editor.ok("session.close", json!({"session": two}));
    let (_, _, _) = create(&mut editor);
    assert_eq!(editor.workspace()["sessions"][2]["name"], "2");
}

#[test]
fn exit_policy_retains_early_errors_but_closes_successful_last_panes() {
    let env = fixture(true);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let (session, original, _) = create(&mut rpc);
    let result = rpc.ok(
        "tab.create",
        json!({"session": session, "argv": ["/bin/sh", "-c", "printf early-error; exit 7"]}),
    );
    let pane = result["pane"].clone();
    eventually("early exit status", || {
        let state = rpc.workspace();
        (state["panes"][pane.as_str().unwrap()]["status"]
            == json!({"state": "exited", "code": 7, "signal": null}))
        .then_some(())
    });
    assert!(
        rpc.ok(
            "pane.capture",
            json!({"pane": pane, "format": "text", "scrollback": false})
        )["data"]
            .as_str()
            .unwrap()
            .contains("early-error")
    );
    rpc.ok("pane.send_text", json!({"pane": pane, "text": "ignored\n"}));
    rpc.ok("tab.close", json!({"tab": original}));
    rpc.ok("pane.close", json!({"pane": pane}));
    let (_, _, pane) = create(&mut rpc);
    rpc.ok("pane.send_text", json!({"pane": pane, "text": "exit 0\n"}));
    eventually("exit closes pane, tab and session", || {
        (rpc.workspace()["sessions"] == json!([])).then_some(())
    });
}

#[test]
fn shutdown_requests_and_signals_end_shells_and_remove_the_socket() {
    for signal in [None, Some(Signal::TERM), Some(Signal::INT)] {
        let env = fixture(false);
        let daemon = env.spawn_daemon(SHOALD);
        let mut rpc = Rpc::new(&env);
        let (_, _, pane) = create(&mut rpc);
        let pid_file = env.home().join("shell.pid");
        rpc.ok(
            "pane.send_text",
            json!({"pane": pane, "text": format!("printf '%s' $$ > '{}'\n", pid_file.display())}),
        );
        let pid = eventually("shell pid file", || {
            fs::read_to_string(&pid_file)
                .ok()
                .filter(|value| !value.is_empty())
        });
        let process = format!("/proc/{}", pid.trim());
        match signal {
            Some(signal) => daemon.signal(signal),
            None => {
                rpc.ok("daemon.shutdown", json!({}));
            }
        }
        assert!(daemon.wait().status.success());
        assert!(!env.socket().exists());
        // Saved first; tests/persist.rs checks what.
        assert!(env.state_home().join("shoal/workspace.json").exists());
        eventually("shell process gone", || {
            (!std::path::Path::new(&process).exists()).then_some(())
        });
    }
}

#[test]
fn shutdown_escalates_all_fifty_panes_concurrently_within_three_seconds() {
    let env = fixture(false);
    let daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let (session, _, original) = create(&mut rpc);
    for _ in 0..49 {
        let pane = rpc.ok("tab.create", json!({"session": session, "argv": ["/bin/sh", "-c", "trap '' HUP; printf ready-to-stop; while :; do sleep 60; done"]}))["pane"].clone();
        eventually("hangup trap installed", || {
            rpc.ok(
                "pane.capture",
                json!({"pane": pane, "format": "text", "scrollback": false}),
            )["data"]
                .as_str()
                .unwrap()
                .contains("ready-to-stop")
                .then_some(())
        });
    }
    rpc.ok(
        "pane.send_text",
        json!({"pane": original, "text": "trap '' HUP; printf 'ready-%s\\n' to-stop\n"}),
    );
    eventually("original shell hangup trap", || {
        rpc.ok(
            "pane.capture",
            json!({"pane": original, "format": "text", "scrollback": false}),
        )["data"]
            .as_str()
            .unwrap()
            .contains("ready-to-stop")
            .then_some(())
    });
    let started = Instant::now();
    rpc.ok("daemon.shutdown", json!({}));
    assert!(daemon.wait().status.success());
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "shutdown took {:?}",
        started.elapsed()
    );
}
