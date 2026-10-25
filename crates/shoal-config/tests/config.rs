use shoal_config::{Action, Config};
use std::path::Path;

fn parse(source: &str) -> shoal_config::LoadedConfig {
    Config::parse(Path::new("/sandbox/config.toml"), source).unwrap()
}

#[test]
fn empty_and_explicit_default_files_match_every_spec_default() {
    let explicit = parse(include_str!("fixtures/defaults.toml"));
    assert!(explicit.warnings.is_empty());
    assert_eq!(parse(""), explicit);
    assert_eq!(Config::default(), explicit.config);
}

#[test]
fn partial_tables_preserve_other_defaults_and_unknown_keys_warn() {
    let loaded =
        parse("mystery = true\n[font]\nsize = 14.5\nfont_typo = 8\n[window]\nopacity = 1\n");
    assert_eq!(loaded.config.font.size, 14.5);
    assert_eq!(loaded.config.font.family, "JetBrains Mono");
    assert_eq!(loaded.config.font.line_height, 1.0);
    assert_eq!(loaded.config.window.opacity, 1.0);
    assert_eq!(loaded.config.window.padding, 8);
    assert_eq!(loaded.config.scrollback.lines, 10000);
    let mut keys: Vec<_> = loaded.warnings.iter().map(|w| w.key.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["font.font_typo", "mystery"]);
    assert!(
        loaded
            .warnings
            .iter()
            .all(|w| w.path == Path::new("/sandbox/config.toml"))
    );
}

#[test]
fn overrides_are_per_chord_and_none_unbinds_only_that_chord() {
    let loaded = parse(
        "[keybinds]\n'ctrl+shift+t' = 'none'\n'ctrl+f2' = 'tab.new'\n'ctrl+insert' = 'pane.zoom'\n",
    );
    let bindings = loaded.config.keybinds;
    assert_eq!(bindings.get(&"ctrl+shift+t".parse().unwrap()), None);
    assert_eq!(
        bindings.get(&"ctrl+f2".parse().unwrap()),
        Some(Action::TabNew)
    );
    assert_eq!(
        bindings.get(&"ctrl+insert".parse().unwrap()),
        Some(Action::PaneZoom)
    );
    assert_eq!(
        bindings.get(&"ctrl+shift+c".parse().unwrap()),
        Some(Action::ClipboardCopy)
    );
    assert_eq!(
        parse("[keybinds]\n'f2' = 'tab.new'")
            .config
            .keybinds
            .get(&"ctrl+shift+t".parse().unwrap()),
        Some(Action::TabNew)
    );
}

#[test]
fn bad_values_report_the_file_and_exact_value_line() {
    let path = Path::new("/sandbox/invalid.toml");
    for (section, assignment, reason) in [
        ("font", "size = 0", "positive"),
        ("font", "line_height = -1", "positive"),
        ("font", "size = nan", "finite"),
        ("font", "line_height = inf", "finite"),
        ("font", "size = 'large'", "invalid type"),
        ("window", "opacity = -0.1", "0..=1"),
        ("window", "opacity = 1.1", "0..=1"),
        ("window", "opacity = 1.00000001", "0..=1"),
        ("window", "opacity = nan", "finite"),
        ("window", "padding = -1", "u32"),
        ("window", "gap = -1", "u32"),
        ("window", "radius = -1", "u32"),
        ("scrollback", "lines = 100001", "100000"),
        ("scrollback", "lines = -1", "u32"),
        ("cursor", "style = 'beam'", "unknown variant"),
        ("cursor", "blink = 'yes'", "invalid type"),
        ("cursor", "color = '#fff'", "palette key"),
        ("cursor", "color = 'oops'", "palette key"),
        ("theme.colors", "accent = '#ggffff'", "#rrggbb"),
        ("shell", "integration = 'sometimes'", "unknown variant"),
        ("prompt", "mode = 'custom'", "unknown variant"),
        ("prompt", "style = 'fancy'", "unknown variant"),
        ("prompt", "segments = ['path', 'bogus']", "unknown variant"),
        ("prompt", "segments = ['user']", "path exactly once"),
        ("prompt", "segments = ['path', 'path']", "path exactly once"),
        ("tabs", "title = 'fixed'", "unknown variant"),
        ("keybinds", "'ctrl+t' = 'tab.typo'", "unknown action"),
        (
            "keybinds",
            "'ctrl+ctrl+t' = 'tab.new'",
            "repeated chord modifier",
        ),
        ("keybinds", "'f13' = 'tab.new'", "unknown chord key"),
    ] {
        let source = format!("# comment\n[{section}]\n{assignment}\n");
        let error = Config::parse(path, &source).unwrap_err();
        match error {
            shoal_config::ConfigError::Invalid {
                path: actual,
                line,
                reason: actual_reason,
            } => {
                assert_eq!(actual, path);
                assert_eq!(line, 3, "{source}: {actual_reason}");
                assert!(actual_reason.contains(reason), "{source}: {actual_reason}");
            }
            other => panic!("{source}: {other}"),
        }
    }
}

