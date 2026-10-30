use shoal_config::{ConfigUpdate, ConfigWatcher};
use std::{
    fs,
    path::Path,
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_secs(5);

fn watch(path: &Path) -> (ConfigWatcher, ConfigUpdate, Receiver<ConfigUpdate>) {
    let (tx, rx) = mpsc::channel();
    let (watcher, initial) = ConfigWatcher::new(path, move |update| {
        let _ = tx.send(update);
    })
    .unwrap();
    (watcher, initial, rx)
}

fn next(rx: &Receiver<ConfigUpdate>, size: f64) {
    let update = rx.recv_timeout(TIMEOUT).unwrap();
    assert!(update.error.is_none(), "{:?}", update.error);
    assert_eq!(update.loaded.config.font.size, size);
}

#[test]
fn watcher_debounces_burst_once_and_ignores_unrelated_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(&path, "[font]\nsize = 14").unwrap();
    let (_watcher, initial, rx) = watch(&path);
    assert!(initial.error.is_none());
    assert_eq!(initial.loaded.config.font.size, 14.0);
    for size in 15..=40 {
        fs::write(&path, format!("[font]\nsize = {size}")).unwrap();
    }
    next(&rx, 40.0);
    fs::write(dir.path().join("unrelated.toml"), "[font]\nsize = 99").unwrap();
    assert!(matches!(
        rx.recv_timeout(Duration::from_millis(300)),
        Err(RecvTimeoutError::Timeout)
    ));
}

#[test]
fn atomic_saves_bad_edits_and_recovery_keep_the_last_good_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let (_watcher, initial, rx) = watch(&path);
    assert_eq!(initial.loaded.config.font.size, 10.0);
    let staging = dir.path().join("staging.toml");
    fs::write(&staging, "[font]\nsize = 18").unwrap();
    fs::rename(&staging, &path).unwrap();
    next(&rx, 18.0);
    fs::write(&path, "[font]\nsize = -1").unwrap();
    let update = rx.recv_timeout(TIMEOUT).unwrap();
    assert!(
        update
            .error
            .unwrap()
            .to_string()
            .starts_with(&format!("{}:2:", path.display()))
    );
    assert_eq!(update.loaded.config.font.size, 18.0);
    fs::write(&staging, "[font]\nsize = 22").unwrap();
    fs::rename(&staging, &path).unwrap();
    next(&rx, 22.0);
    fs::remove_file(&path).unwrap();
    next(&rx, 10.0);
    fs::write(&path, "[font]\nsize = 24").unwrap();
    next(&rx, 24.0);
}

#[test]
fn missing_and_replaced_parent_directories_remain_watchable() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("xdg/shoal");
    let path = parent.join("config.toml");
    let (_watcher, initial, rx) = watch(&path);
    assert!(initial.error.is_none());
    assert!(
        !parent.exists(),
        "watching must not create config directories"
    );
    fs::create_dir_all(&parent).unwrap();
    fs::write(&path, "[font]\nsize = 16").unwrap();
    next(&rx, 16.0);
    fs::remove_dir_all(&parent).unwrap();
    fs::create_dir(&parent).unwrap();
    fs::write(&path, "[font]\nsize = 20").unwrap();
    next(&rx, 20.0);
    fs::write(&path, "[font]\nsize = 25").unwrap();
    next(&rx, 25.0);
}

#[test]
fn invalid_startup_reports_defaults_then_recovers_and_keeps_warnings_on_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    fs::write(&path, "[font]\nsize = 0").unwrap();
    let (_watcher, initial, rx) = watch(&path);
    assert!(initial.error.is_some());
    assert_eq!(initial.loaded.config.font.size, 10.0);
    fs::write(&path, "[font]\nsize = 17\ntypo = true").unwrap();
    let update = rx.recv_timeout(TIMEOUT).unwrap();
    assert!(update.error.is_none());
    assert_eq!(update.loaded.config.font.size, 17.0);
    assert_eq!(update.loaded.warnings[0].key, "font.typo");
    fs::write(&path, "[font]\nsize = -5").unwrap();
    let update = rx.recv_timeout(TIMEOUT).unwrap();
    assert!(update.error.is_some());
    assert_eq!(update.loaded.config.font.size, 17.0);
    assert_eq!(update.loaded.warnings[0].key, "font.typo");
}

#[test]
fn symlink_targets_and_retargeting_are_watched() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let targets = dir.path().join("targets");
    fs::create_dir(&config).unwrap();
    fs::create_dir(&targets).unwrap();
    let first = targets.join("first.toml");
    let second = targets.join("second.toml");
    fs::write(&first, "[font]\nsize = 15").unwrap();
    fs::write(&second, "[font]\nsize = 20").unwrap();
    let path = config.join("config.toml");
    symlink(&first, &path).unwrap();
    let (_watcher, initial, rx) = watch(&path);
    assert_eq!(initial.loaded.config.font.size, 15.0);
    fs::write(&first, "[font]\nsize = 18").unwrap();
    next(&rx, 18.0);
    fs::remove_file(&path).unwrap();
    symlink(&second, &path).unwrap();
    next(&rx, 20.0);
    fs::write(&second, "[font]\nsize = 22").unwrap();
    next(&rx, 22.0);
    fs::remove_file(&second).unwrap();
    next(&rx, 10.0);
    fs::write(&second, "[font]\nsize = 24").unwrap();
    next(&rx, 24.0);
}
