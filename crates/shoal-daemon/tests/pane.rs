//! The pane runtime (M1 spec §4.2) with real shells in a temporary
//! environment, driven through the daemon's internal pane API.

#[path = "pane/attaching.rs"]
mod attaching;

use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use shoal_config::XdgPaths;
use shoal_daemon::{
    pane::{self, Event, Item, Pane, PaneSettings, PaneSpec},
    settings::Settings,
};
use shoal_model::{GridSize, PaneId};
use shoal_term::CaptureFormat;
use shoal_testkit::{TempEnv, eventually};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

const SIZE: GridSize = GridSize { cols: 80, rows: 24 };
static NEXT_PANE: AtomicU64 = AtomicU64::new(1);

struct Harness {
    env: TempEnv,
    sender: UnboundedSender<(PaneId, Event)>,
    events: UnboundedReceiver<(PaneId, Event)>,
}

impl Harness {
    /// A temporary environment whose bash and zsh show a known prompt.
    fn new() -> Self {
        let env = TempEnv::new();
        std::fs::create_dir_all(env.config_home().join("shoal")).unwrap();
        // T11 exercises the pane runtime without T30's prompt replacement.
        std::fs::write(
            env.config_home().join("shoal/config.toml"),
            "[shell]\nintegration = 'off'\n",
        )
        .unwrap();
        std::fs::write(env.home().join(".bashrc"), "PS1='bash$ '\n").unwrap();
        std::fs::write(env.home().join(".zshrc"), "PROMPT='zsh%% '\n").unwrap();
        let (sender, events) = unbounded_channel();
        Self {
            env,
            sender,
            events,
        }
    }

    /// Settings as the daemon loads them from this environment's config.
    fn settings(&self) -> PaneSettings {
        let vars = self.env.vars();
        let paths = XdgPaths::from_lookup(|key| {
            vars.iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        })
        .unwrap();
        Settings::load(&paths).pane(&paths, self.env.socket(), vars)
    }

    fn spec(&mut self, argv: &[&str]) -> PaneSpec {
        PaneSpec {
            id: PaneId::new(NEXT_PANE.fetch_add(1, Ordering::Relaxed)),
            size: SIZE,
            argv: argv.iter().map(|arg| (*arg).to_owned()).collect(),
            cwd: None,
            window_env: BTreeMap::new(),
        }
    }

    fn start(&mut self, spec: PaneSpec) -> Pane {
        pane::spawn(spec, &self.settings(), self.sender.clone()).unwrap()
    }

    fn spawn(&mut self, argv: &[&str]) -> Pane {
        let spec = self.spec(argv);
        self.start(spec)
    }

    /// The pane's exit, skipping its other events.
    fn exited(&mut self, pane: &Pane) -> (Option<i32>, Option<i32>, bool) {
        eventually("the shell to exit", || {
            while let Ok((id, event)) = self.events.try_recv() {
                if let (
                    true,
                    Event::Exited {
                        code,
                        signal,
                        early_failure,
                    },
                ) = (id == pane.id(), event)
                {
                    return Some((code, signal, early_failure));
                }
            }
            None
        })
    }
}

fn screen(pane: &Pane) -> String {
    let text = pane
        .capture(CaptureFormat::Text, false)
        .blocking_recv()
        .expect("the pane thread answers")
        .expect("capture");
    String::from_utf8(text).unwrap()
}

/// Wait until the screen contains `needle`; returns the screen.
fn wait_for(pane: &Pane, needle: &str) -> String {
    eventually(&format!("{needle:?} on screen"), || {
        let text = screen(pane);
        text.contains(needle).then_some(text)
    })
}

fn type_line(pane: &Pane, line: &str) {
    pane.write(format!("{line}\r").into_bytes());
}

#[test]
fn echo_round_trip_in_bash_and_zsh() {
    let mut harness = Harness::new();
    // Captures trim trailing spaces, so prompts show without theirs.
    for (shell, prompt) in [("bash", "bash$"), ("zsh", "zsh%")] {
        let pane = harness.spawn(&[shell]);
        wait_for(&pane, prompt);
        // The arithmetic shows the shell ran the line, not just echoed it.
        type_line(&pane, "echo round-$((6 * 7))-trip");
        let text = wait_for(&pane, "round-42-trip");
        assert!(
            text.contains("echo round-$((6 * 7))-trip"),
            "{shell}: {text}"
        );
    }
}

