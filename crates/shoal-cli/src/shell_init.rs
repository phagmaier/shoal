//! `shoal shell-init bash|zsh`: shell-quoted manual installation.
use shoal_config::{
    Config, XdgPaths,
    schema::{PromptMode, ShellIntegration},
    shell::ShellPaths,
};
use std::{
    ffi::OsString,
    io::{self, Write},
    process::ExitCode,
};

pub const USAGE: &str = "usage: shoal shell-init bash|zsh";

pub fn main(args: impl IntoIterator<Item = OsString>) -> ExitCode {
    let mut args = args.into_iter();
    let shell = match (args.next(), args.next()) {
        (Some(shell), None) if shell == "bash" => "bash",
        (Some(shell), None) if shell == "zsh" => "zsh",
        _ => {
            eprintln!("shoal shell-init: expected bash or zsh\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match script(shell) {
        Ok(script) => {
            let _ = writeln!(io::stdout().lock(), "{script}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("shoal shell-init: {error}");
            ExitCode::FAILURE
        }
    }
}

fn script(shell: &str) -> Result<String, Box<dyn std::error::Error>> {
    let paths = XdgPaths::from_env()?;
    let config = Config::load(&paths.config_file())
        .map(|loaded| loaded.config)
        .unwrap_or_default();
    if config.shell.integration == ShellIntegration::Off {
        return Ok(String::new());
    }
    let assets = ShellPaths::discover()?;
    let source = assets.root.join(shell).join(format!("shoal.{shell}"));
    if !source.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "shell integration script is missing",
        )
        .into());
    }
    let quote = |value: &std::path::Path| -> Result<String, io::Error> {
        let value = value.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "shell integration path is not UTF-8",
            )
        })?;
        Ok(format!("'{}'", value.replace('\'', "'\\''")))
    };
    let mode = match config.prompt.mode {
        PromptMode::Native => "native",
        PromptMode::Shell => "shell",
    };
    Ok(format!(
        "export SHOAL_PROMPT_MODE={mode}\nexport SHOAL_BIN={}\nexport SHOAL_SHELL_DIR={}\n. {}",
        quote(&assets.binary)?,
        quote(&assets.root)?,
        quote(&source)?
    ))
}
