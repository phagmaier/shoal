//! T14 metadata through a real daemon, socket and interactive shells.

#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use shoal_testkit::eventually;
use support::{Rpc, SHOALD, fixture};

fn create(rpc: &mut Rpc) -> (Value, Value, Value) {
    let session = rpc.ok("session.create", json!({}))["session"].clone();
    let workspace = rpc.workspace();
    let tab = &workspace["sessions"][0]["tabs"][0];
    (session, tab["id"].clone(), tab["focused"].clone())
}

fn send(rpc: &mut Rpc, pane: &Value, text: &str) {
    rpc.ok("pane.send_text", json!({"pane": pane, "text": text}));
}

fn wait_pane(rpc: &mut Rpc, pane: &Value, check: impl Fn(&Value) -> bool) -> Value {
    eventually("pane metadata", || {
        let record = rpc.workspace()["panes"][pane.as_str().unwrap()].clone();
        check(&record).then_some(record)
    })
}

#[test]
fn proc_directory_and_foreground_program_follow_cd_and_sleep() {
    let env = fixture(false);
    std::fs::create_dir(env.home().join("project")).unwrap();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let (_, _, pane) = create(&mut rpc);
    wait_pane(&mut rpc, &pane, |p| {
        p["title"] == "~" && p["program"].is_null()
    });
    send(&mut rpc, &pane, "cd project\r");
    wait_pane(&mut rpc, &pane, |p| {
        p["cwd"] == json!(env.home().join("project")) && p["title"] == "project"
    });
    send(&mut rpc, &pane, "sleep 2\r");
    wait_pane(&mut rpc, &pane, |p| {
        p["program"] == "sleep" && p["title"] == "sleep"
    });
    wait_pane(&mut rpc, &pane, |p| {
        p["program"].is_null() && p["title"] == "project"
    });
}

#[test]
fn injected_shells_report_encoded_directories_and_titles() {
    for shell in ["/bin/bash", "/bin/zsh"] {
        let env = fixture(false);
        std::fs::write(
            env.config_home().join("shoal/config.toml"),
            format!("[shell]\nprogram = '{shell}'\nintegration = 'auto'\n[prompt]\nmode = 'shell'\n[behavior]\nclose_pane_on_exit = false\n"),
        ).unwrap();
        std::fs::write(env.home().join(".zshrc"), "PROMPT='fixture> '\n").unwrap();
        let name = "space % café\t";
        let path = env.home().join(name);
        std::fs::create_dir(&path).unwrap();
        let _daemon = env.spawn_daemon(SHOALD);
        let mut rpc = Rpc::new(&env);
        let (_, _, pane) = create(&mut rpc);
        wait_pane(&mut rpc, &pane, |p| p["title"] == "~");
        send(&mut rpc, &pane, "cd $'space % café\\t'\r");
        wait_pane(&mut rpc, &pane, |p| {
            p["cwd"] == json!(path) && p["title"] == name
        });
        send(&mut rpc, &pane, "cd\r");
        wait_pane(&mut rpc, &pane, |p| {
            p["cwd"] == json!(env.home()) && p["title"] == "~"
        });
    }
}

#[test]
fn osc_titles_obey_live_policy_user_names_and_directory_precedence() {
    let env = fixture(false);
    let file = env.config_home().join("shoal/config.toml");
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let (_, tab, pane) = create(&mut rpc);
    // The reported directory deliberately differs from the real cwd. Waiting
    // for it also proves the OSC title in this same read has been processed.
    send(
        &mut rpc,
        &pane,
        "printf '\\033]0;editor\\007\\033]7;file://localhost/reported/project\\007'\r",
    );
    wait_pane(&mut rpc, &pane, |p| {
        p["cwd"] == "/reported/project" && p["title"] == "project"
    });
    rpc.ok("tab.rename", json!({"tab": tab, "name": "mine"}));
    assert_eq!(
        rpc.workspace()["panes"][pane.as_str().unwrap()]["title"],
        "mine"
    );
    std::fs::write(
        &file,
        format!("{}[tabs]\ntitle = 'program'\n", support::config(false)),
    )
    .unwrap();
    wait_pane(&mut rpc, &pane, |p| p["title"] == "editor");
    send(
        &mut rpc,
        &pane,
        "printf '\\033]7;file://localhost/invalid%%FF\\007\\033]2;compiler\\033\\\\'\r",
    );
    wait_pane(&mut rpc, &pane, |p| {
        p["title"] == "compiler" && p["cwd"] == "/reported/project"
    });
    std::fs::write(&file, support::config(false)).unwrap();
    wait_pane(&mut rpc, &pane, |p| p["title"] == "mine");
    rpc.ok("tab.rename", json!({"tab": tab, "name": null}));
    wait_pane(&mut rpc, &pane, |p| p["title"] == "project");
    std::fs::write(
        &file,
        format!("{}[tabs]\ntitle = 'program'\n", support::config(false)),
    )
    .unwrap();
    wait_pane(&mut rpc, &pane, |p| p["title"] == "compiler");
    send(&mut rpc, &pane, "printf '\\033]2;\\007'\r");
    wait_pane(&mut rpc, &pane, |p| p["title"] == "project");
    // A later /proc sample must retain the latest valid OSC 7 report.
    send(&mut rpc, &pane, "sleep 2\r");
    wait_pane(&mut rpc, &pane, |p| {
        p["program"] == "sleep" && p["cwd"] == "/reported/project"
    });
    wait_pane(&mut rpc, &pane, |p| {
        p["program"].is_null() && p["title"] == "project"
    });
}

