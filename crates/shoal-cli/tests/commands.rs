//! T21: `new`, `ls`, `kill`, `send` and `capture` through the real binary
//! and the real `shoald`, in a temporary HOME and XDG environment.

use std::{fs, process::Output};

use serde_json::{Value, json};
use shoal_client::proto::Response;
use shoal_testkit::{Connection, TempEnv, eventually, shoald};

const SHOAL: &str = env!("CARGO_BIN_EXE_shoal");

fn fixture() -> TempEnv {
    let env = TempEnv::new();
    fs::create_dir_all(env.config_home().join("shoal")).unwrap();
    fs::write(
        env.config_home().join("shoal/config.toml"),
        "[shell]\nprogram = '/bin/bash'\nintegration = 'off'\n[prompt]\nmode = 'shell'\n\
         [behavior]\nclose_pane_on_exit = false\n",
    )
    .unwrap();
    fs::write(
        env.home().join(".bashrc"),
        "PS1='fixture> '; unset PROMPT_COMMAND\n",
    )
    .unwrap();
    env
}

fn shoal(env: &TempEnv, args: &[&str]) -> Output {
    env.command(SHOAL).args(args).output().unwrap()
}

fn shoal_in(env: &TempEnv, pane: &str, args: &[&str]) -> Output {
    env.command(SHOAL)
        .env("SHOAL_PANE", pane)
        .args(args)
        .output()
        .unwrap()
}

/// Run it, expect success, and return stdout.
fn ok(env: &TempEnv, args: &[&str]) -> String {
    let output = shoal(env, args);
    assert!(output.status.success(), "{args:?}: {output:?}");
    assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

fn failed(output: &Output, code: i32) -> String {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn request(connection: &mut Connection, method: &str, params: Value) -> Value {
    match connection.request(1, method, params) {
        Response::Success { result, .. } => result,
        Response::Error { error, .. } => panic!("{method}: {error:?}"),
    }
}

fn workspace(env: &TempEnv) -> Value {
    serde_json::from_str(&ok(env, &["ls", "--json"])).unwrap()
}

fn first_pane(env: &TempEnv, session: &str) -> String {
    let state = workspace(env);
    let session = state["workspace"]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == session)
        .unwrap()
        .clone();
    session["tabs"][0]["focused"].as_str().unwrap().to_owned()
}

#[test]
fn ls_shows_sessions_tabs_and_panes_as_text_and_json() {
    let env = fixture();
    let _daemon = env.spawn_daemon(shoald());
    assert_eq!(ok(&env, &["ls"]), "No sessions.\n");
    for folder in ["code/shoal", "notes"] {
        fs::create_dir_all(env.home().join(folder)).unwrap();
    }
    let code = env.home().join("code/shoal");
    let work = ok(
        &env,
        &["new", "-n", "work", "--cwd", code.to_str().unwrap()],
    );
    assert_eq!(work, "s1\n");
    let second = ok(&env, &["new"]);
    assert_eq!(second, "s4\n");
    let mut connection = env.connect();
    connection.hello();
    let first = first_pane(&env, "s1");
    let notes = env.home().join("notes");
    request(
        &mut connection,
        "pane.split",
        json!({"pane": first, "direction": "right", "cwd": notes}),
    );
    request(
        &mut connection,
        "pane.zoom",
        json!({"pane": first, "zoomed": true}),
    );
    let tab = request(
        &mut connection,
        "tab.create",
        json!({"session": "s1", "argv": ["false"]}),
    );
    request(
        &mut connection,
        "tab.rename",
        json!({"tab": tab["tab"], "name": "failing"}),
    );
    // The failing command has ended and the titles have settled.
    eventually("the failed command's status", || {
        let state = workspace(&env);
        let pane = &state["workspace"]["panes"][tab["pane"].as_str().unwrap()];
        (pane["status"]["state"] == "exited").then_some(())
    });

    insta::assert_snapshot!("ls", ok(&env, &["ls"]));

    // The JSON is workspace.get's result, with times, HOME and the revision
    // (which metadata updates advance) redacted.
    let mut state = workspace(&env);
    state["rev"] = json!("[rev]");
    state["workspace"]["rev"] = json!("[rev]");
    for session in state["workspace"]["sessions"].as_array_mut().unwrap() {
        session["created_at"] = json!("[time]");
        session["last_active_at"] = json!("[time]");
    }
    let text = serde_json::to_string_pretty(&state)
        .unwrap()
        .replace(env.home().to_str().unwrap(), "~");
    insta::assert_snapshot!("ls_json", text);
}

#[test]
fn send_then_capture_round_trips_and_kill_closes() {
    let env = fixture();
    let _daemon = env.spawn_daemon(shoald());
    ok(&env, &["new", "-n", "main"]);
    let pane = first_pane(&env, "s1");
    eventually("the prompt", || {
        ok(&env, &["capture", "main"])
            .contains("fixture>")
            .then_some(())
    });
    // A session target means its focused pane; \n presses Enter.
    ok(&env, &["send", "main", "printf 'round-%s\\n' trip\\n"]);
    eventually("the output", || {
        ok(&env, &["capture", &pane])
            .contains("round-trip\n")
            .then_some(())
    });
    // `.` is the pane in $SHOAL_PANE.
    let output = shoal_in(&env, &pane, &["send", ".", "printf 'from-%s\\n' inside\\n"]);
    assert!(output.status.success(), "{output:?}");
    eventually("the second output", || {
        let output = shoal_in(&env, &pane, &["capture", ".", "--scrollback"]);
        String::from_utf8_lossy(&output.stdout)
            .contains("from-inside\n")
            .then_some(())
    });
    let vt = shoal(&env, &["capture", &pane, "--vt"]);
    assert!(vt.status.success(), "{vt:?}");
    assert!(String::from_utf8_lossy(&vt.stdout).contains("round-trip"));

    ok(&env, &["kill", "main"]);
    assert_eq!(ok(&env, &["ls"]), "No sessions.\n");
}

#[test]
fn errors_go_to_stderr_with_the_right_exit_codes() {
    let env = fixture();
    // 3: no daemon. Auto-start arrives with T22.
    let stderr = failed(&shoal(&env, &["ls"]), 3);
    assert!(
        stderr.starts_with("shoal: can't reach shoald: "),
        "{stderr}"
    );

    let _daemon = env.spawn_daemon(shoald());
    // 2: bad usage, with the usage text.
    let stderr = failed(&shoal(&env, &["kill"]), 2);
    assert!(
        stderr.starts_with("shoal kill: which target?\nusage: shoal new"),
        "{stderr}"
    );
    let stderr = failed(&shoal(&env, &["ls", "--bogus"]), 2);
    assert!(
        stderr.starts_with("shoal ls: unknown option --bogus\n"),
        "{stderr}"
    );

    // 1: errors, from the CLI or from the daemon.
    assert_eq!(
        failed(&shoal(&env, &["kill", "nope"]), 1),
        "shoal: no session, tab or pane called \"nope\"\n"
    );
    assert_eq!(
        failed(&shoal(&env, &["capture", "."]), 1),
        "shoal: `.` means this pane, but SHOAL_PANE isn't set\n"
    );
    let missing = env.home().join("missing");
    assert_eq!(
        failed(
            &shoal(&env, &["new", "--cwd", missing.to_str().unwrap()]),
            1
        ),
        format!("shoal: {} isn't a folder\n", missing.display())
    );
    ok(&env, &["new", "-n", "work"]);
    assert_eq!(
        failed(&shoal(&env, &["new", "-n", "work"]), 1),
        "shoal: session name \"work\" is taken\n"
    );
}