#[test]
fn stty_size_follows_a_resize() {
    let mut harness = Harness::new();
    let pane = harness.spawn(&["bash"]);
    let attachment = pane
        .attach(std::sync::Arc::new(tokio::sync::Notify::new()))
        .blocking_recv()
        .unwrap()
        .unwrap();
    let subscribed = attachment.subscribed;
    let mut items = attachment.items;
    wait_for(&pane, "bash$");
    type_line(&pane, "stty size");
    wait_for(&pane, "24 80");
    pane.resize(GridSize {
        cols: 100,
        rows: 30,
    });
    type_line(&pane, "clear; stty size");
    wait_for(&pane, "30 100");

    // A resize to the current size is not numbered again.
    pane.resize(GridSize {
        cols: 100,
        rows: 30,
    });
    // A snapshot numbers any output still being batched first.
    let snapshot = pane.snapshot().blocking_recv().unwrap().unwrap();
    assert_eq!(
        snapshot.size,
        GridSize {
            cols: 100,
            rows: 30
        }
    );

    // Subscribers saw consecutive items up to the snapshot, one resize among them.
    let mut seqs = Vec::new();
    let mut resizes = Vec::new();
    while let Ok(pane::Delivery::Item(item)) = items.try_recv() {
        if let Item::Resized { size, .. } = &item {
            resizes.push(*size);
        }
        seqs.push(item.seq());
    }
    let first = subscribed.seq + 1;
    assert_eq!(seqs, (first..first + seqs.len() as u64).collect::<Vec<_>>());
    assert!(seqs.contains(&snapshot.seq), "{seqs:?} {}", snapshot.seq);
    assert_eq!(
        resizes,
        [GridSize {
            cols: 100,
            rows: 30
        }]
    );
}

#[test]
fn exit_codes_and_the_early_exit_guard() {
    let mut harness = Harness::new();
    let pane = harness.spawn(&["bash", "-c", "exit 3"]);
    assert_eq!(harness.exited(&pane), (Some(3), None, true));

    let pane = harness.spawn(&["bash", "-c", "exit 0"]);
    assert_eq!(harness.exited(&pane), (Some(0), None, false));

    let pane = harness.spawn(&["bash", "-c", "sleep 1.1; exit 3"]);
    assert_eq!(harness.exited(&pane), (Some(3), None, false));
}

#[test]
fn an_exited_pane_keeps_its_screen_and_ignores_input() {
    let mut harness = Harness::new();
    let pane = harness.spawn(&["bash", "-c", "echo last words; exit 1"]);
    harness.exited(&pane);
    pane.write(b"more\r".to_vec());
    pane.resize(GridSize { cols: 60, rows: 10 });
    assert!(screen(&pane).contains("last words"));
    let snapshot = pane.snapshot().blocking_recv().unwrap().unwrap();
    assert_eq!(snapshot.size, GridSize { cols: 60, rows: 10 });
}

