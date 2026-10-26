//! `shoal`, the command-line interface: native prompt and shell integration
//! (M1 spec §8). The remaining commands in §11 arrive with T21.

mod prompt;
mod shell_init;

use std::{process::ExitCode, time::Instant};

fn main() -> ExitCode {
    // The prompt's hard cap counts from here.
    let started = Instant::now();
    let mut args = std::env::args_os().skip(1);
    match args.next() {
        Some(command) if command == "prompt" => prompt::main(args, started),
        Some(command) if command == "shell-init" => shell_init::main(args),
        _ => {
            eprintln!(
                "shoal: unknown command\n{}\n{}",
                prompt::USAGE,
                shell_init::USAGE
            );
            ExitCode::from(2)
        }
    }
}