#[test]
fn syntax_errors_and_normalized_duplicate_chords_are_rejected() {
    let error = Config::parse(Path::new("/sandbox/config.toml"), "[font]\nsize = [").unwrap_err();
    assert!(
        error.to_string().starts_with("/sandbox/config.toml:2:"),
        "{error}"
    );
    let error = Config::parse(
        Path::new("/sandbox/config.toml"),
        "[keybinds]\n'CTRL+t' = 'tab.new'\n'ctrl+T' = 'pane.close'\n",
    )
    .unwrap_err();
    assert!(
        error.to_string().starts_with("/sandbox/config.toml:3:"),
        "{error}"
    );
    assert!(error.to_string().contains("duplicate normalized key chord"));
}

#[test]
fn limits_colors_and_all_enum_alternatives_are_loadable() {
    use shoal_config::schema::*;
    let loaded = parse(
        "[font]\nsize = 0.5\nline_height = 0.1\n[window]\nopacity = 0\npadding = 0\ngap = 0\nradius = 0\n[scrollback]\nlines = 100000\npersist = false\n[theme]\nsource = '/fixture/colors.toml'\n[theme.colors]\naccent = '#fF40a3'\n[cursor]\nstyle = 'bar'\nblink = true\ncolor = '#0102ff'\n[shell]\nprogram = '/bin/zsh'\nintegration = 'manual'\n[prompt]\nmode = 'shell'\nstyle = 'lean'\nsegments = ['path']\n[selection]\ncopy_on_select = true\n[tabs]\ntitle = 'program'\n[behavior]\nclose_pane_on_exit = false\nconfirm_close_running = false\n",
    );
    assert!(loaded.warnings.is_empty());
    let c = loaded.config;
    assert_eq!(c.window.opacity, 0.0);
    assert_eq!(c.scrollback.lines, 100000);
    assert!(!c.scrollback.persist);
    assert_eq!(c.theme.source, "/fixture/colors.toml");
    assert_eq!(c.theme.colors["accent"], Rgb([255, 64, 163]));
    assert_eq!(c.cursor.style, CursorStyle::Bar);
    assert!(c.cursor.blink);
    assert_eq!(c.cursor.color, "#0102ff");
    assert_eq!(c.shell.program, "/bin/zsh");
    assert_eq!(c.shell.integration, ShellIntegration::Manual);
    assert_eq!(c.prompt.mode, PromptMode::Shell);
    assert_eq!(c.prompt.style, PromptStyle::Lean);
    assert_eq!(c.prompt.segments, [PromptSegment::Path]);
    assert!(c.selection.copy_on_select);
    assert_eq!(c.tabs.title, TabTitle::Program);
    assert!(!c.behavior.close_pane_on_exit);
    assert!(!c.behavior.confirm_close_running);
    assert_eq!(parse("[scrollback]\nlines = 0").config.scrollback.lines, 0);
    assert_eq!(
        parse("[shell]\nintegration = 'off'\n[cursor]\nstyle = 'underline'")
            .config
            .shell
            .integration,
        ShellIntegration::Off
    );
    for key in PALETTE_KEYS {
        assert_eq!(
            parse(&format!("[cursor]\ncolor = '{key}'"))
                .config
                .cursor
                .color,
            *key
        );
    }
}

#[test]
fn real_file_reload_retains_last_good_config_and_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let mut file = shoal_config::ConfigFile::new(&path);
    assert_eq!(file.reload().unwrap().config, Config::default());
    std::fs::write(&path, "[font]\nsize = 18").unwrap();
    assert_eq!(file.reload().unwrap().config.font.size, 18.0);
    std::fs::write(&path, "[font]\nsize = 0").unwrap();
    assert!(file.reload().is_err());
    assert_eq!(file.current().config.font.size, 18.0);
    std::fs::write(&path, b"# first line\n\xff").unwrap();
    assert!(
        file.reload()
            .unwrap_err()
            .to_string()
            .starts_with(&format!("{}:2:", path.display()))
    );
    assert_eq!(file.current().config.font.size, 18.0);
    std::fs::write(&path, "[font]\nsize = 20").unwrap();
    assert_eq!(file.reload().unwrap().config.font.size, 20.0);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(file.reload().unwrap().config, Config::default());
    std::fs::create_dir(&path).unwrap();
    assert!(matches!(
        file.reload(),
        Err(shoal_config::ConfigError::Io { .. })
    ));
}
