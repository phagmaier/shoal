//! Length-prefixed framing. Length prefixes alone never trigger allocation.

use serde_json::{Map, Value};
use shoal_model::{GridSize, PaneId};

/// Maximum declared length (kind + payload), excluding the four-byte prefix.
pub const MAX_FRAME_LEN: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Frame<'a> {
    /// A raw object, deliberately independent of JSON-RPC envelope validation.
    Json(Map<String, Value>),
    Output(Output<'a>),
    Snapshot(Snapshot<'a>),
    Resized(Resized),
    Input(Input<'a>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Output<'a> {
    pub pane: PaneId,
    pub seq: u64,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot<'a> {
    pub pane: PaneId,
    pub seq: u64,
    pub size: GridSize,
    /// Opaque libghostty snapshot, decoded only by shoal-term.
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resized {
    pub pane: PaneId,
    pub seq: u64,
    pub size: GridSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Input<'a> {
    pub pane: PaneId,
    pub bytes: &'a [u8],
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame exceeds the 64 MiB limit")]
    TooLarge,
    #[error("unknown frame kind {0:#04x}")]
    UnknownKind(u8),
    #[error("invalid frame length")]
    InvalidLength,
    #[error("truncated frame")]
    Truncated,
    #[error("bytes follow the frame")]
    TrailingBytes,
    #[error("invalid JSON object")]
    Json(#[from] serde_json::Error),
    #[error("snapshot dimensions must be between 1 and 1000")]
    SnapshotSize,
}

/// Decodes one frame, returning its consumed wire length. An incomplete frame
/// returns `None`; the caller retains those bytes and supplies more. At EOF use
/// [`decode_exact`] on any nonempty remainder to reject truncation. Errors are fatal to the connection.
/// Oversized lengths, unknown kinds and impossible headers fail immediately,
/// without waiting for their declared payload. Frames are never resynchronized.
pub fn decode(input: &[u8]) -> Result<Option<(Frame<'_>, usize)>, FrameError> {
    let Some(prefix) = input.get(..4) else {
        return Ok(None);
    };
    let len = u32::from_le_bytes(array(prefix)) as usize;
    check_len(len)?;
    let Some(&kind) = input.get(4) else {
        return Ok(None);
    };
    let (minimum, exact) = match kind {
        0x01 => (1, false),
        0x10 => (17, false),
        0x11 => (21, false),
        0x12 => (21, true),
        0x20 => (9, false),
        other => return Err(FrameError::UnknownKind(other)),
    };
    if len < minimum || (exact && len != minimum) {
        return Err(FrameError::InvalidLength);
    }
    let total = len + 4;
    let Some(payload) = input.get(5..total) else {
        return Ok(None);
    };
    let frame = match kind {
        0x01 => Frame::Json(serde_json::from_slice(payload)?),
        0x10 => Frame::Output(Output {
            pane: pane(payload),
            seq: seq(payload),
            bytes: &payload[16..],
        }),
        0x11 => {
            let size = size(payload);
            check_snapshot_size(size)?;
            Frame::Snapshot(Snapshot {
                pane: pane(payload),
                seq: seq(payload),
                size,
                bytes: &payload[20..],
            })
        }
        0x12 => Frame::Resized(Resized {
            pane: pane(payload),
            seq: seq(payload),
            size: size(payload),
        }),
        0x20 => Frame::Input(Input {
            pane: pane(payload),
            bytes: &payload[8..],
        }),
        _ => unreachable!("kind validated above"),
    };
    Ok(Some((frame, total)))
}

/// Decodes exactly one complete wire frame, rejecting trailing bytes.
pub fn decode_exact(input: &[u8]) -> Result<Frame<'_>, FrameError> {
    let (frame, consumed) = decode(input)?.ok_or(FrameError::Truncated)?;
    if consumed != input.len() {
        return Err(FrameError::TrailingBytes);
    }
    Ok(frame)
}

impl Frame<'_> {
    /// Encodes a complete frame, checking the same limits as the decoder.
    pub fn encode(&self) -> Result<Vec<u8>, FrameError> {
        let json;
        let (kind, pane, seq, size, bytes) = match self {
            Self::Json(object) => {
                json = serde_json::to_vec(object)?;
                (0x01, None, None, None, json.as_slice())
            }
            Self::Output(frame) => (0x10, Some(frame.pane), Some(frame.seq), None, frame.bytes),
            Self::Snapshot(frame) => {
                check_snapshot_size(frame.size)?;
                (
                    0x11,
                    Some(frame.pane),
                    Some(frame.seq),
                    Some(frame.size),
                    frame.bytes,
                )
            }
            Self::Resized(frame) => (
                0x12,
                Some(frame.pane),
                Some(frame.seq),
                Some(frame.size),
                &[][..],
            ),
            Self::Input(frame) => (0x20, Some(frame.pane), None, None, frame.bytes),
        };
        let header_len = 1
            + usize::from(pane.is_some()) * 8
            + usize::from(seq.is_some()) * 8
            + usize::from(size.is_some()) * 4;
        let len = bytes
            .len()
            .checked_add(header_len)
            .ok_or(FrameError::TooLarge)?;
        check_len(len)?;
        let mut wire = Vec::with_capacity(len + 4);
        wire.extend_from_slice(&(len as u32).to_le_bytes());
        wire.push(kind);
        if let Some(pane) = pane {
            wire.extend_from_slice(&pane.get().to_le_bytes());
        }
        if let Some(seq) = seq {
            wire.extend_from_slice(&seq.to_le_bytes());
        }
        if let Some(size) = size {
            wire.extend_from_slice(&size.cols.to_le_bytes());
            wire.extend_from_slice(&size.rows.to_le_bytes());
        }
        wire.extend_from_slice(bytes);
        Ok(wire)
    }
}

fn check_len(len: usize) -> Result<(), FrameError> {
    if len > MAX_FRAME_LEN {
        Err(FrameError::TooLarge)
    } else if len == 0 {
        Err(FrameError::InvalidLength)
    } else {
        Ok(())
    }
}

fn check_snapshot_size(size: GridSize) -> Result<(), FrameError> {
    if (1..=1000).contains(&size.cols) && (1..=1000).contains(&size.rows) {
        Ok(())
    } else {
        Err(FrameError::SnapshotSize)
    }
}

// Called only with slices whose minimum lengths were checked above.
fn array<const N: usize>(slice: &[u8]) -> [u8; N] {
    let mut bytes = [0; N];
    bytes.copy_from_slice(slice);
    bytes
}

fn pane(payload: &[u8]) -> PaneId {
    PaneId::new(u64::from_le_bytes(array(&payload[..8])))
}

fn seq(payload: &[u8]) -> u64 {
    u64::from_le_bytes(array(&payload[8..16]))
}

fn size(payload: &[u8]) -> GridSize {
    GridSize {
        cols: u16::from_le_bytes(array(&payload[16..18])),
        rows: u16::from_le_bytes(array(&payload[18..20])),
    }
}