#[test]
fn silent_input_samples_after_enter_on_both_input_paths() {
    let env = fixture(false);
    std::fs::write(
        env.home().join(".bashrc"),
        "stty -echo; PS1=''; unset PROMPT_COMMAND\n",
    )
    .unwrap();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let (_, _, pane) = create(&mut rpc);
    // A round trip through a built-in verifies rc completion without echo.
    send(&mut rpc, &pane, "printf ready\r");
    eventually("silent shell ready", || {
        rpc.ok(
            "pane.capture",
            json!({"pane": pane, "format": "text", "scrollback": false}),
        )["data"]
            .as_str()
            .unwrap()
            .contains("ready")
            .then_some(())
    });
    wait_pane(&mut rpc, &pane, |p| p["title"] == "~");
    let mut connection = env.connect();
    connection.hello();
    for binary in [false, true] {
        if binary {
            connection.send_raw(
                &shoal_proto::Frame::Input(shoal_proto::Input {
                    pane: serde_json::from_value(pane.clone()).unwrap(),
                    bytes: b"sleep 2; printf done\n",
                })
                .encode()
                .unwrap(),
            );
        } else {
            send(&mut rpc, &pane, "sleep 2; printf done\r");
        }
        wait_pane(&mut rpc, &pane, |p| p["program"] == "sleep");
        wait_pane(&mut rpc, &pane, |p| p["program"].is_null());
    }
}

fn bell(rpc: &mut Rpc, pane: &Value, stage: &str, expected: bool) {
    send(
        rpc,
        pane,
        &format!("printf '\\007\\033]7;file://localhost/bells/{stage}\\007'\r"),
    );
    let value = wait_pane(rpc, pane, |p| p["cwd"] == format!("/bells/{stage}"));
    assert_eq!(value["bell"], expected, "{stage}");
}

fn queue_bell(rpc: &mut Rpc, pane: &Value, stage: &str) {
    send(
        rpc,
        pane,
        &format!("printf '\\033]7;file://localhost/bells/{stage}-prime\\007'\r"),
    );
    wait_pane(rpc, pane, |p| p["cwd"] == format!("/bells/{stage}-prime"));
    send(
        rpc,
        pane,
        &format!(
            "printf '\\007\\033]7;file://localhost/bells/{stage}\\007queued-%s-%s' {stage} 42\r"
        ),
    );
    // The expected text occurs only after printf runs, never in input echo.
    let marker = format!("queued-{stage}-42");
    eventually("queued bell output", || {
        rpc.ok(
            "pane.capture",
            json!({"pane": pane, "format": "text", "scrollback": false}),
        )["data"]
            .as_str()
            .unwrap()
            .contains(&marker)
            .then_some(())
    });
}

