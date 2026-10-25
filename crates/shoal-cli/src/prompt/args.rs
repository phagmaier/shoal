use std::{ffi::OsString, str::FromStr, time::Duration};

use super::render::Shell;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub shell: Shell,
    /// The last command's exit status.
    pub status: i32,
    pub duration: Duration,
    pub jobs: u32,
}

impl Args {
    /// `--name value` or `--name=value`; every option but `--shell` defaults to 0.
    /// `--cols` is validated and ignored: M1 prompts do not change with width.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut args = args.into_iter();
        let (mut shell, mut status, mut duration_ms, mut jobs) = (None, 0, 0, 0);
        while let Some(arg) = args.next() {
            let arg = arg
                .into_string()
                .map_err(|_| "arguments must be UTF-8".to_owned())?;
            let (name, inline) = match arg.split_once('=') {
                Some((name, value)) => (name.to_owned(), Some(value.to_owned())),
                None => (arg, None),
            };
            let mut value = || match inline.clone() {
                Some(value) => Ok(value),
                None => args
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .ok_or_else(|| format!("{name} needs a value")),
            };
            match name.as_str() {
                "--shell" => {
                    shell = Some(match value()?.as_str() {
                        "bash" => Shell::Bash,
                        "zsh" => Shell::Zsh,
                        _ => return Err("--shell must be bash or zsh".into()),
                    })
                }
                "--status" => status = number(&name, &value()?)?,
                "--duration-ms" => duration_ms = number(&name, &value()?)?,
                "--jobs" => jobs = number(&name, &value()?)?,
                "--cols" => _ = number::<u16>(&name, &value()?)?,
                _ => return Err(format!("unknown option {name}")),
            }
        }
        Ok(Self {
            shell: shell.ok_or("--shell is required")?,
            status,
            duration: Duration::from_millis(duration_ms),
            jobs,
        })
    }
}

fn number<T: FromStr>(name: &str, value: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{name} needs a whole number, not {value:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn reads_every_option_in_both_spellings() {
        let args = parse(&[
            "--shell",
            "zsh",
            "--status=127",
            "--duration-ms",
            "2500",
            "--jobs=2",
            "--cols",
            "80",
        ]);
        assert_eq!(
            args,
            Ok(Args {
                shell: Shell::Zsh,
                status: 127,
                duration: Duration::from_millis(2500),
                jobs: 2,
            })
        );
    }

    #[test]
    fn only_shell_is_required() {
        assert_eq!(
            parse(&["--shell=bash"]),
            Ok(Args {
                shell: Shell::Bash,
                status: 0,
                duration: Duration::ZERO,
                jobs: 0,
            })
        );
        assert_eq!(parse(&[]), Err("--shell is required".into()));
    }

    #[test]
    fn rejects_bad_usage() {
        for (args, error) in [
            (&["--shell", "fish"][..], "--shell must be bash or zsh"),
            (&["--shell"], "--shell needs a value"),
            (
                &["--shell=bash", "--jobs", "-1"],
                "--jobs needs a whole number, not \"-1\"",
            ),
            (
                &["--shell=bash", "--cols=wide"],
                "--cols needs a whole number, not \"wide\"",
            ),
            (&["--shell=bash", "--color"], "unknown option --color"),
        ] {
            assert_eq!(parse(args), Err(error.into()), "{args:?}");
        }
    }
}
