//! `shoal prompt` (M1 spec §8.3): one prompt line for bash or zsh, drawn in the
//! window's theme.
//!
//! `git status` and runtime version commands start first and run while the
//! config and theme load. Each gets until the hard cap; one still running then
//! shows `?`.

mod args;
mod command;
mod git;
mod path;
mod render;
mod runtime;

use std::{
    io::Write,
    path::PathBuf,
    process::ExitCode,
    time::{Duration, Instant},
};

use shoal_config::{
    Config, LoadedTheme, Palette, ThemeSource, XdgPaths,
    schema::{PromptSegment, Rgb},
};

use args::Args;
use git::{Git, Head};
use render::{Segment, Tail};
use runtime::Runtime;

pub const USAGE: &str = "usage: shoal prompt --shell bash|zsh --status <exit> \
                         --duration-ms <n> --jobs <n> --cols <COLUMNS>";

/// From the start of `main`, leaving time to print and exit inside 100 ms.
const SEGMENT_DEADLINE: Duration = Duration::from_millis(95);
const BRANCH_ICON: char = '\u{e0a0}';

pub fn main(args: impl IntoIterator<Item = std::ffi::OsString>, started: Instant) -> ExitCode {
    let args = match Args::parse(args) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("shoal prompt: {error}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let line = prompt(&args, started + SEGMENT_DEADLINE);
    // A shell that stopped reading has no use for the prompt; nothing to report.
    let _ = writeln!(std::io::stdout().lock(), "{line}");
    ExitCode::SUCCESS
}

fn prompt(args: &Args, deadline: Instant) -> String {
    let paths = XdgPaths::from_env().ok();
    let cwd = std::env::current_dir().ok();
    let config = paths
        .as_ref()
        .and_then(|paths| Config::load(&paths.config_file()).ok())
        .map(|loaded| loaded.config)
        .unwrap_or_default();
    let wanted = |segment| config.prompt.segments.contains(&segment);
    let git = cwd
        .as_deref()
        .filter(|_| wanted(PromptSegment::Git))
        .and_then(git::start);
    let runtime = cwd
        .as_deref()
        .filter(|_| wanted(PromptSegment::Runtime))
        .and_then(|cwd| {
            let path = std::env::var_os("PATH");
            runtime::start(
                cwd,
                path.as_deref(),
                paths.as_ref().map(XdgPaths::runtimes_file),
            )
        });

    let palette = match &paths {
        Some(paths) => ThemeSource::from_config(&config.theme, paths).load(),
        None => LoadedTheme::default(),
    }
    .palette(&config);
    let colors = Colors::new(&palette);
    let mut git = git.map(|git| git.finish(deadline));
    let mut runtime = runtime.map(|runtime| runtime.finish(deadline));

    let segments: Vec<_> = config
        .prompt
        .segments
        .iter()
        .filter_map(|segment| match segment {
            PromptSegment::User => user_segment(&colors),
            PromptSegment::Path => Some(path_segment(cwd.clone(), &colors)),
            PromptSegment::Git => git.take().map(|git| git_segment(git, &colors)),
            PromptSegment::Runtime => runtime
                .take()
                .map(|runtime| runtime_segment(runtime, &colors)),
        })
        .collect();
    let tail = Tail {
        took: args.duration,
        jobs: args.jobs,
        took_color: colors.yellow,
        jobs_color: colors.cyan,
        character_color: if args.status == 0 {
            colors.accent
        } else {
            colors.red
        },
    };
    render::render(
        args.shell,
        config.prompt.style,
        &segments,
        colors.background,
        &tail,
    )
}

/// The palette entries the prompt uses (M1 spec §8.3 and §9.2).
struct Colors {
    background: Rgb,
    accent: Rgb,
    red: Rgb,
    green: Rgb,
    yellow: Rgb,
    blue: Rgb,
    magenta: Rgb,
    cyan: Rgb,
}

impl Colors {
    fn new(palette: &Palette) -> Self {
        let [_, red, green, yellow, blue, magenta, cyan, ..] = palette.ansi;
        Self {
            background: palette.background,
            accent: palette.accent,
            red,
            green,
            yellow,
            blue,
            magenta,
            cyan,
        }
    }
}

/// `$USER`, with `@host` over SSH; hidden without a user name.
fn user_segment(colors: &Colors) -> Option<Segment> {
    let user = std::env::var("USER").ok().filter(|user| !user.is_empty())?;
    let ssh = std::env::var_os("SSH_CONNECTION").is_some_and(|value| !value.is_empty());
    let host = ssh
        .then(|| std::fs::read_to_string("/proc/sys/kernel/hostname").ok())
        .flatten();
    let text = match host {
        Some(host) => format!("{user}@{}", host.trim_end()),
        None => user,
    };
    Some(Segment {
        color: colors.magenta,
        text,
    })
}

/// `$PWD` as the shell knows it, keeping symlinked names, else the real directory.
fn path_segment(cwd: Option<PathBuf>, colors: &Colors) -> Segment {
    let pwd = std::env::var_os("PWD")
        .map(PathBuf::from)
        .filter(|pwd| pwd.is_absolute())
        .or(cwd)
        .unwrap_or_else(|| PathBuf::from("?"));
    let home = std::env::var("HOME").ok();
    Segment {
        color: colors.blue,
        text: path::display(&pwd.to_string_lossy(), home.as_deref()),
    }
}

fn git_segment(git: Git, colors: &Colors) -> Segment {
    let head = |head: &Head| match head {
        Head::Branch(name) | Head::Detached(name) => format!(" {name}"),
    };
    let mut text = BRANCH_ICON.to_string();
    let mut color = colors.green;
    match git {
        Git::Known(status) => {
            text.push_str(&head(&status.head));
            if status.dirty {
                text.push_str(" ●");
                color = colors.yellow;
            }
            if status.ahead > 0 {
                text.push_str(&format!(" ⇡{}", status.ahead));
            }
            if status.behind > 0 {
                text.push_str(&format!(" ⇣{}", status.behind));
            }
        }
        Git::Unknown(known) => {
            text.push_str(&known.as_ref().map(head).unwrap_or_default());
            text.push_str(" ?");
        }
    }
    Segment { color, text }
}

fn runtime_segment(runtime: Runtime, colors: &Colors) -> Segment {
    Segment {
        color: colors.cyan,
        text: format!(
            "{} {}",
            runtime.language.icon(),
            runtime.version.as_deref().unwrap_or("?")
        ),
    }
}
