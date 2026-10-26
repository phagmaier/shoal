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

// Make replayed output inert (M1 spec §6.4 step 4). CAN aborts any sequence the
// saved bytes left unfinished. After leaving an alternate screen: DECSTR (styles,
// cursor visibility, origin, insert, wrap, cursor keys, keypad, margins,
// charsets, palette), and turn off what DECSTR keeps: reverse video, mouse
// reporting, focus events, bracketed paste, synchronized output, in-band resize,
// newline mode, the Kitty keyboard flags, an open hyperlink and dynamic colors.
const ABORT: &[u8] = b"\x18";
const INERT: &[u8] = b"\x1b[!p\
\x1b[?5;9;1000;1002;1003;1004;1005;1006;1015;1016;2004;2026;2048l\x1b[20l\
\x1b[=0u\x1b]8;;\x1b\\\x1b]110\x1b\\\x1b]111\x1b\\\x1b]112\x1b\\";

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

impl Terminal {
    /// Restore saved output into this fresh terminal (M1 spec §6.4 steps 3–5):
    /// replay it, make it inert, then write `divider` dimmed on the line below
    /// the last text, leaving the cursor at the start of the next line.
    pub fn replay_saved(&mut self, saved: &[u8], divider: &str) -> Result<(), crate::TermError> {
        self.feed(saved);
        self.feed(ABORT);
        // Only when one is showing: on the primary screen `?1049l` would
        // restore a saved cursor and overwrite the old output.
        if self.alternate_screen()? {
            self.feed(LEAVE_ALTERNATE);
        }
        self.feed(INERT);
        // Below the last row with text, wherever the old cursor was left.
        let text = self.capture(crate::CaptureFormat::Text, false)?;
        let used = text.split(|&byte| byte == b'\n').count();
        if text.is_empty() {
            self.feed(b"\x1b[H");
        } else {
            self.feed(format!("\x1b[{used}H\r\n").as_bytes());
        }
        self.feed(b"\x1b[2m");
        // Only printable text: a divider never becomes a control sequence.
        let text: String = divider.chars().filter(|c| !c.is_control()).collect();
        self.feed(text.as_bytes());
        self.feed(b"\x1b[0m\r\n");
        Ok(())
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