#[test]
fn bells_follow_shared_pane_tab_session_and_window_focus() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let (session, tab, pane) = create(&mut rpc);
    bell(&mut rpc, &pane, "headless", true);
    let mut window = Rpc::hello(&env, "gui", json!({}));
    window.ok("window.show_session", json!({"session": session}));
    window.ok("window.set_focused", json!({"focused": true}));
    wait_pane(&mut rpc, &pane, |p| p["bell"] == false);
    bell(&mut rpc, &pane, "focused", false);
    let split = rpc.ok("pane.split", json!({"pane": pane, "direction": "right"}))["pane"].clone();
    bell(&mut rpc, &pane, "unfocused-split", true);
    rpc.ok("pane.focus", json!({"pane": pane}));
    assert_eq!(
        rpc.workspace()["panes"][pane.as_str().unwrap()]["bell"],
        false
    );
    rpc.ok("pane.focus", json!({"pane": split}));
    // A callback can wait for the throttle while focus enters and leaves.
    // Focusing must clear that bell as well as one already in the workspace.
    queue_bell(&mut rpc, &pane, "queued-before-focus");
    rpc.ok("pane.focus", json!({"pane": pane}));
    rpc.ok("pane.focus", json!({"pane": split}));
    let queued = wait_pane(&mut rpc, &pane, |p| {
        p["cwd"] == "/bells/queued-before-focus"
    });
    assert_eq!(queued["bell"], false, "focus must clear a throttled bell");
    rpc.ok("pane.focus", json!({"pane": pane}));
    queue_bell(&mut rpc, &pane, "queued-while-focused");
    rpc.ok("pane.focus", json!({"pane": split}));
    let queued = wait_pane(&mut rpc, &pane, |p| {
        p["cwd"] == "/bells/queued-while-focused"
    });
    assert_eq!(
        queued["bell"], false,
        "leaving focus must not revive a bell"
    );
    rpc.ok("pane.focus", json!({"pane": pane}));
    bell(&mut rpc, &split, "other-split", true);
    rpc.ok("pane.zoom", json!({"pane": split, "zoomed": true}));
    assert_eq!(
        rpc.workspace()["panes"][split.as_str().unwrap()]["bell"],
        false
    );
    let other_tab = rpc.ok("tab.create", json!({"session": session}))["tab"].clone();
    bell(&mut rpc, &split, "inactive-tab", true);
    rpc.ok("tab.activate", json!({"tab": tab}));
    assert_eq!(
        rpc.workspace()["panes"][split.as_str().unwrap()]["bell"],
        false
    );
    let other_session = rpc.ok("session.create", json!({}))["session"].clone();
    window.ok("window.show_session", json!({"session": other_session}));
    bell(&mut rpc, &split, "hidden-session", true);
    window.ok("window.show_session", json!({"session": session}));
    assert_eq!(
        rpc.workspace()["panes"][split.as_str().unwrap()]["bell"],
        false
    );
    queue_bell(&mut rpc, &split, "queued-window-focus");
    window.ok("window.set_focused", json!({"focused": false}));
    let queued = wait_pane(&mut rpc, &split, |p| {
        p["cwd"] == "/bells/queued-window-focus"
    });
    assert_eq!(queued["bell"], false, "window blur must not revive a bell");
    bell(&mut rpc, &split, "unfocused-window", true);
    // Any focused window showing the pane suppresses the bell.
    let mut second = Rpc::hello(&env, "gui", json!({}));
    second.ok("window.show_session", json!({"session": session}));
    second.ok("window.set_focused", json!({"focused": true}));
    bell(&mut rpc, &split, "second-window", false);
    rpc.ok("tab.activate", json!({"tab": other_tab}));
    bell(&mut rpc, &split, "close-activates", true);
    rpc.ok("tab.close", json!({"tab": other_tab}));
    assert_eq!(
        rpc.workspace()["panes"][split.as_str().unwrap()]["bell"],
        false
    );
}

#[test]
fn initial_reports_survive_spawn_and_an_immediate_exit() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let (session, _, _) = create(&mut rpc);
    let created = rpc.ok("tab.create", json!({"session": session, "argv": ["/bin/bash", "-c", "printf '\\033]7;file://localhost/startup/here\\007\\033]2;initial\\007\\007'; exit 1"]}));
    let pane = &created["pane"];
    wait_pane(&mut rpc, pane, |p| {
        p["status"]["state"] == "exited"
            && p["cwd"] == "/startup/here"
            && p["title"] == "here"
            && p["bell"] == true
    });
    std::fs::write(
        env.config_home().join("shoal/config.toml"),
        format!("{}[tabs]\ntitle = 'program'\n", support::config(false)),
    )
    .unwrap();
    wait_pane(&mut rpc, pane, |p| {
        p["title"] == "initial" && p["program"].is_null()
    });
}
