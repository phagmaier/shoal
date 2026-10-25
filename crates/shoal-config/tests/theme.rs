use shoal_config::{Config, LoadedTheme, Palette, ThemeSource, XdgPaths, schema::Rgb};
use std::{
    fs,
    path::{Path, PathBuf},
};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/themes");

fn xdg(home: &Path) -> XdgPaths {
    XdgPaths::from_lookup(|key| (key == "HOME").then(|| home.as_os_str().to_owned())).unwrap()
}

fn omarchy4(home: &Path) -> PathBuf {
    home.join(".local/state/omarchy/current/theme")
}

fn install(fixture: &str, folder: &Path) {
    fs::create_dir_all(folder).unwrap();
    for name in ["colors.toml", "shoal.toml"] {
        let source = Path::new(FIXTURES).join(fixture).join(name);
        if source.exists() {
            fs::copy(source, folder.join(name)).unwrap();
        }
    }
}

fn expected(file: &Path) -> Palette {
    let text = fs::read_to_string(file).unwrap();
    let mut roles = text.lines().map(|line| {
        let (role, hex) = line.split_once('\t').unwrap();
        (role.to_owned(), hex.parse::<Rgb>().unwrap())
    });
    let mut next = |name: &str| {
        let (role, rgb) = roles.next().unwrap();
        assert_eq!(role, name, "{}", file.display());
        rgb
    };
    Palette {
        background: next("background"),
        foreground: next("foreground"),
        cursor: next("cursor"),
        selection_background: next("selection_background"),
        selection_foreground: next("selection_foreground"),
        ansi: std::array::from_fn(|index| next(&format!("ansi{index}"))),
        top_bar: next("top_bar"),
        accent: next("accent"),
        unfocused_border: next("unfocused_border"),
        inactive_text: next("inactive_text"),
    }
}

fn load(config: &Config, home: &Path) -> LoadedTheme {
    ThemeSource::from_config(&config.theme, &xdg(home)).load()
}

#[test]
fn fixture_themes_map_to_exact_expected_palettes() {
    let config = Config::default();
    for fixture in ["dark", "light", "legacy", "shoal"] {
        let home = tempfile::tempdir().unwrap();
        install(fixture, &omarchy4(home.path()));
        let theme = load(&config, home.path());
        assert_eq!(theme.folder.as_deref(), Some(&*omarchy4(home.path())));
        let want = expected(&Path::new(FIXTURES).join(fixture).join("expected.tsv"));
        assert_eq!(theme.palette(&config), want, "fixture {fixture}");
        if fixture == "shoal" {
            let [warning] = &theme.warnings[..] else {
                panic!("{:?}", theme.warnings)
            };
            assert_eq!(warning.path, omarchy4(home.path()).join("shoal.toml"));
            assert_eq!(warning.line, Some(5));
            assert!(warning.reason.contains("bright_red"), "{warning}");
        } else {
            assert_eq!(theme.warnings, [], "fixture {fixture}");
        }
    }
}

#[test]
fn without_a_theme_folder_the_builtin_palette_is_tokyo_night() {
    let home = tempfile::tempdir().unwrap();
    let config = Config::default();
    let theme = load(&config, home.path());
    assert_eq!(theme.folder, None);
    assert_eq!(theme.warnings, []);
    let builtin = expected(&Path::new(FIXTURES).join("builtin-expected.tsv"));
    assert_eq!(theme.palette(&config), builtin);
    // An Omarchy-generated folder without colors.toml also falls back per key.
    fs::create_dir_all(omarchy4(home.path())).unwrap();
    assert_eq!(load(&config, home.path()).palette(&config), builtin);
}

#[test]
fn omarchy_4_folder_wins_and_a_3x_symlinked_folder_is_used_alone() {
    use std::os::unix::fs::symlink;
    let home = tempfile::tempdir().unwrap();
    let config = Config::default();
    let themes = home.path().join(".config/omarchy/themes");
    install("light", &themes.join("lupine"));
    let legacy = home.path().join(".config/omarchy/current/theme");
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    symlink(themes.join("lupine"), &legacy).unwrap();
    let light = expected(&Path::new(FIXTURES).join("light/expected.tsv"));
    let theme = load(&config, home.path());
    assert_eq!(theme.folder.as_deref(), Some(&*legacy));
    assert_eq!(theme.palette(&config), light);

    install("dark", &omarchy4(home.path()));
    let theme = load(&config, home.path());
    assert_eq!(theme.folder.as_deref(), Some(&*omarchy4(home.path())));
    let dark = expected(&Path::new(FIXTURES).join("dark/expected.tsv"));
    assert_eq!(theme.palette(&config), dark);
}

