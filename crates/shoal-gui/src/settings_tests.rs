use std::{fs, path::Path};

use shoal_config::schema::Rgb;
use shoal_testkit::{TempEnv, eventually};

use super::*;

fn paths(env: &TempEnv) -> XdgPaths {
    XdgPaths::from_lookup(|key| match key {
        "HOME" => Some(env.home().into_os_string()),
        "XDG_CONFIG_HOME" => Some(env.config_home().into_os_string()),
        "XDG_STATE_HOME" => Some(env.state_home().into_os_string()),
        _ => None,
    })
    .unwrap()
}

fn next(updates: &mut UnboundedReceiver<Update>, check: impl Fn(&Update) -> bool) -> Update {
    eventually("window settings update", || {
        while let Ok(update) = updates.try_recv() {
            if check(&update) {
                return Some(update);
            }
        }
        None
    })
}

#[test]
fn native_updates_apply_theme_overrides_and_keep_last_good_config() {
    let env = TempEnv::new();
    let paths = paths(&env);
    let folder = paths.state_home().join("omarchy/current/theme");
    fs::create_dir_all(&folder).unwrap();
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(folder.join("colors.toml"), "accent = '#123456'\n").unwrap();
    let (mut updates, watchers) = watch(paths.clone()).unwrap();
    next(&mut updates, |u| {
        u.palette.accent == Rgb([0x12, 0x34, 0x56])
    });
    fs::write(
        paths.config_file(),
        "[font]\nsize = 14\n[cursor]\ncolor = '#fedcba'\n[theme.colors]\naccent = '#abcdef'\n",
    )
    .unwrap();
    let updated = next(&mut updates, |u| u.config.font.size == 14.0);
    assert_eq!(updated.palette.accent, Rgb([0xab, 0xcd, 0xef]));
    assert_eq!(updated.palette.cursor, Rgb([0xfe, 0xdc, 0xba]));
    fs::write(paths.config_file(), "[font]\nsize = -2\n").unwrap();
    let invalid = next(&mut updates, |u| !u.notices.is_empty());
    assert_eq!(invalid.config.font.size, 14.0);
    assert_eq!(invalid.palette, updated.palette);
    assert!(
        invalid
            .notices
            .iter()
            .any(|(_, message)| message.contains("finite and positive"))
    );
    fs::write(folder.join("colors.toml"), "foreground = '#778899'\n").unwrap();
    let themed = next(&mut updates, |u| {
        u.palette.foreground == Rgb([0x77, 0x88, 0x99])
    });
    assert_eq!(themed.palette.accent, Rgb([0xab, 0xcd, 0xef]));
    drop(watchers);
    // Dropping the window stops the coordinator and both native workers.
    eventually("settings workers stop", || {
        matches!(
            updates.try_recv(),
            Err(futures::channel::mpsc::TryRecvError::Closed)
        )
        .then_some(())
    });
}

#[test]
fn queued_old_source_updates_cannot_win_after_config_switches_source() {
    let env = TempEnv::new();
    let paths = paths(&env);
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(
        paths.config_dir().join("first.toml"),
        "accent = '#111111'\n",
    )
    .unwrap();
    fs::write(
        paths.config_dir().join("second.toml"),
        "accent = '#222222'\n",
    )
    .unwrap();
    let config = Config::parse(Path::new("config.toml"), "[theme]\nsource = 'first.toml'\n")
        .unwrap()
        .config;
    let source = ThemeSource::from_config(&config.theme, &paths);
    let old = source.load();
    let (control, _events) = mpsc::channel();
    let mut state = State {
        paths,
        config,
        theme: old.clone(),
        source,
        generation: 0,
        theme_watcher: None,
        _config_watcher: None,
        control,
    };
    let loaded = Config::parse(
        Path::new("config.toml"),
        "[theme]\nsource = 'second.toml'\n",
    )
    .unwrap();
    assert!(state.apply(
        Event::Config(Box::new(ConfigUpdate {
            loaded,
            error: None
        })),
        &mut Vec::new()
    ));
    assert!(!state.apply(
        Event::Theme(
            0,
            ThemeUpdate {
                theme: old,
                error: None
            }
        ),
        &mut Vec::new()
    ));
    assert_eq!(
        state.theme.palette(&state.config).accent,
        Rgb([0x22, 0x22, 0x22])
    );
}
