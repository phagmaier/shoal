//! `new`, `ls`, `kill`, `send` and `capture` (M1 spec §11), through
//! `shoal-client`. Exit codes: 0 success, 1 error, 2 bad usage, 3 daemon
//! unreachable. `shoal` and `shoal attach` open windows (T40); `shoal daemon`
//! and auto-start come with T22, so until then a stopped daemon is exit 3.

mod ls;
mod target;

use std::{
    ffi::OsString,
    io::{self, Write as _},
    path::PathBuf,
    process::ExitCode,
};

use base64::Engine as _;
use serde_json::Value;
use shoal_client::{
    Client, Error, NoAutoStart, Options,
    model::{PaneId, SessionId},
    proto::{Call, CaptureFormat, ClientKind, Empty, WorkspaceState},
};
use target::Target;

pub const COMMANDS: [&str; 5] = ["new", "ls", "kill", "send", "capture"];

pub const USAGE: &str = "\
usage: shoal new [-n NAME] [--cwd DIR]
       shoal ls [--json]
       shoal kill <target>
       shoal send <target> <text>      (\\n presses Enter)
       shoal capture <target> [--scrollback] [--vt]
targets: an id (s3, t12, p40), a session name, or . for this pane";

#[derive(Debug, PartialEq, Eq)]
enum Command {
    New {
        name: Option<String>,
        cwd: Option<PathBuf>,
    },
    Ls {
        json: bool,
    },
    Kill {
        target: String,
    },
    Send {
        target: String,
        text: String,
    },
    Capture {
        target: String,
        scrollback: bool,
        vt: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum Failure {
    Usage(String),
    Error(String),
    Unreachable(String),
}

pub fn main(command: &str, args: impl IntoIterator<Item = OsString>) -> ExitCode {
    match parse(command, args).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            eprintln!("shoal {command}: {message}\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Failure::Error(message)) => {
            eprintln!("shoal: {message}");
            ExitCode::FAILURE
        }
        Err(Failure::Unreachable(message)) => {
            eprintln!("shoal: can't reach shoald: {message}");
            ExitCode::from(3)
        }
    }
}

fn parse(command: &str, args: impl IntoIterator<Item = OsString>) -> Result<Command, Failure> {
    let usage = |message: String| Failure::Usage(message);
    let mut words = Vec::new();
    let (mut name, mut cwd, mut flags) = (None, None, Vec::new());
    let mut args = args.into_iter();
    let mut options = true;
    while let Some(arg) = args.next() {
        let arg = arg
            .into_string()
            .map_err(|_| usage("arguments must be UTF-8".into()))?;
        if !options || !arg.starts_with('-') || arg == "-" {
            words.push(arg);
            continue;
        }
        let (option, inline) = match arg.split_once('=') {
            Some((option, value)) => (option.to_owned(), Some(value.to_owned())),
            None => (arg, None),
        };
        let mut value = || match inline.clone() {
            Some(value) => Ok(value),
            None => args
                .next()
                .and_then(|value| value.into_string().ok())
                .ok_or_else(|| usage(format!("{option} needs a value"))),
        };
        match (command, option.as_str()) {
            (_, "--") => options = false,
            ("new", "-n" | "--name") => name = Some(value()?),
            ("new", "--cwd") => cwd = Some(PathBuf::from(value()?)),
            ("ls", "--json") | ("capture", "--scrollback" | "--vt") if inline.is_none() => {
                flags.push(option);
            }
            _ => return Err(usage(format!("unknown option {option}"))),
        }
    }
    let flag = |name: &str| flags.iter().any(|flag| flag == name);
    let mut words = words.into_iter();
    let parsed = match command {
        "new" => Command::New { name, cwd },
        "ls" => Command::Ls {
            json: flag("--json"),
        },
        "kill" => Command::Kill {
            target: words.next().ok_or_else(|| usage("which target?".into()))?,
        },
        "send" => Command::Send {
            target: words.next().ok_or_else(|| usage("which pane?".into()))?,
            text: words.next().ok_or_else(|| usage("what text?".into()))?,
        },
        "capture" => Command::Capture {
            target: words.next().ok_or_else(|| usage("which pane?".into()))?,
            scrollback: flag("--scrollback"),
            vt: flag("--vt"),
        },
        _ => return Err(usage("unknown command".into())),
    };
    match words.next() {
        Some(extra) => Err(usage(format!("unexpected argument {extra:?}"))),
        None => Ok(parsed),
    }
}

fn run(command: Command) -> Result<(), Failure> {
    let client = connect()?;
    match command {
        Command::New { name, cwd } => {
            let cwd = cwd.map(absolute).transpose()?;
            let result = request(&client, Call::SessionCreate { name, cwd })?;
            let session: SessionId = field(&result, "session")?;
            print(format!("{session}\n").as_bytes())
        }
        Command::Ls { json } => {
            let result = request(&client, Call::WorkspaceGet(Empty {}))?;
            if json {
                let text = serde_json::to_string_pretty(&result).map_err(internal)?;
                return print(format!("{text}\n").as_bytes());
            }
            let state: WorkspaceState = serde_json::from_value(result).map_err(internal)?;
            let home = std::env::var_os("HOME").map(PathBuf::from);
            print(ls::render(&state.workspace, home.as_deref()).as_bytes())
        }
        Command::Kill { target } => {
            let call = match resolve(&client, &target)?.0 {
                Target::Session(session) => Call::SessionClose { session },
                Target::Tab(tab) => Call::TabClose { tab },
                Target::Pane(pane) => Call::PaneClose { pane },
            };
            request(&client, call).map(drop)
        }
        Command::Send { target, text } => {
            let pane = resolve_pane(&client, &target)?;
            let text = keys(&text);
            request(&client, Call::PaneSendText { pane, text }).map(drop)
        }
        Command::Capture {
            target,
            scrollback,
            vt,
        } => {
            let pane = resolve_pane(&client, &target)?;
            let format = if vt {
                CaptureFormat::Vt
            } else {
                CaptureFormat::Text
            };
            let result = request(
                &client,
                Call::PaneCapture {
                    pane,
                    format,
                    scrollback,
                },
            )?;
            let data: String = field(&result, "data")?;
            if vt {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(internal)?;
                print(&bytes)
            } else if data.is_empty() || data.ends_with('\n') {
                print(data.as_bytes())
            } else {
                print(format!("{data}\n").as_bytes())
            }
        }
    }
}

/// `\n` in the text, or a real line break, presses Enter (a carriage return,
/// as the key sends); `\\` is a backslash. Other text is typed as it is.
fn keys(text: &str) -> String {
    let mut keys = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => keys.push('\r'),
                Some('\\') => keys.push('\\'),
                Some(other) => {
                    keys.push('\\');
                    keys.push(other);
                }
                None => keys.push('\\'),
            },
            '\n' => keys.push('\r'),
            c => keys.push(c),
        }
    }
    keys
}

