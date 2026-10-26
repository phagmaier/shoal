//! T30: real interactive shells on the pane PTY, with isolated startup files.
use std::{collections::BTreeMap, fs, path::Path};

use shoal_config::XdgPaths;
use shoal_daemon::{
    pane::{self, Item, Pane, PaneSpec},
    settings::Settings,
};
use shoal_model::{GridSize, PaneId};
use shoal_term::CaptureFormat;
use shoal_testkit::{TempEnv, eventually};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

struct Shell {
    // Drop the pane before its files.
    pane: Pane,
    items: UnboundedReceiver<Item>,
    _env: TempEnv,
}

fn stream_text(bytes: &[u8]) -> String {
    // Both ST and BEL are valid OSC terminators.
    String::from_utf8_lossy(bytes).replace("\x1b\\", "\x07")
}

impl Shell {
    fn start(shell: &str, mode: &str, integration: &str, rc: &str, zdotdir: bool) -> Self {
        let env = TempEnv::new();
        fs::create_dir_all(env.config_home().join("shoal")).unwrap();
        fs::write(
            env.config_home().join("shoal/config.toml"),
            format!("[shell]\nintegration = {integration:?}\n[prompt]\nmode = {mode:?}\n"),
        )
        .unwrap();
        let rc_dir = if zdotdir {
            env.home().join("dot files")
        } else {
            env.home()
        };
        fs::create_dir_all(&rc_dir).unwrap();
        if mode == "native" {
            use std::os::unix::fs::PermissionsExt;
            let prompt = env.home().join("prompt-fixture");
            fs::write(
                &prompt,
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HOME/prompt-args\"\nprintf 'native> '\n",
            )
            .unwrap();
            fs::set_permissions(prompt, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::write(
            rc_dir.join(".zshenv"),
            "export ENV_FIXTURE=from-zshenv\nexport ZDOTDIR=\"${ZDOTDIR-$HOME}/after-env\"\n",
        )
        .unwrap();
        let user_rc = if shell == "zsh" {
            rc_dir.join("after-env")
        } else {
            rc_dir.clone()
        };
        fs::create_dir_all(&user_rc).unwrap();
        fs::write(
            user_rc.join(if shell == "bash" { ".bashrc" } else { ".zshrc" }),
            rc,
        )
        .unwrap();
        let mut vars = env.vars();
        if zdotdir {
            vars.push(("ZDOTDIR".into(), rc_dir.into()));
        }
        let paths = XdgPaths::from_lookup(|key| {
            vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
        })
        .unwrap();
        let settings = Settings::load(&paths).pane(&paths, env.socket(), vars);
        let (events, _receiver) = unbounded_channel();
        let pane = pane::spawn(
            PaneSpec {
                id: PaneId::new(1),
                size: GridSize {
                    cols: 120,
                    rows: 30,
                },
                argv: vec![shell.into()],
                cwd: None,
                window_env: BTreeMap::new(),
            },
            &settings,
            events,
        )
        .unwrap();
        let (_, items) = pane.subscribe();
        Self {
            pane,
            items,
            _env: env,
        }
    }

    fn screen(&self) -> String {
        String::from_utf8(
            self.pane
                .capture(CaptureFormat::Text, false)
                .blocking_recv()
                .unwrap()
                .unwrap(),
        )
        .unwrap()
    }

    fn ready(&self, prompt: &str) {
        eventually("fixture prompt", || {
            self.screen().contains(prompt).then_some(())
        });
    }

    fn line(&self, text: &str) {
        self.pane.write(format!("{text}\r").into_bytes());
    }

    fn drain(&mut self) -> Vec<u8> {
        let mut bytes = Vec::new();
        while let Ok(item) = self.items.try_recv() {
            if let Item::Output { bytes: output, .. } = item {
                bytes.extend_from_slice(&output);
            }
        }
        bytes
    }

    fn round(&mut self, command: &str, status: i32) -> Vec<u8> {
        self.drain();
        self.line(command);
        let mut bytes = Vec::new();
        eventually(
            &format!("completion of {command:?} with status {status}"),
            || {
                bytes.extend(self.drain());
                let text = stream_text(&bytes);
                let finished = format!("\x1b]133;D;{status}\x07");
                text.split_once("\x1b]133;C\x07")
                    .and_then(|(_, after)| after.split_once(&finished))
                    .is_some_and(|(_, after)| after.contains("\x1b]133;B\x07"))
                    .then_some(())
            },
        );
        // The startup prompt can still be arriving when the command is sent.
        // Keep only this command's lifecycle, not that earlier prompt.
        let start = bytes
            .windows(7)
            .position(|part| part == b"\x1b]133;C")
            .unwrap();
        bytes.drain(..start);
        bytes
    }
}

fn rc(shell: &str) -> &'static str {
    if shell == "bash" {
        "export RC_FIXTURE=from-rc\nPS1='fixture> '\nPROMPT_COMMAND='HOOK_RAN=yes'\n"
    } else {
        "export RC_FIXTURE=from-rc\nPROMPT='fixture> '\nprecmd() { HOOK_RAN=yes; }\n"
    }
}

#[test]
fn auto_preserves_startup_hooks_and_shell_prompt_and_reports_commands() {
    for shell in ["bash", "zsh"] {
        let mut sh = Shell::start(shell, "shell", "auto", rc(shell), shell == "zsh");
        sh.ready("fixture>");
        let bytes = sh.round(
            "printf 'values:%s:%s\\n' \"$RC_FIXTURE\" \"$HOOK_RAN\"; (exit 7)",
            7,
        );
        let text = stream_text(&bytes);
        assert!(text.contains("values:from-rc:yes"), "{shell}: {text:?}");
        let c = text.find("\x1b]133;C\x07").unwrap();
        let output = text.find("values:from-rc:yes").unwrap();
        let d = text.find("\x1b]133;D;7\x07").unwrap();
        let a = text.find("\x1b]133;A\x07").unwrap();
        let b = text.find("\x1b]133;B\x07").unwrap();
        assert!(
            c < output && output < d && d < a && a < b,
            "{shell}: {text:?}"
        );
        assert!(text.contains("\x1b]7;file://"), "{shell}: {text:?}");
        assert!(text[a..b].contains("fixture> "), "{shell}: {text:?}");
        if shell == "zsh" {
            let bytes = sh.round("printf 'env:%s\\n' \"$ENV_FIXTURE\"", 0);
            assert!(String::from_utf8_lossy(&bytes).contains("env:from-zshenv"));
        }
        // Sourcing twice must not stack prompt wrappers or lifecycle hooks.
        let source = format!(
            ". \"$SHOAL_SHELL_DIR/{shell}/shoal.{}\"; . \"$SHOAL_SHELL_DIR/{shell}/shoal.{}\"",
            if shell == "bash" { "bash" } else { "zsh" },
            if shell == "bash" { "bash" } else { "zsh" }
        );
        sh.round(&source, 0);
        let bytes = sh.round("printf 'again:%s\\n' ok", 0);
        let text = stream_text(&bytes);
        for mark in ["A", "B", "C", "D;0"] {
            assert_eq!(
                text.matches(&format!("\x1b]133;{mark}\x07")).count(),
                1,
                "{shell}: {text:?}"
            );
        }
    }
}

#[test]
fn manual_and_off_do_not_inject_hooks() {
    for integration in ["manual", "off"] {
        for shell in ["bash", "zsh"] {
            let mut sh = Shell::start(shell, "shell", integration, rc(shell), false);
            sh.ready("fixture>");
            sh.drain();
            sh.line("printf 'guard:%s\\n' \"${__shoal_loaded-unloaded}\"");
            eventually("uninjected shell", || {
                sh.screen().contains("guard:unloaded").then_some(())
            });
            assert!(!String::from_utf8_lossy(&sh.drain()).contains("\x1b]133;"));
        }
    }
}

#[test]
fn directory_reports_escape_control_and_uri_characters() {
    for shell in ["bash", "zsh"] {
        let rc = format!(
            "{}HOSTNAME=$'host\\aattack'\nHOST=$'host\\aattack'\n",
            rc(shell)
        );
        let mut sh = Shell::start(shell, "shell", "auto", &rc, false);
        sh.ready("fixture>");
        let dir = sh._env.home().join("space #percent%\x07é");
        fs::create_dir(&dir).unwrap();
        let bytes = sh.round("cd -- \"$HOME/\"$'space #percent%\\aé'", 0);
        let text = stream_text(&bytes);
        assert!(
            text.contains("\x1b]7;file://hostattack/"),
            "{shell}: {text:?}"
        );
        let path = Path::new(&dir).to_string_lossy();
        let safe_parent = path.split("space").next().unwrap();
        assert!(
            text.contains(&format!("{safe_parent}space%20%23percent%25%07%C3%A9\x07")),
            "{shell}: {text:?}"
        );
    }
}

#[test]
fn native_prompt_gets_live_status_duration_jobs_and_size_after_user_hooks() {
    for shell in ["bash", "zsh"] {
        let rc = if shell == "bash" {
            "PS1='user> '\nSHOAL_BIN=\"$HOME/prompt-fixture\"\nPROMPT_COMMAND=('HOOK_STATUS=$?' 'HOOK_RAN=yes; PS1=\"user> \"')\n"
        } else {
            "PROMPT='user> '\nSHOAL_BIN=\"$HOME/prompt-fixture\"\nprecmd() { HOOK_STATUS=$?; HOOK_RAN=yes; PROMPT='user> '; }\n"
        };
        let mut sh = Shell::start(shell, "native", "auto", rc, false);
        sh.ready("native>");
        sh.round("sleep 0.05; (exit 7)", 7);
        let args = fs::read_to_string(sh._env.home().join("prompt-args")).unwrap();
        let args: Vec<_> = args.lines().collect();
        assert_eq!(&args[..5], ["prompt", "--shell", shell, "--status", "7"]);
        assert_eq!(args[5], "--duration-ms");
        let duration: u64 = args[6].parse().unwrap();
        assert!((40..5000).contains(&duration), "{shell}: {args:?}");
        assert_eq!(&args[7..], ["--jobs", "0", "--cols", "120"]);
        let bytes = sh.round(
            "printf 'hook:%s:%s\\n' \"$HOOK_STATUS\" \"$HOOK_RAN\"; sleep 100 &",
            0,
        );
        assert!(String::from_utf8_lossy(&bytes).contains("hook:7:yes"));
        let args = fs::read_to_string(sh._env.home().join("prompt-args")).unwrap();
        assert!(args.contains("--jobs\n1\n"), "{shell}: {args}");
    }
}