#[test]
fn kill_ends_sleep_with_a_hangup() {
    let mut harness = Harness::new();
    let pane = harness.spawn(&["sleep", "100"]);
    let start = Instant::now();
    pane.kill();
    assert_eq!(harness.exited(&pane), (None, Some(1), false));
    assert!(
        start.elapsed() < Duration::from_millis(2500),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn kill_escalates_when_the_hangup_is_ignored() {
    let mut harness = Harness::new();
    let pane = harness.spawn(&["bash", "-c", "trap '' HUP; echo ready; sleep 100; true"]);
    wait_for(&pane, "ready");
    let start = Instant::now();
    pane.kill();
    assert_eq!(harness.exited(&pane), (None, Some(9), false));
    let took = start.elapsed();
    assert!(
        took >= Duration::from_secs(2) && took < Duration::from_millis(2500),
        "{took:?}"
    );
}

#[test]
fn a_shared_shutdown_deadline_shortens_existing_grace_and_handles_expiry() {
    let mut harness = Harness::new();
    let pane = harness.spawn(&["bash", "-c", "trap '' HUP; echo ready; sleep 100; true"]);
    wait_for(&pane, "ready");
    let started = Instant::now();
    pane.kill();
    pane.kill_by(started + Duration::from_millis(200))
        .blocking_recv()
        .unwrap();
    assert_eq!(harness.exited(&pane), (None, Some(9), false));
    assert!(started.elapsed() >= Duration::from_millis(200));
    assert!(started.elapsed() < Duration::from_secs(1));

    let pane = harness.spawn(&["bash", "-c", "trap '' HUP; echo ready; sleep 100; true"]);
    wait_for(&pane, "ready");
    let started = Instant::now();
    pane.kill_by(started).blocking_recv().unwrap();
    assert_eq!(harness.exited(&pane), (None, Some(9), false));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn dropping_the_handle_ends_the_shell() {
    let mut harness = Harness::new();
    let pane = harness.spawn(&["sleep", "100"]);
    let id = pane.id();
    drop(pane);
    let exit = eventually("the shell to exit", || match harness.events.try_recv() {
        Ok((got, Event::Exited { signal, .. })) if got == id => Some(signal),
        _ => None,
    });
    assert_eq!(exit, Some(1));
}

/// Run `script` in bash: it asks the terminal something and prints the
/// answer between `got ` and `|`.
fn ask(harness: &mut Harness, script: &str) -> String {
    let pane = harness.spawn(&["bash", "--norc", "--noprofile", "-c", script]);
    assert_eq!(harness.exited(&pane).0, Some(0));
    let text = screen(&pane);
    let answer = text
        .split("got ")
        .nth(1)
        .and_then(|rest| rest.split('|').next());
    answer
        .unwrap_or_else(|| panic!("no answer in {text:?}"))
        .to_owned()
}

#[test]
fn a_cursor_position_query_is_answered() {
    let mut harness = Harness::new();
    let script =
        r"printf '\033[5;9H\033[6n'; IFS= read -rs -d R -t 4 r; printf '\033[Hgot %s|' ${r#??}";
    assert_eq!(ask(&mut harness, script), "5;9");
}

#[test]
fn grapheme_clustering_is_on_in_a_new_pane() {
    let mut harness = Harness::new();
    let script = r"printf '\033[?2027$p'; IFS= read -rs -d y -t 4 r; printf 'got %s|' ${r#???}";
    // DECRPM: mode 2027 is set (1).
    assert_eq!(ask(&mut harness, script), "2027;1$");
}

#[test]
fn a_color_query_gets_the_configured_theme() {
    let mut harness = Harness::new();
    let config = harness.env.config_home().join("shoal");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("config.toml"),
        "[theme.colors]\nbackground = \"#123456\"\n",
    )
    .unwrap();
    let script = r"printf '\033]11;?\033\\'; IFS= read -rs -d '\' -t 4 r; r=${r%?}; printf 'got %s|' ${r#*;}";
    assert_eq!(ask(&mut harness, script), "rgb:1212/3434/5656");
}

#[test]
fn live_settings_change_color_answers_without_replacing_program_colors() {
    let mut harness = Harness::new();
    let script = r"
        stty -echo
        printf 'ready-first'
        IFS= read -r -t 4 step || exit 1
        printf '\033]11;?\033\\'
        IFS= read -rs -d '\' -t 4 r || exit 2
        r=${r%?}; printf '\r\nfirst %s|' ${r#*;}
        printf '\033]11;#445566\033\\ready-second'
        IFS= read -r -t 4 step || exit 3
        printf '\033]11;?\033\\'
        IFS= read -rs -d '\' -t 4 r || exit 4
        r=${r%?}; printf '\r\nkept %s|' ${r#*;}
        printf '\033]111\033\\\033]11;?\033\\'
        IFS= read -rs -d '\' -t 4 r || exit 5
        r=${r%?}; printf '\r\nreset %s|' ${r#*;}
    ";
    let pane = harness.spawn(&["bash", "--norc", "--noprofile", "-c", script]);
    wait_for(&pane, "ready-first");
    let settings = harness.settings();
    let mut theme = settings.theme;
    theme.background = shoal_term::Rgb {
        r: 0x12,
        g: 0x34,
        b: 0x56,
    };
    pane.apply_settings(theme, settings.scrollback, settings.persist);
    pane.write(b"\n".to_vec());
    let text = wait_for(&pane, "ready-second");
    assert!(text.contains("first rgb:1212/3434/5656|"), "{text}");
    theme.background = shoal_term::Rgb {
        r: 0xab,
        g: 0xcd,
        b: 0xef,
    };
    pane.apply_settings(theme, settings.scrollback, settings.persist);
    pane.write(b"\n".to_vec());
    assert_eq!(harness.exited(&pane).0, Some(0));
    let text = screen(&pane);
    assert!(text.contains("kept rgb:4444/5555/6666|"), "{text}");
    assert!(text.contains("reset rgb:abab/cdcd/efef|"), "{text}");
}

#[test]
fn live_settings_limit_future_scrollback() {
    let mut harness = Harness::new();
    let script = r"
        stty -echo
        printf 'ready'
        IFS= read -r -t 4 step || exit 1
        for ((i = 0; i < 100; i++)); do printf '\r\nline-%03d' $i; done
    ";
    let mut spec = harness.spec(&["bash", "--norc", "--noprofile", "-c", script]);
    spec.size = GridSize { cols: 40, rows: 3 };
    let pane = harness.start(spec);
    wait_for(&pane, "ready");
    pane.apply_settings(harness.settings().theme, 0, true);
    pane.write(b"\n".to_vec());
    assert_eq!(harness.exited(&pane).0, Some(0));
    let capture = pane
        .capture(CaptureFormat::Text, true)
        .blocking_recv()
        .unwrap()
        .unwrap();
    let text = String::from_utf8(capture).unwrap();
    assert!(text.contains("line-099"), "{text}");
    assert!(!text.contains("line-000"), "{text}");
    assert!(text.lines().count() <= 3, "{text}");
}

#[test]
fn shell_program_is_used_when_the_pane_has_no_argv() {
    let mut harness = Harness::new();
    let probe = "echo shell=${ZSH_VERSION:+zsh}${BASH_VERSION:+bash}";
    // TempEnv has no SHELL, so without `shell.program` the passwd entry
    // decides; set SHELL to tell the two apart.
    let mut settings = harness.settings();
    settings.base_env.push(("SHELL".into(), "bash".into()));
    let spec = harness.spec(&[]);
    let pane = pane::spawn(spec, &settings, harness.sender.clone()).unwrap();
    type_line(&pane, probe);
    wait_for(&pane, "shell=bash");

    let config = harness.env.config_home().join("shoal");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("config.toml"),
        "[shell]\nprogram = \"zsh\"\nintegration = 'off'\n",
    )
    .unwrap();
    let mut settings = harness.settings();
    settings.base_env.push(("SHELL".into(), "bash".into()));
    let spec = harness.spec(&[]);
    let pane = pane::spawn(spec, &settings, harness.sender.clone()).unwrap();
    type_line(&pane, probe);
    wait_for(&pane, "shell=zsh");
}

#[test]
fn the_required_environment_variables_are_present() {
    let mut harness = Harness::new();
    let names = [
        "TERM",
        "COLORTERM",
        "TERM_PROGRAM",
        "TERM_PROGRAM_VERSION",
        "SHOAL_PANE",
        "SHOAL_SOCKET",
        "WAYLAND_DISPLAY",
        "NOT_ALLOWED",
        "HOME",
    ];
    let script = format!(
        "for v in {}; do printenv \"$v\" | sed \"s/^/$v=/\"; done",
        names.join(" ")
    );
    let mut spec = harness.spec(&["sh", "-c", &script]);
    spec.size = GridSize {
        cols: 200,
        rows: 24,
    };
    spec.window_env = BTreeMap::from([
        ("WAYLAND_DISPLAY".to_owned(), "wayland-7".to_owned()),
        ("NOT_ALLOWED".to_owned(), "x".to_owned()),
    ]);
    let id = spec.id;
    let pane = harness.start(spec);
    harness.exited(&pane);
    let text = screen(&pane);
    let lines: Vec<_> = text.lines().filter(|line| !line.is_empty()).collect();
    let socket = harness.env.socket();
    let home = harness.env.home();
    assert_eq!(
        lines,
        [
            "TERM=xterm-256color".to_owned(),
            "COLORTERM=truecolor".to_owned(),
            "TERM_PROGRAM=shoal".to_owned(),
            "TERM_PROGRAM_VERSION=0.1.0".to_owned(),
            format!("SHOAL_PANE={id}"),
            format!("SHOAL_SOCKET={}", socket.display()),
            "WAYLAND_DISPLAY=wayland-7".to_owned(),
            format!("HOME={}", home.display()),
        ]
    );
}

#[test]
fn a_shell_starts_in_the_requested_directory_or_home() {
    let mut harness = Harness::new();
    let dir = tempfile::tempdir().unwrap();
    let mut spec = harness.spec(&["sh", "-c", "pwd"]);
    spec.size = GridSize {
        cols: 200,
        rows: 24,
    };
    spec.cwd = Some(dir.path().to_owned());
    let pane = harness.start(spec);
    harness.exited(&pane);
    assert!(screen(&pane).contains(&dir.path().display().to_string()));

    let mut spec = harness.spec(&["sh", "-c", "pwd"]);
    spec.size = GridSize {
        cols: 200,
        rows: 24,
    };
    spec.cwd = Some(dir.path().join("gone"));
    let pane = harness.start(spec);
    harness.exited(&pane);
    let home = harness.env.home().display().to_string();
    assert_eq!(screen(&pane).trim(), home);
}

#[test]
fn a_missing_program_fails_to_spawn() {
    let mut harness = Harness::new();
    let spec = harness.spec(&["no-such-program-shoal"]);
    let error = pane::spawn(spec, &harness.settings(), harness.sender.clone()).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("cannot start no-such-program-shoal"),
        "{error}"
    );
}

/// One field of `/proc/self/task/<tid>/stat`, after the parenthesized name.
fn stat_field(stat: &str, field: usize) -> String {
    let rest = &stat[stat.rfind(')').unwrap() + 2..];
    rest.split(' ').nth(field - 3).unwrap().to_owned()
}

#[test]
fn the_pane_thread_sleeps_while_the_shell_is_idle() {
    let mut harness = Harness::new();
    let pane = harness.spawn(&["bash"]);
    wait_for(&pane, "bash$");
    let _attachment = pane
        .attach(std::sync::Arc::new(tokio::sync::Notify::new()))
        .blocking_recv()
        .unwrap()
        .unwrap();
    eventually(
        "initial output metadata and scrollback saving to settle",
        || {
            (pane.metadata_samples() > 0 && !pane.metadata_pending() && !pane.persist_pending())
                .then_some(())
        },
    );
    let name = format!("pane-{}", pane.id().get());
    let task = std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .find(|task| {
            std::fs::read_to_string(task.join("comm"))
                .ok()
                .is_some_and(|comm| comm.trim() == name)
        })
        .expect("a thread named after the pane");
    let sample = || {
        let stat = std::fs::read_to_string(task.join("stat")).unwrap();
        let status = std::fs::read_to_string(task.join("status")).unwrap();
        let switches = status
            .lines()
            .find_map(|line| line.strip_prefix("voluntary_ctxt_switches:"))
            .unwrap()
            .trim()
            .to_owned();
        // Fields 3, 14 and 15: state, user and system ticks.
        (
            stat_field(&stat, 3),
            stat_field(&stat, 14),
            stat_field(&stat, 15),
            switches,
        )
    };
    let before = eventually("the pane thread to sleep", || {
        let sample = sample();
        (sample.0 == "S").then_some(sample)
    });
    let samples = pane.metadata_samples();
    // Measuring, not synchronizing: a quiet pane must not wake at all.
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(pane.metadata_samples(), samples);
    assert_eq!(sample(), before);
}
