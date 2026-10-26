use std::{
    fs,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use shoal_testkit::eventually;

use crate::{
    create,
    support::{Rpc, SHOALD, config, fixture},
};

fn capture(rpc: &mut Rpc, pane: &Value) -> String {
    rpc.ok(
        "pane.capture",
        json!({"pane": pane, "format": "text", "scrollback": true}),
    )["data"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn color(rpc: &mut Rpc, pane: &Value, sequence: &mut usize) -> String {
    *sequence += 1;
    let marker = format!("query-{} ", *sequence);
    let command = format!(
        r"printf '\033]11;?\033\\'; IFS= read -rs -d '\' -t 4 r; r=${{r%?}}; printf '\nquery-{} %s|\n' ${{r#*;}}",
        *sequence
    );
    rpc.ok(
        "pane.send_text",
        json!({"pane": pane, "text": format!("{command}\n")}),
    );
    eventually("terminal color reply", || {
        let text = capture(rpc, pane);
        let (_, rest) = text.rsplit_once(&format!("{marker}rgb:"))?;
        let (rgb, _) = rest.split_once('|')?;
        Some(format!("rgb:{rgb}"))
    })
}

#[test]
fn omarchy_directory_swap_recolors_an_existing_pane_within_one_second() {
    let env = fixture(false);
    let current = env.state_home().join("omarchy/current");
    fs::create_dir_all(current.join("theme")).unwrap();
    fs::write(
        current.join("theme/colors.toml"),
        "background = '#123456'\n",
    )
    .unwrap();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let (_, _, pane) = create(&mut rpc);
    let mut query = 0;
    assert_eq!(color(&mut rpc, &pane, &mut query), "rgb:1212/3434/5656");
    fs::create_dir(current.join("next-theme")).unwrap();
    fs::write(
        current.join("next-theme/colors.toml"),
        "background = '#abcdef'\n",
    )
    .unwrap();
    let started = Instant::now();
    fs::rename(current.join("theme"), current.join("previous-theme")).unwrap();
    fs::rename(current.join("next-theme"), current.join("theme")).unwrap();
    fs::write(current.join("theme.name"), "fixture-next\n").unwrap();
    eventually("new Omarchy color on running pane", || {
        (color(&mut rpc, &pane, &mut query) == "rgb:abab/cdcd/efef").then_some(())
    });
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "theme reload took {:?}",
        started.elapsed()
    );
}

#[test]
fn invalid_config_sends_a_notice_retains_last_good_settings_and_recovers() {
    let env = fixture(false);
    let file = env.config_home().join("shoal/config.toml");
    let initial = format!("{}[theme.colors]\nbackground = '#112233'\n", config(false));
    fs::write(&file, &initial).unwrap();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::hello(&env, "gui", json!({}));
    let (_, _, pane) = create(&mut rpc);
    let mut query = 0;
    assert_eq!(color(&mut rpc, &pane, &mut query), "rgb:1111/2222/3333");
    fs::write(&file, format!("{initial}[scrollback]\nlines = -1\n")).unwrap();
    loop {
        let notice = rpc.notification("daemon.notice");
        let message = notice["message"].as_str().unwrap();
        if message.contains("config.toml") {
            assert!(
                message.contains("config.toml:"),
                "file and line missing: {message}"
            );
            assert_eq!(notice["level"], "error");
            break;
        }
    }
    assert_eq!(color(&mut rpc, &pane, &mut query), "rgb:1111/2222/3333");
    rpc.ok(
        "pane.send_text",
        json!({"pane": pane, "text": "for i in {0..60}; do printf 'saved-%03d\\n' \"$i\"; done\n"}),
    );
    eventually("scrollback fixture finished", || {
        capture(&mut rpc, &pane).contains("saved-060").then_some(())
    });
    assert!(capture(&mut rpc, &pane).contains("saved-000"));
    fs::write(
        &file,
        format!(
            "{}[theme.colors]\nbackground = '#445566'\n[scrollback]\nlines = 0\n",
            config(false)
        ),
    )
    .unwrap();
    eventually("valid config recolors old pane", || {
        (color(&mut rpc, &pane, &mut query) == "rgb:4444/5555/6666").then_some(())
    });
    assert!(
        !capture(&mut rpc, &pane).contains("saved-000"),
        "live history limit did not apply"
    );
    // A client connected after recovery must not receive the obsolete config error.
    let mut later = Rpc::hello(&env, "gui", json!({}));
    later.workspace();
    assert!(
        !later
            .notices
            .iter()
            .any(|notice| notice["message"].as_str().unwrap().contains("config.toml"))
    );
}

#[test]
fn theme_source_switches_and_new_shell_settings_take_effect_without_restart() {
    let env = fixture(false);
    let file = env.config_home().join("shoal/config.toml");
    let theme = env.config_home().join("shoal/custom.toml");
    fs::write(&theme, "background = '#010203'\n").unwrap();
    let _daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::new(&env);
    let (session, _, pane) = create(&mut rpc);
    fs::write(
        &file,
        format!("{}[theme]\nsource = 'custom.toml'\n", config(false)),
    )
    .unwrap();
    let mut query = 0;
    eventually("configured theme source applies", || {
        (color(&mut rpc, &pane, &mut query) == "rgb:0101/0202/0303").then_some(())
    });
    fs::write(&theme, "background = '#070809'\n").unwrap();
    eventually("new theme source is watched", || {
        (color(&mut rpc, &pane, &mut query) == "rgb:0707/0808/0909").then_some(())
    });
    let shell = env.home().join("new-shell");
    fs::write(&shell, "#!/bin/sh\nprintf changed-shell-marker\nexit 0\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&shell, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(&file, format!("[shell]\nprogram = '{}'\nintegration = 'off'\n[prompt]\nmode = 'shell'\n[behavior]\nclose_pane_on_exit = false\n[theme.colors]\nbackground = '#101112'\n", shell.display())).unwrap();
    eventually("latest config reached the actor", || {
        (color(&mut rpc, &pane, &mut query) == "rgb:1010/1111/1212").then_some(())
    });
    let new_pane = rpc.ok("tab.create", json!({"session": session}))["pane"].clone();
    eventually("new shell program used", || {
        capture(&mut rpc, &new_pane)
            .contains("changed-shell-marker")
            .then_some(())
    });
}

#[test]
fn the_latest_gui_hello_supplies_only_allowlisted_environment_to_new_panes() {
    let env = fixture(false);
    let _daemon = env.spawn_daemon(SHOALD);
    let _first = Rpc::hello(&env, "gui", json!({"WAYLAND_DISPLAY": "wayland-old"}));
    let _second = Rpc::hello(
        &env,
        "gui",
        json!({"WAYLAND_DISPLAY": "wayland-new", "SHOAL_PRIVATE_TEST": "must-not-leak"}),
    );
    let mut rpc = Rpc::hello(&env, "cli", json!({"WAYLAND_DISPLAY": "not-a-window"}));
    let (_, _, pane) = create(&mut rpc);
    rpc.ok("pane.send_text", json!({"pane": pane, "text": "printf 'window-%s|private-%s|\\n' \"$WAYLAND_DISPLAY\" \"${SHOAL_PRIVATE_TEST-unset}\"\n"}));
    eventually("window environment in shell", || {
        capture(&mut rpc, &pane)
            .contains("window-wayland-new|private-unset|")
            .then_some(())
    });
}

#[test]
fn a_failed_initial_theme_watch_keeps_serving_and_a_new_source_recovers() {
    use std::{os::unix::fs::PermissionsExt, path::PathBuf};
    struct RestoreAccess(PathBuf);
    impl Drop for RestoreAccess {
        fn drop(&mut self) {
            fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    let env = fixture(false);
    let config_dir = env.config_home().join("shoal");
    let blocked = config_dir.join("blocked");
    fs::create_dir(&blocked).unwrap();
    fs::write(blocked.join("colors.toml"), "background = '#445566'\n").unwrap();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
    let _access = RestoreAccess(blocked);
    let file = config_dir.join("config.toml");
    fs::write(
        &file,
        format!(
            "{}[theme]\nsource = 'blocked/colors.toml'\n[theme.colors]\nbackground = '#123456'\n",
            config(false)
        ),
    )
    .unwrap();
    let daemon = env.spawn_daemon(SHOALD);
    let mut rpc = Rpc::hello(&env, "gui", json!({}));
    let (_, _, pane) = create(&mut rpc);
    let mut query = 0;
    assert_eq!(color(&mut rpc, &pane, &mut query), "rgb:1212/3434/5656");
    assert!(rpc.notices.iter().any(|notice| {
        notice["message"]
            .as_str()
            .unwrap()
            .contains("watch unavailable")
    }));
    fs::write(config_dir.join("readable.toml"), "background = '#abcdef'\n").unwrap();
    fs::write(
        &file,
        format!("{}[theme]\nsource = 'readable.toml'\n", config(false)),
    )
    .unwrap();
    eventually("new source recovers theme watching", || {
        (color(&mut rpc, &pane, &mut query) == "rgb:abab/cdcd/efef").then_some(())
    });
    fs::write(config_dir.join("readable.toml"), "background = '#070809'\n").unwrap();
    eventually("recovered theme source remains watched", || {
        (color(&mut rpc, &pane, &mut query) == "rgb:0707/0808/0909").then_some(())
    });
    rpc.ok("daemon.shutdown", json!({}));
    assert!(daemon.wait().status.success());
}
