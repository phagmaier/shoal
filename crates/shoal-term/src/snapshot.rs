//! Binary snapshots for attach (M1 spec §5.5) and persistence (§6.3).

use std::io::Read;

use libghostty_vt::snapshot::Decoder;

use crate::{
    TermError, Terminal,
    alloc::{self, Budget, Usage},
    terminal::CONTINUATION_BYTES,
};

/// The decoding limits from M1 spec §5.5.
pub const MAX_SIZE: u16 = 1000;
pub const MAX_HISTORY_ROWS: u64 = 100_000;
pub const MAX_DECODE_BYTES: usize = 256 << 20;
/// Terminal pages are mapped straight from the OS, outside any allocator, so
/// their memory is bounded by the declared cells instead: (history + rows) ×
/// cols over both screens, at the measured 4–8 bytes per cell.
pub const MAX_DECODE_CELLS: u64 = 32 << 20;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// The parser is inside a sequence longer than the continuation cap; a
    /// snapshot is possible again once it reaches ground.
    #[error("the terminal is in the middle of an escape sequence")]
    MidSequence,
    #[error(transparent)]
    Encode(#[from] TermError),
}

#[derive(Debug, thiserror::Error)]
pub enum RestoreError {
    #[error("snapshot size {cols}x{rows} is outside 1..={MAX_SIZE}")]
    Size { cols: u16, rows: u16 },
    #[error("snapshot decoded to {got_cols}x{got_rows}, expected {cols}x{rows}")]
    SizeMismatch {
        cols: u16,
        rows: u16,
        got_cols: u16,
        got_rows: u16,
    },
    #[error("snapshot declares {rows} history rows, more than {MAX_HISTORY_ROWS}")]
    History { rows: u64 },
    #[error("snapshot declares {cells} cells, more than {MAX_DECODE_CELLS}")]
    Cells { cells: u64 },
    #[error("snapshot decoding needed more than {MAX_DECODE_BYTES} bytes of memory")]
    Allocation,
    #[error("snapshot ended after {consumed} of {len} bytes")]
    TrailingBytes { consumed: usize, len: usize },
    #[error("snapshot: {0}")]
    Decode(#[from] TermError),
}

impl Terminal {
    /// Encode the whole terminal state, unfinished input included.
    pub fn snapshot(&mut self) -> Result<Vec<u8>, SnapshotError> {
        let mut bytes = Vec::new();
        match self.inner.encode_snapshot(&mut bytes) {
            Ok(()) => Ok(bytes),
            // Encoding rejects a parser inside a sequence longer than the
            // tracked cap; at ground the same error means something else.
            Err(libghostty_vt::Error::InvalidValue) if !self.is_ground()? => {
                Err(SnapshotError::MidSequence)
            }
            Err(error) => Err(TermError(error).into()),
        }
    }

    /// Decode a snapshot from a daemon with the same [`core_id`](crate::core_id)
    /// into a fresh mirror of `cols` by `rows`, within every decoding limit.
    /// The mirror keeps `scrollback` lines afterwards and never answers queries.
    pub fn restore(
        snapshot: &[u8],
        cols: u16,
        rows: u16,
        scrollback: u32,
    ) -> Result<Self, RestoreError> {
        let (terminal, _) = restore_within(snapshot, cols, rows, scrollback, MAX_DECODE_BYTES)?;
        Ok(terminal)
    }
}

/// A reader that records how much of the snapshot the decoder took, since a
/// complete decode leaves trailing bytes unread without complaint.
struct Counted<'a> {
    bytes: &'a [u8],
    taken: usize,
}

impl Read for Counted<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = (&self.bytes[self.taken..]).read(buf)?;
        self.taken += n;
        Ok(n)
    }
}

fn check_declared(
    primary: u64,
    alternate: Option<u64>,
    cols: u16,
    rows: u16,
) -> Result<(), RestoreError> {
    let screens = [Some(primary), alternate];
    if let Some(rows) = screens
        .into_iter()
        .flatten()
        .find(|&n| n > MAX_HISTORY_ROWS)
    {
        return Err(RestoreError::History { rows });
    }
    let cells = screens
        .into_iter()
        .flatten()
        .map(|history| (history + u64::from(rows)) * u64::from(cols))
        .sum();
    if cells > MAX_DECODE_CELLS {
        return Err(RestoreError::Cells { cells });
    }
    Ok(())
}

