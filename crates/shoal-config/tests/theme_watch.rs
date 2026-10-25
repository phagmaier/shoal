use shoal_config::{Config, ThemeSource, ThemeUpdate, ThemeWatcher, XdgPaths, schema::Rgb};
use std::{
    fs,
    os::unix::fs::symlink,
    path::Path,
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_secs(5);
// Longer than the 100 ms debounce, so a stray reload would have arrived.
const QUIET: Duration = Duration::from_millis(300);

fn xdg(home: &Path) -> XdgPaths {
    XdgPaths::from_lookup(|key| (key == "HOME").then(|| home.as_os_str().to_owned())).unwrap()
}

fn colors(folder: &Path, accent: &str) {
    fs::create_dir_all(folder).unwrap();
    fs::write(
        folder.join("colors.toml"),
        format!("accent = \"{accent}\"\nbackground = \"#101010\"\n"),
    )
    .unwrap();
}

fn watch(home: &Path) -> (ThemeWatcher, ThemeUpdate, Receiver<ThemeUpdate>) {
    let source = ThemeSource::from_config(&Config::default().theme, &xdg(home));
    let (tx, rx) = mpsc::channel();
    let (watcher, initial) = ThemeWatcher::new(source, move |update| {
        let _ = tx.send(update);
    })
    .unwrap();
    (watcher, initial, rx)
}

fn accent(update: &ThemeUpdate) -> Rgb {
    assert!(update.error.is_none(), "{:?}", update.error);
    update.theme.palette(&Config::default()).accent
}

fn next(rx: &Receiver<ThemeUpdate>, hex: &str) {
    let update = rx.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(accent(&update), hex.parse().unwrap());
}

fn quiet(rx: &Receiver<ThemeUpdate>) {
    assert!(matches!(
        rx.recv_timeout(QUIET),
        Err(RecvTimeoutError::Timeout)
    ));
}

#[test]
fn an_omarchy_theme_swap_fires_exactly_one_reload() {
    let home = tempfile::tempdir().unwrap();
    let current = home.path().join(".local/state/omarchy/current");
    colors(&current.join("theme"), "#111111");
    fs::write(current.join("theme.name"), "first\n").unwrap();
    let (_watcher, initial, rx) = watch(home.path());
    assert_eq!(accent(&initial), "#111111".parse().unwrap());

    // omarchy-theme-set stages and renders the next theme beside the current one.
    let staging = current.join("next-theme");
    colors(&staging, "#222222");
    fs::write(staging.join("alacritty.toml"), "# rendered").unwrap();
    quiet(&rx);
    // Then it swaps the folder in and records the name.
    fs::remove_dir_all(current.join("theme")).unwrap();
    fs::rename(&staging, current.join("theme")).unwrap();
    fs::write(current.join("theme.name"), "second\n").unwrap();
    next(&rx, "#222222");
    quiet(&rx);
    // Repointing the background afterwards is not a theme change.
    symlink(
        current.join("theme/colors.toml"),
        current.join("background"),
    )
    .unwrap();
    quiet(&rx);
}

#[test]
fn edits_3x_symlink_switches_and_a_late_omarchy_4_folder_reload() {
    let home = tempfile::tempdir().unwrap();
    let themes = home.path().join(".config/omarchy/themes");
    colors(&themes.join("one"), "#111111");
    colors(&themes.join("two"), "#222222");
    let current = home.path().join(".config/omarchy/current");
    fs::create_dir_all(&current).unwrap();
    symlink(themes.join("one"), current.join("theme")).unwrap();
    let (_watcher, initial, rx) = watch(home.path());
    assert_eq!(accent(&initial), "#111111".parse().unwrap());

    // An edit inside the symlinked theme folder.
    fs::write(themes.join("one/shoal.toml"), "accent = \"#333333\"\n").unwrap();
    next(&rx, "#333333");
    // Omarchy 3.x switches by repointing the symlink.
    fs::remove_file(current.join("theme")).unwrap();
    symlink(themes.join("two"), current.join("theme")).unwrap();
    next(&rx, "#222222");
    // An unrelated theme's edit does not count.
    fs::write(themes.join("one/colors.toml"), "accent = \"#444444\"\n").unwrap();
    quiet(&rx);
    // Upgrading to Omarchy 4 creates the state folder, which then takes over.
    colors(
        &home.path().join(".local/state/omarchy/current/theme"),
        "#555555",
    );
    next(&rx, "#555555");
}
