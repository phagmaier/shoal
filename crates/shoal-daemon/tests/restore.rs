//! T16: restore on daemon start (M1 spec §6.4) and the "Sessions survive"
//! checks in §13, through the real daemon. `daemon.shutdown` stands in for
//! `shoal daemon stop` and `workspace.get` for `shoal ls` until the CLI exists.

#[allow(dead_code)]
mod support;

use std::{fs, path::Path};

use serde_json::{Value, json};
use shoal_testkit::{Daemon, Signal, TempEnv, eventually};
use support::{
    Rpc, SHOALD, config, fixture, print, saved_text, saved_workspace, screen, scrollback, state,
    wait_for,
};

const DIVIDER: &str = "── restored · ";

fn stop(daemon: Daemon, rpc: &mut Rpc) {
    rpc.ok("daemon.shutdown", json!({}));
    assert!(daemon.wait().status.success());
}

fn folder(env: &TempEnv, name: &str) -> String {
    let path = env.home().join(name);
    fs::create_dir_all(&path).unwrap();
    path.to_str().unwrap().to_owned()
}

/// Create a session in `cwd` and return its first pane once its prompt is up.
fn session_in(rpc: &mut Rpc, cwd: &str) -> Value {
    let session = rpc.ok("session.create", json!({"cwd": cwd}))["session"].clone();
    let pane = rpc.workspace()["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == session)
        .unwrap()["tabs"][0]["focused"]
        .clone();
    wait_for(rpc, &pane, "fixture>");
    pane
}

fn split_in(rpc: &mut Rpc, pane: &Value, direction: &str, cwd: &str) -> Value {
    let pane = rpc.ok(
        "pane.split",
        json!({"pane": pane, "direction": direction, "cwd": cwd}),
    )["pane"]
        .clone();
    wait_for(rpc, &pane, "fixture>");
    pane
}

/// The screen after the restored divider, which must come after `marker`.
fn after_divider(rpc: &mut Rpc, pane: &Value, marker: Option<&str>) -> String {
    wait_for(rpc, pane, DIVIDER);
    let text = screen(rpc, pane);
    let divider = text.find(DIVIDER).unwrap();
    if let Some(marker) = marker {
        let at = text
            .find(&format!("{marker}\n"))
            .unwrap_or_else(|| panic!("{marker}: {text}"));
        assert!(at < divider, "{marker} above the divider: {text}");
    }
    text[divider..].to_owned()
}

/// Run `pwd` in a restored pane's new shell and return the folder it prints.
fn pwd(rpc: &mut Rpc, pane: &Value) -> String {
    eventually("the new shell's prompt", || {
        after_divider(rpc, pane, None)
            .contains("fixture>")
            .then_some(())
    });
    rpc.ok("pane.send_text", json!({"pane": pane, "text": "pwd\n"}));
    eventually("pwd output", || {
        let after = after_divider(rpc, pane, None);
        let lines: Vec<_> = after.lines().collect();
        let command = lines.iter().rposition(|line| line.ends_with("> pwd"))?;
        let folder = lines.get(command + 1)?;
        folder.starts_with('/').then(|| (*folder).to_owned())
    })
}

/// The workspace without what a restart may change.
fn layout(workspace: &Value) -> (Value, Vec<(String, Value, Value)>) {
    let panes = workspace["panes"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, pane)| (id.clone(), pane["cwd"].clone(), pane["size"].clone()))
        .collect();
    (workspace["sessions"].clone(), panes)
}

fn max_id(workspace: &Value) -> u64 {
    let text = workspace["sessions"].to_string();
    let mut max = 0;
    for kind in ["s", "t", "p", "x"] {
        for piece in text.split(&format!("\"{kind}")).skip(1) {
            let digits: String = piece.chars().take_while(char::is_ascii_digit).collect();
            if piece[digits.len()..].starts_with('"') && !digits.is_empty() {
                max = max.max(digits.parse().unwrap());
            }
        }
    }
    max
}

