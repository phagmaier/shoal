//! A restored pane's old output (M1 spec §6.4 steps 3–5), fed into its own
//! terminal before the shell starts, so the shell's output lands below it.

use std::{cell::RefCell, path::Path};

use shoal_term::{TermError, Terminal};

use super::super::{Restore, metadata::Tracker};

/// Replay, make inert and divide. Old bytes may hold queries or reports, but
/// nothing answers them and the pane's metadata starts fresh.
pub(super) fn replay(
    terminal: &mut Terminal,
    restore: &Restore,
    replies: &RefCell<Vec<u8>>,
    metadata: &RefCell<Tracker>,
    cwd: &Path,
) -> Result<(), TermError> {
    terminal.replay_saved(&restore.saved, &restore.divider)?;
    replies.borrow_mut().clear();
    *metadata.borrow_mut() = Tracker::new(cwd.to_owned());
    Ok(())
}
