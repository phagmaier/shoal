//! A pane's mirror and its attach stream (M1 spec §5.5): decode each
//! snapshot into a fresh terminal within the decoding limits, feed it every
//! numbered item in order, and handle failures: re-attach once, then show a
//! banner with Retry. A parser stuck mid-sequence gets its own banner and no
//! automatic retry.

use shoal_client::{Error, StreamItem, model::GridSize, proto::DetachReason};
use shoal_config::schema::{Config, CursorStyle};
use shoal_term::{CellSize, CursorShape, KeyEvent, MouseEvent, RestoreError, TermError, Terminal};

use crate::draw::{self, DrawList};
mod interaction;

/// What every mirror is set up with, from config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setup {
    pub scrollback: u32,
    /// `cursor.style`: the shape until a program chooses one.
    pub cursor: CursorShape,
    pub blink: bool,
}

impl Setup {
    pub fn from_config(config: &Config) -> Self {
        Self {
            scrollback: config.scrollback.lines,
            cursor: match config.cursor.style {
                CursorStyle::Block => CursorShape::Block,
                CursorStyle::Bar => CursorShape::Bar,
                CursorStyle::Underline => CursorShape::Underline,
            },
            blink: config.cursor.blink,
        }
    }
}

/// A window's copy of a pane's terminal, with its draw-list cache.
#[derive(Debug)]
pub struct Mirror {
    terminal: Terminal,
    builder: draw::Builder,
    fraction: f32,
    new_output: bool,
}

impl Mirror {
    /// Decode a snapshot into a fresh mirror. It has no callbacks, so it
    /// never answers the program; only its drawing settings are set before
    /// the first live byte.
    fn new(bytes: &[u8], size: GridSize, setup: &Setup) -> Result<Self, MirrorError> {
        let mut terminal = Terminal::restore(bytes, size.cols, size.rows, setup.scrollback)?;
        terminal.set_default_cursor(setup.cursor, setup.blink)?;
        terminal.set_overscan(1)?;
        Ok(Self {
            terminal,
            builder: draw::Builder::default(),
            fraction: 0.0,
            new_output: false,
        })
    }

    pub fn draw(&mut self, settings: &draw::Settings) -> Result<DrawList, TermError> {
        if self.terminal.viewport()?.at_bottom {
            self.fraction = 0.0;
            self.new_output = false;
        }
        let mut list = self.builder.build(&mut self.terminal, settings)?;
        list.fraction = self.fraction;
        Ok(list)
    }

    pub fn key(&mut self, event: &KeyEvent<'_>) -> Result<Vec<u8>, TermError> {
        let mut bytes = Vec::new();
        self.terminal.encode_key(event, &mut bytes)?;
        Ok(bytes)
    }

    pub fn paste(&self, text: &str) -> Result<Vec<u8>, TermError> {
        let mut bytes = Vec::new();
        self.terminal.encode_paste(text, &mut bytes)?;
        Ok(bytes)
    }

    pub fn focus(&self, focused: bool) -> Result<Vec<u8>, TermError> {
        let mut bytes = Vec::new();
        self.terminal.encode_focus(focused, &mut bytes)?;
        Ok(bytes)
    }

    pub fn mouse_reporting(&self) -> Result<bool, TermError> {
        self.terminal.mouse_reporting()
    }

    pub fn mouse(&mut self, event: &MouseEvent, cell: CellSize) -> Result<Vec<u8>, TermError> {
        let mut bytes = Vec::new();
        self.terminal.encode_mouse(event, cell, &mut bytes)?;
        Ok(bytes)
    }
}

#[derive(Debug, thiserror::Error)]
enum MirrorError {
    #[error(transparent)]
    Restore(#[from] RestoreError),
    #[error(transparent)]
    Term(#[from] TermError),
}

/// Why a pane shows a banner instead of its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Banner {
    /// The daemon couldn't take a snapshot: the program is inside a long
    /// escape sequence.
    MidSequence,
    /// Attaching or decoding failed twice in a row.
    Failed(String),
}

impl Banner {
    pub fn text(&self) -> String {
        match self {
            Self::MidSequence => "This pane is in the middle of an escape sequence.".to_owned(),
            Self::Failed(reason) => format!("Couldn't show this pane: {reason}"),
        }
    }
}

#[derive(Debug)]
pub enum State {
    /// Waiting for a snapshot; `retried` once a failure already re-attached.
    Attaching {
        retried: bool,
    },
    Live(Box<Mirror>),
    Stopped(Banner),
}

/// What the window does after an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// The pane looks different: request a frame.
    Redraw,
    /// The mirror was dropped: attach again.
    Reattach,
}

