//! `shoald`, the per-user daemon that owns every shell, pane and layout.

mod connection;
mod log;
mod server;
mod startup;

use std::process::ExitCode;

use shoal_config::XdgPaths;
use startup::Start;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "shoald failed");
            eprintln!("shoald: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let paths = XdgPaths::from_env()?;
    log::init(&paths.logs_dir()).map_err(|error| {
        format!(
            "cannot open the log in {}: {error}",
            paths.logs_dir().display()
        )
    })?;
    let bound = match startup::start(&paths)? {
        Start::Bound(bound) => bound,
        Start::AlreadyRunning => {
            tracing::info!("already running");
            return Ok(());
        }
    };
    let daemon = connection::Daemon {
        version: env!("CARGO_PKG_VERSION"),
        core: shoal_term::core_id(),
        pid: std::process::id(),
    };
    tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .build()?
        .block_on(server::serve(bound, daemon))?;
    Ok(())
}
