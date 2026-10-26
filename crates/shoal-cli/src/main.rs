//! `shoal`, the command-line interface (M1 spec §11): opening windows,
//! workspace commands, the native prompt and shell integration (§8).

mod commands;
mod prompt;
mod shell_init;

use std::{process::ExitCode, time::Instant};

fn main() -> ExitCode {
    // The prompt's hard cap counts from here.
    let started = Instant::now();
    let mut args = std::env::args_os().skip(1);
    match args.next() {
        None => commands::main(commands::OPEN, args),
        Some(command) if command == "prompt" => prompt::main(args, started),
        Some(command) if command == "shell-init" => shell_init::main(args),
        Some(command) if commands::COMMANDS.iter().any(|name| command == *name) => {
            let command = command.to_string_lossy().into_owned();
            commands::main(&command, args)
        }
        _ => {
            eprintln!(
                "shoal: unknown command\n{}\n{}\n{}",
                commands::USAGE,
                prompt::USAGE,
                shell_init::USAGE
            );
            ExitCode::from(2)
        }
    }
}
