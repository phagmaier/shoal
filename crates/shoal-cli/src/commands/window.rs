//! `shoal` and `shoal attach` (M1 spec §7.1, §11): the daemon must answer
//! first, so failures get the CLI's exit codes; then `shoal-gui` starts
//! detached and chooses its session itself.

use std::{path::PathBuf, process::Command};

use shoal_client::{lifecycle, model::SessionId};

use super::Failure;

/// Start a window on `session`, or on the smart-open choice. The window
/// inherits the current folder, where smart open creates a new session.
pub fn launch(session: Option<SessionId>) -> Result<(), Failure> {
    let gui = gui();
    let mut command = Command::new(&gui);
    if let Some(session) = session {
        command.arg("--session").arg(session.to_string());
    }
    // The window outlives this command; nothing waits for it.
    lifecycle::detach(&mut command)
        .spawn()
        .map(drop)
        .map_err(|error| Failure::Error(format!("can't start {}: {error}", gui.display())))
}

/// The `shoal-gui` next to this binary, else the one on `PATH`.
fn gui() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("shoal-gui")))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| "shoal-gui".into())
}