#[test]
fn config_colors_override_every_theme_file_and_cursor_follows_config() {
    let home = tempfile::tempdir().unwrap();
    install("shoal", &omarchy4(home.path()));
    let mut config = Config::default();
    for (key, hex) in [
        ("accent", "#123456"),
        ("fg", "#abcdef"),
        ("color2", "#00ff00"),
    ] {
        config.theme.colors.insert(key.into(), hex.parse().unwrap());
    }
    let theme = load(&config, home.path());
    let mut want = expected(&Path::new(FIXTURES).join("shoal/expected.tsv"));
    let accent: Rgb = "#123456".parse().unwrap();
    let foreground: Rgb = "#abcdef".parse().unwrap();
    (want.accent, want.cursor) = (accent, accent);
    (want.foreground, want.ansi[7]) = (foreground, foreground);
    want.ansi[2] = "#00ff00".parse().unwrap();
    assert_eq!(theme.palette(&config), want);

    config.cursor.color = "red".into();
    assert_eq!(theme.palette(&config).cursor, want.ansi[1]);
    config.cursor.color = "#0a0b0c".into();
    assert_eq!(theme.palette(&config).cursor, Rgb([0x0a, 0x0b, 0x0c]));
    // Keys are also readable by name, as the prompt names them.
    let colors = theme.colors(&config.theme);
    assert_eq!(colors.get("magenta"), Some(want.ansi[5]));
    assert_eq!(colors.get("bg"), None);
}

#[test]
fn a_file_source_replaces_the_theme_folder_and_resolves_its_path() {
    let home = tempfile::tempdir().unwrap();
    install("shoal", &omarchy4(home.path()));
    let mine = home.path().join(".config/shoal/themes/mine.toml");
    fs::create_dir_all(mine.parent().unwrap()).unwrap();
    fs::copy(Path::new(FIXTURES).join("legacy/colors.toml"), &mine).unwrap();
    let legacy = expected(&Path::new(FIXTURES).join("legacy/expected.tsv"));
    let mut config = Config::default();
    for source in [
        "themes/mine.toml",
        "~/.config/shoal/themes/mine.toml",
        mine.to_str().unwrap(),
    ] {
        config.theme.source = source.into();
        assert_eq!(
            ThemeSource::from_config(&config.theme, &xdg(home.path())),
            ThemeSource::File(mine.clone())
        );
        let theme = load(&config, home.path());
        assert_eq!(theme.folder, None);
        assert_eq!(theme.warnings, []);
        assert_eq!(theme.palette(&config), legacy, "source {source}");
    }

    config.theme.source = "missing.toml".into();
    let theme = load(&config, home.path());
    let [warning] = &theme.warnings[..] else {
        panic!("{:?}", theme.warnings)
    };
    assert_eq!(warning.path, home.path().join(".config/shoal/missing.toml"));
    let builtin = expected(&Path::new(FIXTURES).join("builtin-expected.tsv"));
    assert_eq!(theme.palette(&config), builtin);
}

#[test]
fn unreadable_theme_files_warn_and_fall_through_whole() {
    let home = tempfile::tempdir().unwrap();
    let folder = omarchy4(home.path());
    install("dark", &folder);
    let config = Config::default();
    let dark = expected(&Path::new(FIXTURES).join("dark/expected.tsv"));
    for (contents, line) in [
        (&b"accent = \"#ffffff\"\n\naccent = \"#000000\"\n"[..], 3),
        (&b"accent = \"#ffffff\"\nred = \"\xff\"\n"[..], 2),
    ] {
        fs::write(folder.join("shoal.toml"), contents).unwrap();
        let theme = load(&config, home.path());
        let [warning] = &theme.warnings[..] else {
            panic!("{:?}", theme.warnings)
        };
        assert_eq!(warning.path, folder.join("shoal.toml"));
        assert_eq!(warning.line, Some(line));
        assert_eq!(theme.palette(&config), dark);
    }
    // A used key with a non-string value warns; unused keys never do.
    fs::write(
        folder.join("shoal.toml"),
        "mode = 3\n[hyprland]\nborder = 1\naccent = 7\n",
    )
    .unwrap();
    let theme = load(&config, home.path());
    assert_eq!(theme.palette(&config), dark);
    assert_eq!(theme.warnings, []);
    fs::write(folder.join("shoal.toml"), "mode = 3\naccent = 7\n").unwrap();
    let theme = load(&config, home.path());
    assert_eq!(theme.palette(&config), dark);
    let [warning] = &theme.warnings[..] else {
        panic!("{:?}", theme.warnings)
    };
    assert_eq!(warning.line, Some(2));
}