fn connect() -> Result<Client, Failure> {
    let paths = shoal_config::XdgPaths::from_env().map_err(|e| Failure::Error(e.to_string()))?;
    let socket = paths
        .socket_file()
        .map_err(|e| Failure::Error(e.to_string()))?;
    let mut options = Options::new(socket, ClientKind::Cli);
    options.reconnect = false;
    Client::connect(options, NoAutoStart)
        .map(|connected| connected.client)
        .map_err(failure)
}

fn request(client: &Client, call: Call) -> Result<Value, Failure> {
    client.request(call).map_err(failure)
}

fn failure(error: Error) -> Failure {
    match error {
        Error::Rpc(error) => Failure::Error(error.message),
        Error::Protocol(_) => Failure::Error(error.to_string()),
        Error::Connect(reason) => Failure::Unreachable(reason),
        Error::Timeout(_) | Error::NotConnected | Error::Stopped => {
            Failure::Unreachable(error.to_string())
        }
    }
}

fn resolve(client: &Client, name: &str) -> Result<(Target, WorkspaceState), Failure> {
    let result = request(client, Call::WorkspaceGet(Empty {}))?;
    let state: WorkspaceState = serde_json::from_value(result).map_err(internal)?;
    let here = std::env::var("SHOAL_PANE").ok();
    let target =
        target::resolve(&state.workspace, name, here.as_deref()).map_err(Failure::Error)?;
    Ok((target, state))
}