#[test]
fn a_restart_restores_layouts_old_output_and_folders() {
    let env = fixture(false);
    let daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    // Three sessions with splits, every pane in its own folder.
    let mut panes = Vec::new();
    let one = session_in(&mut rpc, &folder(&env, "one"));
    panes.push((one.clone(), "one"));
    panes.push((
        split_in(&mut rpc, &one, "right", &folder(&env, "one/right")),
        "one/right",
    ));
    let two = session_in(&mut rpc, &folder(&env, "two"));
    panes.push((two.clone(), "two"));
    let down = split_in(&mut rpc, &two, "down", &folder(&env, "two/down"));
    panes.push((down.clone(), "two/down"));
    panes.push((
        split_in(&mut rpc, &down, "right", &folder(&env, "two/down/right")),
        "two/down/right",
    ));
    rpc.ok("pane.zoom", json!({"pane": down, "zoomed": true}));
    let three = session_in(&mut rpc, &folder(&env, "three"));
    panes.push((three, "three"));
    let session = rpc.workspace()["sessions"][2]["id"].clone();
    let tab = rpc.ok(
        "tab.create",
        json!({"session": session, "cwd": folder(&env, "three/tab")}),
    );
    wait_for(&mut rpc, &tab["pane"], "fixture>");
    panes.push((tab["pane"].clone(), "three/tab"));
    rpc.ok("tab.rename", json!({"tab": tab["tab"], "name": "second"}));
    for (pane, name) in &panes {
        print(
            &mut rpc,
            pane,
            &format!("marker-{}", name.replace('/', "-")),
        );
    }
    let before = rpc.workspace();
    stop(daemon, &mut rpc);
    let saved_at = saved_workspace(&env)["saved_at"].clone();

    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let after = rpc.workspace();
    assert_eq!(after["rev"], 1);
    assert_eq!(layout(&after), layout(&before));
    for (pane, name) in &panes {
        assert_eq!(
            after["panes"][pane.as_str().unwrap()]["restored_from"],
            saved_at
        );
        let marker = format!("marker-{}", name.replace('/', "-"));
        after_divider(&mut rpc, pane, Some(&marker));
        assert_eq!(
            pwd(&mut rpc, pane),
            env.home().join(name).to_str().unwrap(),
            "{name}"
        );
    }

    // New ids continue above every restored one.
    let highest = max_id(&after);
    assert!(after["next_id"].as_u64().unwrap() > highest);
    let session = rpc.ok("session.create", json!({}))["session"].clone();
    let workspace = rpc.workspace();
    let created = workspace["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == session)
        .unwrap();
    for id in [
        &created["id"],
        &created["tabs"][0]["id"],
        &created["tabs"][0]["focused"],
    ] {
        let number: u64 = id.as_str().unwrap()[1..].parse().unwrap();
        assert!(number > highest, "{id} after {highest}");
    }
}

#[test]
fn after_kill_9_the_last_scheduled_save_is_restored() {
    let env = fixture(false);
    let daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let pane = session_in(&mut rpc, env.home().to_str().unwrap());
    print(&mut rpc, &pane, "marker-saved");
    // The quiet-period save, then the layout that names it.
    let file = scrollback(&env, &pane);
    eventually("the scheduled scrollback save", || {
        (file.exists() && saved_text(&file).contains("marker-saved")).then_some(())
    });
    eventually("the layout to reference it", || {
        saved_workspace(&env)["panes"][pane.as_str().unwrap()]["scrollback"]
            .is_string()
            .then_some(())
    });
    daemon.signal(Signal::KILL);
    assert!(!daemon.wait().status.success());

    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    after_divider(&mut rpc, &pane, Some("marker-saved"));
    // The layout written at startup still names the old file, so a second
    // crash before the pane's next save loses nothing either.
    eventually("the startup layout write", || {
        let status = rpc.ok("daemon.status", json!({}));
        status["last_persist_at"].is_string().then_some(())
    });
    let saved = saved_workspace(&env);
    assert!(
        saved["panes"][pane.as_str().unwrap()]["scrollback"].is_string(),
        "{saved}"
    );
    assert!(saved_text(&file).contains("marker-saved"));
}

#[test]
fn a_deleted_folder_restores_into_home_and_says_so() {
    let env = fixture(false);
    let daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let gone = folder(&env, "gone");
    let pane = session_in(&mut rpc, &gone);
    print(&mut rpc, &pane, "marker-gone");
    stop(daemon, &mut rpc);
    fs::remove_dir(&gone).unwrap();

    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let after = after_divider(&mut rpc, &pane, Some("marker-gone"));
    let divider = after.lines().next().unwrap();
    assert!(divider.contains("(folder gone)"), "{divider}");
    let home = env.home().to_str().unwrap().to_owned();
    assert_eq!(pwd(&mut rpc, &pane), home);
    assert_eq!(
        rpc.workspace()["panes"][pane.as_str().unwrap()]["cwd"],
        home
    );
}

#[test]
fn a_corrupt_workspace_is_moved_aside_and_the_first_window_is_told() {
    let env = fixture(false);
    fs::create_dir_all(state(&env)).unwrap();
    fs::write(state(&env).join("workspace.json"), "{\"format\": 1, \"sess").unwrap();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut window = Rpc::hello(&env, "gui", json!({}));
    let workspace = window.workspace();
    assert_eq!(workspace["sessions"], json!([]));
    let broken: Vec<_> = fs::read_dir(state(&env))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("workspace.json.broken-"))
        .collect();
    let [name] = &broken[..] else {
        panic!("{broken:?}")
    };
    assert_eq!(
        fs::read_to_string(state(&env).join(name)).unwrap(),
        "{\"format\": 1, \"sess"
    );
    let notice = |rpc: &Rpc| {
        rpc.notices
            .iter()
            .any(|notice| notice["message"].as_str().unwrap().contains(name.as_str()))
    };
    assert!(notice(&window), "{:?}", window.notices);
    assert_eq!(
        window
            .notices
            .iter()
            .find(|n| n["message"].as_str().unwrap().contains(name.as_str()))
            .unwrap()["level"],
        "warn"
    );
    // The first window only.
    let mut later = Rpc::hello(&env, "gui", json!({}));
    later.workspace();
    assert!(!notice(&later), "{:?}", later.notices);
}

