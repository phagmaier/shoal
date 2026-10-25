//! Terminal bytes for reboot persistence (M1 spec §6.3).

use libghostty_vt::fmt::{Format, FormatterOptions};

use crate::{
    RestoreError, SnapshotError, Terminal,
    snapshot::{MAX_DECODE_BYTES, restore_within},
};

// `?1049l` alone returns to the primary screen but leaves a 47 or 1047 flag
// set, which a formatter emitting modes would replay as a fresh alternate
// screen. Each flag clears only through its own exit.
const LEAVE_ALTERNATE: &[u8] = b"\x1b[?1049l\x1b[?1047l\x1b[?47l";

/// What the pane thread hands to the blocking worker that writes scrollback.
/// `Send`, unlike [`Terminal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Persisted {
    /// Formatter output for the primary screen and its history.
    Vt(Vec<u8>),
    /// A full-screen program is up: a snapshot whose primary screen still has
    /// to be formatted, off the pane thread, with [`Persisted::into_vt`].
    Snapshot {
        bytes: Vec<u8>,
        cols: u16,
        rows: u16,
    },
}

impl Terminal {
    /// The cheap, pane-thread half of persisting. A snapshot is only needed when
    /// the formatter would see an alternate screen instead of the primary one.
    pub fn persisted(&mut self) -> Result<Persisted, SnapshotError> {
        if !self.alternate_screen()? {
            return Ok(Persisted::Vt(self.format_vt_now()?));
        }
        Ok(Persisted::Snapshot {
            bytes: self.snapshot()?,
            cols: self.cols()?,
            rows: self.rows()?,
        })
    }

    /// The primary screen and its history as terminal bytes, even while an
    /// alternate screen is showing. The live terminal is never changed.
    pub fn format_vt(&mut self) -> Result<Vec<u8>, FormatVtError> {
        self.persisted()?.into_vt()
    }

    fn format_vt_now(&self) -> Result<Vec<u8>, crate::TermError> {
        // Unwrapped lines rewrap where they were when replayed at the same
        // width, so restored history keeps reflowing. No palette: a program's
        // old OSC 4 overrides must not outlive it and mask the theme.
        self.format(
            FormatterOptions::new()
                .with_format(Format::Vt)
                .with_unwrap(true)
                .with_trim(false)
                .with_style(true)
                .with_hyperlink(true),
        )
    }
}

impl Persisted {
    /// Finish on a worker thread: decode a snapshot into a scratch terminal with
    /// no callbacks, leave its alternate screen and format what is beneath.
    pub fn into_vt(self) -> Result<Vec<u8>, FormatVtError> {
        match self {
            Self::Vt(bytes) => Ok(bytes),
            Self::Snapshot { bytes, cols, rows } => {
                let (mut scratch, _) =
                    restore_within(&bytes, cols, rows, u32::MAX, MAX_DECODE_BYTES)?;
                scratch.feed(LEAVE_ALTERNATE);
                Ok(scratch.format_vt_now()?)
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FormatVtError {
    #[error(transparent)]
    Snapshot(#[from] SnapshotError),
    #[error(transparent)]
    Restore(#[from] RestoreError),
    #[error(transparent)]
    Format(#[from] crate::TermError),
}