fn resolve_pane(client: &Client, name: &str) -> Result<PaneId, Failure> {
    let (target, state) = resolve(client, name)?;
    Ok(target::pane(&state.workspace, target))
}

fn absolute(cwd: PathBuf) -> Result<PathBuf, Failure> {
    let cwd = std::path::absolute(&cwd).map_err(|e| Failure::Error(e.to_string()))?;
    if cwd.is_dir() {
        Ok(cwd)
    } else {
        Err(Failure::Error(format!("{} isn't a folder", cwd.display())))
    }
}

fn field<T: serde::de::DeserializeOwned>(result: &Value, name: &str) -> Result<T, Failure> {
    serde_json::from_value(result[name].clone()).map_err(internal)
}

fn internal(error: impl std::fmt::Display) -> Failure {
    Failure::Error(format!("unexpected answer from shoald: {error}"))
}

fn print(bytes: &[u8]) -> Result<(), Failure> {
    let mut stdout = io::stdout().lock();
    match stdout.write_all(bytes).and_then(|()| stdout.flush()) {
        // A closed pipe (`shoal ls | head`) is not an error worth reporting.
        Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
            Err(Failure::Error(error.to_string()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(words: &[&str]) -> Result<Command, Failure> {
        super::parse(words[0], words[1..].iter().map(OsString::from))
    }

    fn usage(message: &str) -> Result<Command, Failure> {
        Err(Failure::Usage(message.into()))
    }

    #[test]
    fn commands_parse_options_anywhere() {
        assert_eq!(
            parse(&["new", "--cwd", "code", "-n", "work"]),
            Ok(Command::New {
                name: Some("work".into()),
                cwd: Some("code".into())
            })
        );
        assert_eq!(
            parse(&["new", "--name=work"]),
            Ok(Command::New {
                name: Some("work".into()),
                cwd: None
            })
        );
        assert_eq!(parse(&["ls", "--json"]), Ok(Command::Ls { json: true }));
        assert_eq!(
            parse(&["capture", "--vt", "p3", "--scrollback"]),
            Ok(Command::Capture {
                target: "p3".into(),
                scrollback: true,
                vt: true
            })
        );
        assert_eq!(
            parse(&["send", ".", "--", "-n is text"]),
            Ok(Command::Send {
                target: ".".into(),
                text: "-n is text".into()
            })
        );
        assert_eq!(
            parse(&["kill", "main"]),
            Ok(Command::Kill {
                target: "main".into()
            })
        );
    }

    #[test]
    fn bad_usage_says_what_is_wrong() {
        assert_eq!(parse(&["kill"]), usage("which target?"));
        assert_eq!(parse(&["send", "p3"]), usage("what text?"));
        assert_eq!(
            parse(&["ls", "extra"]),
            usage("unexpected argument \"extra\"")
        );
        assert_eq!(parse(&["ls", "--vt"]), usage("unknown option --vt"));
        assert_eq!(parse(&["ls", "--json=yes"]), usage("unknown option --json"));
        assert_eq!(parse(&["new", "-n"]), usage("-n needs a value"));
    }

    #[test]
    fn escapes_press_enter_and_type_backslashes() {
        assert_eq!(keys("ls -la\\n"), "ls -la\r");
        assert_eq!(keys("one\ntwo"), "one\rtwo");
        assert_eq!(keys("a\\\\nb"), "a\\nb");
        assert_eq!(keys("tab\\t and \\"), "tab\\t and \\");
    }
}