impl State {
    pub fn new() -> Self {
        Self::Attaching { retried: false }
    }

    /// Apply the next item of the pane's stream.
    pub fn apply(&mut self, item: StreamItem, setup: &Setup) -> Next {
        match item {
            StreamItem::Snapshot { size, bytes, .. } => match Mirror::new(&bytes, size, setup) {
                Ok(mirror) => {
                    *self = Self::Live(Box::new(mirror));
                    Next::Redraw
                }
                Err(error) => self.fail(error.to_string()),
            },
            StreamItem::Output { bytes, .. } => {
                if let Self::Live(mirror) = self {
                    let scrolled = mirror.terminal.viewport().is_ok_and(|view| !view.at_bottom);
                    mirror.terminal.feed(&bytes);
                    mirror.new_output |= scrolled && !bytes.is_empty();
                }
                Next::Redraw
            }
            StreamItem::Resized { size, .. } => match self {
                Self::Live(mirror) => match mirror.terminal.resize(size.cols, size.rows) {
                    Ok(()) => Next::Redraw,
                    Err(error) => self.fail(error.to_string()),
                },
                _ => Next::Redraw,
            },
            StreamItem::Failed(error) if mid_sequence(&error) => {
                *self = Self::Stopped(Banner::MidSequence);
                Next::Redraw
            }
            StreamItem::Failed(error) => self.fail(error.to_string()),
            StreamItem::Detached(DetachReason::MidSequence) => {
                *self = Self::Stopped(Banner::MidSequence);
                Next::Redraw
            }
        }
    }

    /// Drop the mirror and re-attach once; the second failure in a row
    /// shows a banner.
    fn fail(&mut self, reason: String) -> Next {
        tracing::warn!(%reason, "can't mirror a pane");
        if matches!(self, Self::Attaching { retried: true }) {
            *self = Self::Stopped(Banner::Failed(reason));
            Next::Redraw
        } else {
            *self = Self::Attaching { retried: true };
            Next::Reattach
        }
    }
}

/// `pane.attach` failed with 1002 because the parser never reached ground.
fn mid_sequence(error: &Error) -> bool {
    matches!(error, Error::Rpc(rpc)
        if rpc.code == shoal_client::proto::error_code::INVALID_STATE
            && rpc.data.as_ref().and_then(|data| data["reason"].as_str()) == Some("mid_sequence"))
}

#[cfg(test)]
mod tests {
    use shoal_client::proto::RpcError;

    use super::*;

    const SETUP: Setup = Setup {
        scrollback: 100,
        cursor: CursorShape::Bar,
        blink: false,
    };

    fn snapshot_of(bytes: &[u8]) -> StreamItem {
        let mut pane = Terminal::new(20, 4, 100).unwrap();
        pane.feed(bytes);
        StreamItem::Snapshot {
            seq: 7,
            size: GridSize { cols: 20, rows: 4 },
            bytes: pane.snapshot().unwrap(),
        }
    }

    fn corrupt() -> StreamItem {
        StreamItem::Snapshot {
            seq: 7,
            size: GridSize { cols: 20, rows: 4 },
            bytes: b"not a snapshot".to_vec(),
        }
    }

    fn rpc(code: i32, reason: Option<&str>) -> StreamItem {
        StreamItem::Failed(Error::Rpc(RpcError {
            code,
            message: "failed".into(),
            data: reason.map(|reason| serde_json::json!({ "reason": reason })),
        }))
    }