pub(crate) fn restore_within(
    snapshot: &[u8],
    cols: u16,
    rows: u16,
    scrollback: u32,
    cap: usize,
) -> Result<(Terminal, Usage), RestoreError> {
    let in_range = |n| (1..=MAX_SIZE).contains(&n);
    if !in_range(cols) || !in_range(rows) {
        return Err(RestoreError::Size { cols, rows });
    }
    let budget = Budget::start(cap);
    let refused = |error: libghostty_vt::Error| {
        if budget.usage().refused {
            RestoreError::Allocation
        } else {
            TermError(error).into()
        }
    };
    let mut reader = Counted {
        bytes: snapshot,
        taken: 0,
    };
    let mut decoder = Decoder::new_with_alloc(alloc::allocator(), &mut reader).map_err(refused)?;
    decoder
        .set_max_continuation_bytes(CONTINUATION_BYTES)
        .map_err(refused)?;
    let mut incremental = decoder.ready().map_err(refused)?;
    let mut checked = false;
    while let Some(progress) = incremental.next().map_err(refused)? {
        if !checked {
            // Declared history becomes readable once READY validates; check it
            // at the first page, before decoding the rest.
            let decoder = progress.as_decoder();
            let primary = decoder.history_rows_primary().map_err(refused)?;
            let alternate = decoder.history_rows_alternate().map_err(refused)?;
            check_declared(primary, alternate, cols, rows)?;
            checked = true;
        }
    }
    let inner = incremental.into_terminal();
    let usage = budget.usage();
    drop(budget);
    let len = snapshot.len();
    if reader.taken != len {
        return Err(RestoreError::TrailingBytes {
            consumed: reader.taken,
            len,
        });
    }
    let mut terminal = Terminal::from_inner(inner);
    let (got_cols, got_rows) = (terminal.cols()?, terminal.rows()?);
    if (got_cols, got_rows) != (cols, rows) {
        return Err(RestoreError::SizeMismatch {
            cols,
            rows,
            got_cols,
            got_rows,
        });
    }
    terminal.set_scrollback(scrollback)?;
    Ok((terminal, usage))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The T00 benchmark shape: 200 × 60 with 10,000 styled lines of history.
    fn large_snapshot() -> Vec<u8> {
        let mut terminal = Terminal::new(200, 60, 10_000).unwrap();
        for i in 0..10_000 {
            let line = format!(
                "\x1b[3{}m{i:05} \x1b[1mbold\x1b[0m {} \x1b[4munder\x1b[0m\r\n",
                i % 8,
                "x".repeat(i % 150)
            );
            terminal.feed(line.as_bytes());
        }
        terminal.snapshot().unwrap()
    }

    #[test]
    fn decoding_under_a_cap_it_needs_fails_cleanly_and_disarms_the_budget() {
        let bytes = large_snapshot();
        // Most decode memory is page memory outside the allocator; what the
        // allocator does see still fails the decode once over its cap.
        let result = restore_within(&bytes, 200, 60, 10_000, 16 << 10);
        assert!(
            matches!(result, Err(RestoreError::Allocation)),
            "{result:?}"
        );
        // The cap lifted with the failed decode: this thread allocates freely.
        let (mut mirror, _) = restore_within(&bytes, 200, 60, 10_000, MAX_DECODE_BYTES).unwrap();
        // A mirror keeps its counting allocator, but no cap applies after FINISH.
        for _ in 0..2000 {
            mirror.feed(&[b'y'; 400]);
        }
        assert!(mirror.scrollback_rows().unwrap() > 9_000);
    }

    #[test]
    fn allocator_visible_decode_memory_for_200_by_60_with_10000_lines() {
        let bytes = large_snapshot();
        let (terminal, usage) = restore_within(&bytes, 200, 60, 10_000, MAX_DECODE_BYTES).unwrap();
        assert!(terminal.scrollback_rows().unwrap() >= 9_900);
        assert!(!usage.refused);
        // Page memory is measured separately, in tests/memory.rs.
        println!("allocator-visible decode peak: {} bytes", usage.peak);
        assert!(usage.peak > 0 && usage.peak < MAX_DECODE_BYTES);
    }

    #[test]
    fn declared_cells_bound_both_screens() {
        // The default 10,000-line scrollback fits at the widest allowed size.
        assert!(check_declared(10_000, Some(0), 1000, 1000).is_ok());
        // 100,000 lines fit up to 320 columns.
        assert!(check_declared(100_000, Some(0), 320, 60).is_ok());
        assert!(matches!(
            check_declared(100_000, None, 1000, 60),
            Err(RestoreError::Cells { cells: 100_060_000 })
        ));
        // The alternate screen counts too.
        assert!(check_declared(60_000, None, 500, 50).is_ok());
        assert!(matches!(
            check_declared(60_000, Some(20_000), 500, 50),
            Err(RestoreError::Cells { cells: 40_050_000 })
        ));
        assert!(matches!(
            check_declared(100_001, None, 1, 1),
            Err(RestoreError::History { rows: 100_001 })
        ));
        assert!(matches!(
            check_declared(0, Some(100_001), 1, 1),
            Err(RestoreError::History { rows: 100_001 })
        ));
    }
}
