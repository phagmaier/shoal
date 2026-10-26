//! T40: `shoal` and `shoal attach` check the daemon, then start the
//! `shoal-gui` beside them detached. A copy of `shoal` sits next to a fake
//! `shoal-gui` that records how it was started, so no test opens a window.

use std::{
    fs, io,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::Output,
};

use shoal_testkit::{TempEnv, eventually, shoald};

const SHOAL: &str = env!("CARGO_BIN_EXE_shoal");

/// A folder holding a copy of `shoal` and a fake `shoal-gui`, which writes
/// its pid, session id, folder and arguments to `gui.log`, one per line.
struct Prefix {
    bin: PathBuf,
    log: PathBuf,
}

impl Prefix {
    fn new(env: &TempEnv) -> Self {
        let bin = env.data_home().join("prefix/bin");
        fs::create_dir_all(&bin).unwrap();
        // A copy, not a link: `shoal` finds `shoal-gui` beside its real path.
        fs::copy(SHOAL, bin.join("shoal")).unwrap();
        let log = env.data_home().join("gui.log");
        let gui = bin.join("shoal-gui");
        fs::write(
            &gui,
            format!(
                "#!/bin/sh\nsid=$(cut -d' ' -f6 /proc/$$/stat)\n\
                 printf '%s\\n' \"$$\" \"$sid\" \"$PWD\" \"$@\" > '{log}.tmp'\n\
                 mv '{log}.tmp' '{log}'\n",
                log = log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&gui, fs::Permissions::from_mode(0o755)).unwrap();
        Self { bin, log }
    }

    fn shoal(&self, env: &TempEnv, cwd: &Path, args: &[&str]) -> Output {
        // A test running in parallel may fork while the fresh copy is still
        // open for writing; its exec then fails until that child execs too.
        eventually("shoal to start", || {
            match env
                .command(self.bin.join("shoal"))
                .current_dir(cwd)
                .args(args)
                .output()
            {
                Err(error) if error.kind() == io::ErrorKind::ExecutableFileBusy => None,
                output => Some(output.unwrap()),
            }
        })
    }

    /// How the fake window was started: once it has written its log.
    fn started(&self) -> Vec<String> {
        let lines = eventually("the window to start", || fs::read_to_string(&self.log).ok());
        fs::remove_file(&self.log).unwrap();
        lines.lines().map(str::to_owned).collect()
    }
}

fn fixture() -> TempEnv {
    let env = TempEnv::new();
    fs::create_dir_all(env.config_home().join("shoal")).unwrap();
    fs::write(
        env.config_home().join("shoal/config.toml"),
        "[shell]\nprogram = '/bin/bash'\nintegration = 'off'\n",
    )
    .unwrap();
    env
}

fn succeeded(output: &Output) -> String {
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn failed(output: &Output, code: i32) -> String {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn shoal_opens_a_detached_window_in_the_current_folder() {
    let env = fixture();
    let _daemon = env.spawn_daemon(shoald());
    let prefix = Prefix::new(&env);
    let code = env.home().join("code");
    fs::create_dir(&code).unwrap();

    assert_eq!(succeeded(&prefix.shoal(&env, &code, &[])), "");
    let started = prefix.started();
    let [pid, sid, cwd] = &started[..] else {
        panic!("no arguments for smart open: {started:?}")
    };
    // Its own session: closing the terminal can't hang the window up.
    assert_eq!(pid, sid);
    assert_eq!(Path::new(cwd), code);
}

#[test]
fn attach_names_the_session_and_rejects_other_targets() {
    let env = fixture();
    let _daemon = env.spawn_daemon(shoald());
    let prefix = Prefix::new(&env);
    let home = env.home();
    let session = succeeded(&prefix.shoal(&env, &home, &["new", "-n", "work"]));
    let session = session.trim();

    for target in ["work", session] {
        succeeded(&prefix.shoal(&env, &home, &["attach", target]));
        assert_eq!(prefix.started()[3..], ["--session", session], "{target}");
    }

    let ls = succeeded(&prefix.shoal(&env, &home, &["ls", "--json"]));
    let ls: serde_json::Value = serde_json::from_str(&ls).unwrap();
    let tab = ls["workspace"]["sessions"][0]["tabs"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        failed(&prefix.shoal(&env, &home, &["attach", &tab]), 1),
        format!("shoal: {tab} isn't a session\n")
    );
    assert_eq!(
        failed(&prefix.shoal(&env, &home, &["attach", "nope"]), 1),
        "shoal: no session, tab or pane called \"nope\"\n"
    );
    assert!(!prefix.log.exists(), "no window for a bad target");
}

#[test]
fn no_window_starts_when_the_daemon_cannot() {
    // No `shoald` beside the copy or on PATH, and systemd is absent.
    let env = fixture();
    let prefix = Prefix::new(&env);
    let stderr = failed(&prefix.shoal(&env, &env.home(), &[]), 3);
    assert!(
        stderr.starts_with("shoal: can't reach shoald: "),
        "{stderr}"
    );
    assert_eq!(env.systemctl_calls(), ["--user start shoald.service"]);
    assert!(!prefix.log.exists());
}