#[test]
fn an_unreadable_scrollback_file_is_skipped_with_a_note() {
    let env = fixture(false);
    let daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let pane = session_in(&mut rpc, env.home().to_str().unwrap());
    print(&mut rpc, &pane, "marker-corrupt");
    stop(daemon, &mut rpc);
    fs::write(scrollback(&env, &pane), "not zstd at all").unwrap();

    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let after = after_divider(&mut rpc, &pane, None);
    assert!(
        after
            .lines()
            .next()
            .unwrap()
            .contains("(old output unreadable)"),
        "{after}"
    );
    assert!(!screen(&mut rpc, &pane).contains("marker-corrupt\n"));
}

#[test]
fn with_saving_off_old_output_stays_hidden_and_is_deleted() {
    let env = fixture(false);
    let daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let pane = session_in(&mut rpc, env.home().to_str().unwrap());
    print(&mut rpc, &pane, "marker-private");
    stop(daemon, &mut rpc);
    let file = scrollback(&env, &pane);
    assert!(saved_text(&file).contains("marker-private"));
    fs::write(
        env.config_home().join("shoal/config.toml"),
        format!("{}[scrollback]\npersist = false\n", config(false)),
    )
    .unwrap();

    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let after = after_divider(&mut rpc, &pane, None);
    assert!(!after.contains("unreadable"), "{after}");
    assert!(!screen(&mut rpc, &pane).contains("marker-private\n"));
    eventually("the old file to be deleted", || {
        (!file.exists()).then_some(())
    });
    assert!(Path::new(&state(&env).join("workspace.json")).exists());
}