    fn text(state: &mut State) -> String {
        let State::Live(mirror) = state else {
            panic!("not live: {state:?}");
        };
        let settings = draw::Settings {
            opacity: 1.0,
            cursor: shoal_config::schema::Rgb([0, 0, 0]),
            focused: true,
        };
        let list = mirror.draw(&settings).unwrap();
        list.rows
            .iter()
            .map(|row| {
                row.text
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    #[test]
    fn a_snapshot_then_output_and_resizes_make_a_live_mirror() {
        let mut state = State::new();
        assert_eq!(state.apply(snapshot_of(b"hello"), &SETUP), Next::Redraw);
        assert_eq!(text(&mut state), "hello|||");
        let output = StreamItem::Output {
            seq: 8,
            bytes: b" world\r\nnext".to_vec(),
        };
        assert_eq!(state.apply(output, &SETUP), Next::Redraw);
        assert_eq!(text(&mut state), "hello world|next||");
        let resized = StreamItem::Resized {
            seq: 9,
            size: GridSize { cols: 30, rows: 2 },
        };
        state.apply(resized, &SETUP);
        let State::Live(mirror) = &mut state else {
            panic!()
        };
        assert_eq!(
            (
                mirror.terminal.cols().unwrap(),
                mirror.terminal.rows().unwrap()
            ),
            (30, 2)
        );
        // The configured cursor shape, since the program chose none.
        let cursor = mirror.terminal.frame().unwrap().cursor().unwrap();
        assert_eq!(cursor.shape, CursorShape::Bar);
        // A later snapshot replaces the mirror.
        state.apply(snapshot_of(b"fresh"), &SETUP);
        assert_eq!(text(&mut state), "fresh|||");
    }

    #[test]
    fn a_failure_reattaches_once_and_a_second_shows_a_banner() {
        let mut state = State::new();
        assert_eq!(state.apply(corrupt(), &SETUP), Next::Reattach);
        assert!(matches!(state, State::Attaching { retried: true }));
        assert_eq!(state.apply(corrupt(), &SETUP), Next::Redraw);
        let State::Stopped(Banner::Failed(reason)) = &state else {
            panic!("{state:?}")
        };
        assert!(reason.contains("snapshot"), "{reason}");

        // A success in between resets the count, and a live mirror that
        // fails is dropped and re-attached once.
        let mut state = State::new();
        state.apply(corrupt(), &SETUP);
        state.apply(snapshot_of(b"ok"), &SETUP);
        assert!(matches!(state, State::Live(_)));
        assert_eq!(state.apply(rpc(1005, None), &SETUP), Next::Reattach);
        assert_eq!(state.apply(snapshot_of(b"ok"), &SETUP), Next::Redraw);
        assert_eq!(state.apply(corrupt(), &SETUP), Next::Reattach);
    }

    #[test]
    fn a_pane_mid_sequence_waits_for_retry() {
        for item in [
            rpc(1002, Some("mid_sequence")),
            StreamItem::Detached(DetachReason::MidSequence),
        ] {
            let mut state = State::new();
            assert_eq!(
                state.apply(item, &SETUP),
                Next::Redraw,
                "no automatic retry"
            );
            assert!(matches!(state, State::Stopped(Banner::MidSequence)));
        }
        // Too large is a failure like any other.
        let mut state = State::new();
        assert_eq!(
            state.apply(rpc(1002, Some("too_large")), &SETUP),
            Next::Reattach
        );
        assert_eq!(
            Banner::MidSequence.text(),
            "This pane is in the middle of an escape sequence."
        );
    }

    #[test]
    fn live_input_uses_snapshot_modes_and_subsequent_mode_changes() {
        let mut state = State::new();
        state.apply(snapshot_of(b"\x1b[?2004h\x1b[?1h\x1b[?1004h"), &SETUP);
        let State::Live(mirror) = &mut state else {
            panic!()
        };
        assert_eq!(mirror.paste("a\nb").unwrap(), b"\x1b[200~a\nb\x1b[201~");
        assert_eq!(mirror.focus(true).unwrap(), b"\x1b[I");
        assert_eq!(
            mirror
                .key(&KeyEvent {
                    key: shoal_term::Key::ArrowUp,
                    ..KeyEvent::default()
                })
                .unwrap(),
            b"\x1bOA"
        );
        state.apply(
            StreamItem::Output {
                seq: 8,
                bytes: b"\x1b[?2004l\x1b[?1l\x1b[?1004l".to_vec(),
            },
            &SETUP,
        );
        let State::Live(mirror) = &mut state else {
            panic!()
        };
        assert_eq!(mirror.paste("a\nb").unwrap(), b"a\rb");
        assert_eq!(mirror.focus(false).unwrap(), b"");
        assert_eq!(
            mirror
                .key(&KeyEvent {
                    key: shoal_term::Key::ArrowUp,
                    ..KeyEvent::default()
                })
                .unwrap(),
            b"\x1b[A"
        );
    }
}
