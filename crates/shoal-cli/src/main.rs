//! `shoal`, the command-line interface. So far it renders the native prompt
//! (M1 spec §8.3); the other commands in §11 arrive with T21.

mod prompt;

use std::{process::ExitCode, time::Instant};

fn main() -> ExitCode {
    // The prompt's hard cap counts from here.
    let started = Instant::now();
    let mut args = std::env::args_os().skip(1);
    match args.next() {
        Some(command) if command == "prompt" => prompt::main(args, started),
        _ => {
            eprintln!("shoal: unknown command\n{}", prompt::USAGE);
            ExitCode::from(2)
        }
    }
}