#[test]
fn panes_whose_shell_cannot_start_are_dropped_with_a_notice() {
    let env = fixture(false);
    let daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let pane = session_in(&mut rpc, env.home().to_str().unwrap());
    split_in(&mut rpc, &pane, "right", env.home().to_str().unwrap());
    stop(daemon, &mut rpc);
    fs::write(
        env.config_home().join("shoal/config.toml"),
        config(false).replace("/bin/bash", "/nonexistent/shell"),
    )
    .unwrap();

    let _daemon = env.spawn_daemon(SHOALD);
    let mut window = Rpc::hello(&env, "gui", json!({}));
    let workspace = window.workspace();
    assert_eq!(workspace["sessions"], json!([]), "{workspace}");
    assert_eq!(workspace["panes"], json!({}));
    assert!(
        window
            .notices
            .iter()
            .any(|notice| notice["message"] == "2 saved panes couldn't be restored."),
        "{:?}",
        window.notices
    );
}

/// M1 spec §12: 20 panes × 10,000 lines restored in under 500 ms. Timing
/// depends on the build and machine, so run it deliberately:
/// `cargo test --release -p shoal-daemon --test restore -- --ignored`.
#[test]
#[ignore = "performance budget; run in release"]
fn restoring_twenty_panes_of_ten_thousand_lines_meets_the_budget() {
    let env = fixture(false);
    fs::write(
        env.config_home().join("shoal/config.toml"),
        format!("{}[scrollback]\nlines = 10000\n", config(false)),
    )
    .unwrap();
    let lines: String = (0..10_000)
        .map(|i| {
            format!(
                "\x1b[3{}mline {i:05} of saved output, long enough to look real\x1b[0m\r\n",
                i % 8
            )
        })
        .collect();
    let compressed = zstd::bulk::compress(lines.as_bytes(), 3).unwrap();
    let ids: Vec<u64> = (0..20).map(|i| 3 + 2 * i).collect();
    let tabs: Vec<Value> = ids
        .iter()
        .map(|id| json!({"id": format!("t{}", id - 1), "name": null, "focused": format!("p{id}"), "zoomed": null, "layout": {"pane": format!("p{id}")}}))
        .collect();
    let panes: serde_json::Map<String, Value> = ids
        .iter()
        .map(|id| {
            let pane = json!({"kind": {"kind": "shell", "argv": []}, "cwd": env.home(), "size": {"cols": 120, "rows": 40}, "program": null, "scrollback": format!("panes/p{id}/scrollback.vt.zst")});
            (format!("p{id}"), pane)
        })
        .collect();
    let document = json!({
        "format": 1, "saved_at": "2026-10-24T14:02:11Z", "shoal_version": "0.1.0", "next_id": 100,
        "sessions": [{"id": "s1", "name": "main", "created_at": "2026-10-20T09:00:00Z",
            "last_active_at": "2026-10-24T14:01:58Z", "active_tab": "t2", "tabs": tabs}],
        "panes": panes,
    });
    for id in &ids {
        let folder = state(&env).join(format!("panes/p{id}"));
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("scrollback.vt.zst"), &compressed).unwrap();
    }
    fs::write(state(&env).join("workspace.json"), document.to_string()).unwrap();
    let started = std::time::Instant::now();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    assert_eq!(rpc.workspace()["panes"].as_object().unwrap().len(), 20);
    let elapsed = started.elapsed();
    eprintln!("restored 20 panes x 10,000 lines in {elapsed:?}");
    let pane = json!(format!("p{}", ids[19]));
    after_divider(
        &mut rpc,
        &pane,
        Some("line 09999 of saved output, long enough to look real"),
    );
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "{elapsed:?}"
    );
}
