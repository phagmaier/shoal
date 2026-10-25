use shoal_config::{PathError, XdgPaths};
use std::{collections::BTreeMap, ffi::OsString, path::Path};

fn paths(entries: &[(&str, &str)]) -> Result<XdgPaths, PathError> {
    let entries: BTreeMap<_, _> = entries
        .iter()
        .map(|&(k, v)| (k, OsString::from(v)))
        .collect();
    XdgPaths::from_lookup(|key| entries.get(key).cloned())
}

#[test]
fn xdg_overrides_and_derived_paths_are_used_without_creating_anything() {
    let p = paths(&[
        ("HOME", "/sandbox/home"),
        ("XDG_CONFIG_HOME", "/sandbox/config"),
        ("XDG_STATE_HOME", "/sandbox/state"),
        ("XDG_CACHE_HOME", "/sandbox/cache"),
        ("XDG_RUNTIME_DIR", "/sandbox/run"),
    ])
    .unwrap();
    assert_eq!(
        p.config_file(),
        Path::new("/sandbox/config/shoal/config.toml")
    );
    assert_eq!(
        p.workspace_file(),
        Path::new("/sandbox/state/shoal/workspace.json")
    );
    assert_eq!(p.panes_dir(), Path::new("/sandbox/state/shoal/panes"));
    assert_eq!(p.logs_dir(), Path::new("/sandbox/state/shoal/logs"));
    assert_eq!(
        p.runtimes_file(),
        Path::new("/sandbox/cache/shoal/runtimes.json")
    );
    assert_eq!(
        p.socket_file().unwrap(),
        Path::new("/sandbox/run/shoal/shoald.sock")
    );
    assert_eq!(
        p.lock_file().unwrap(),
        Path::new("/sandbox/run/shoal/shoald.lock")
    );
}

#[test]
fn unset_empty_and_relative_xdg_roots_fall_back_to_home_and_runtime_is_separate() {
    for value in [None, Some(""), Some("relative/path")] {
        let mut entries = vec![("HOME", "/sandbox/home")];
        if let Some(value) = value {
            entries.extend([
                ("XDG_CONFIG_HOME", value),
                ("XDG_STATE_HOME", value),
                ("XDG_CACHE_HOME", value),
                ("XDG_RUNTIME_DIR", value),
            ]);
        }
        let p = paths(&entries).unwrap();
        assert_eq!(
            p.config_file(),
            Path::new("/sandbox/home/.config/shoal/config.toml")
        );
        assert_eq!(
            p.workspace_file(),
            Path::new("/sandbox/home/.local/state/shoal/workspace.json")
        );
        assert_eq!(
            p.runtimes_file(),
            Path::new("/sandbox/home/.cache/shoal/runtimes.json")
        );
        assert!(matches!(p.socket_file(), Err(PathError::MissingRuntime)));
    }
    for entries in [&[][..], &[("HOME", "")][..], &[("HOME", "relative")][..]] {
        assert!(matches!(paths(entries), Err(PathError::MissingHome)));
    }
}

#[test]
fn non_utf8_home_is_preserved() {
    use std::os::unix::ffi::OsStringExt;
    let home = OsString::from_vec(b"/sandbox/h\xffme".to_vec());
    let p = XdgPaths::from_lookup(|key| (key == "HOME").then(|| home.clone())).unwrap();
    assert_eq!(
        p.config_file(),
        Path::new(&home).join(".config/shoal/config.toml")
    );
}
