use proptest::prelude::*;
use shoal_model::{GridSize, PaneId};
use shoal_proto::{
    Frame, FrameError, Input, MAX_FRAME_LEN, Output, Resized, Snapshot, decode, decode_exact,
};

fn fixtures() -> Vec<(&'static str, Vec<u8>)> {
    include_str!("fixtures/frames.hex")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let (name, hex) = line.split_once(':').unwrap();
            (
                name,
                hex.split_whitespace()
                    .map(|byte| u8::from_str_radix(byte, 16).unwrap())
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn every_frame_matches_literal_fixture_and_every_prefix_is_truncated() {
    let size = GridSize {
        cols: 120,
        rows: 40,
    };
    for (name, wire) in fixtures() {
        let expected = match name {
            "json" => Frame::Json(serde_json::Map::new()),
            "output" => Frame::Output(Output {
                pane: PaneId::new(3),
                seq: 1881,
                bytes: b"h\xff\n",
            }),
            "snapshot" => Frame::Snapshot(Snapshot {
                pane: PaneId::new(3),
                seq: 1880,
                size,
                bytes: b"\x00\xff\x80",
            }),
            "resized" => Frame::Resized(Resized {
                pane: PaneId::new(3),
                seq: 1882,
                size,
            }),
            "input" => Frame::Input(Input {
                pane: PaneId::new(3),
                bytes: b"ls\r",
            }),
            _ => panic!("unknown fixture"),
        };
        assert_eq!(decode_exact(&wire).unwrap(), expected, "{name}");
        assert_eq!(expected.encode().unwrap(), wire, "{name}");
        for end in 0..wire.len() {
            assert!(decode(&wire[..end]).unwrap().is_none(), "{name} at {end}");
            assert!(matches!(
                decode_exact(&wire[..end]),
                Err(FrameError::Truncated)
            ));
        }
    }
}

#[test]
fn decoder_consumes_one_frame_and_leaves_next_frame_or_partial_prefix() {
    let fixtures = fixtures();
    let wire: Vec<u8> = fixtures
        .iter()
        .flat_map(|(_, wire)| wire)
        .copied()
        .collect();
    let mut remaining = wire.as_slice();
    for (_, expected) in &fixtures {
        let (actual, consumed) = decode(remaining).unwrap().unwrap();
        assert_eq!(consumed, expected.len());
        assert_eq!(actual, decode_exact(expected).unwrap());
        remaining = &remaining[consumed..];
    }
    assert!(remaining.is_empty());
    assert!(decode(remaining).unwrap().is_none());
    let mut trailing = fixtures[0].1.clone();
    trailing.extend_from_slice(&[0x15, 0]);
    assert_eq!(decode(&trailing).unwrap().unwrap().1, fixtures[0].1.len());
    assert!(matches!(
        decode_exact(&trailing),
        Err(FrameError::TrailingBytes)
    ));
}

#[test]
fn rejects_oversized_prefix_unknown_kinds_and_impossible_lengths_immediately() {
    for len in [MAX_FRAME_LEN as u32 + 1, u32::MAX] {
        assert!(matches!(
            decode(&len.to_le_bytes()),
            Err(FrameError::TooLarge)
        ));
    }
    assert!(matches!(
        decode(&[0, 0, 0, 0]),
        Err(FrameError::InvalidLength)
    ));
    for kind in 0..=u8::MAX {
        if ![0x01, 0x10, 0x11, 0x12, 0x20].contains(&kind) {
            assert!(
                matches!(decode(&[100, 0, 0, 0, kind]), Err(FrameError::UnknownKind(k)) if k == kind)
            );
        }
    }
    for (kind, minimum) in [(0x10, 17), (0x11, 21), (0x12, 21), (0x20, 9)] {
        for len in 1..minimum {
            assert!(matches!(
                decode(&[len, 0, 0, 0, kind]),
                Err(FrameError::InvalidLength)
            ));
        }
    }
    assert!(matches!(
        decode(&[22, 0, 0, 0, 0x12]),
        Err(FrameError::InvalidLength)
    ));
}

#[test]
fn accepts_exact_limit_excluding_prefix_and_rejects_one_byte_over_on_encode() {
    // INPUT's kind and pane consume 9 bytes of the declared len.
    let payload = vec![0xff; MAX_FRAME_LEN - 8];
    let frame = Frame::Input(Input {
        pane: PaneId::new(u64::MAX),
        bytes: &payload[..payload.len() - 1],
    });
    let wire = frame.encode().unwrap();
    assert_eq!(wire.len(), MAX_FRAME_LEN + 4);
    assert_eq!(&wire[..4], &[0, 0, 0, 4]); // 0x04000000 = 64 MiB
    assert_eq!(decode_exact(&wire).unwrap(), frame);
    assert!(matches!(
        Frame::Input(Input {
            pane: PaneId::new(1),
            bytes: &payload
        })
        .encode(),
        Err(FrameError::TooLarge)
    ));
}

#[test]
fn snapshot_headers_enforce_size_limits_without_decoding_opaque_bytes() {
    for size in [
        GridSize { cols: 0, rows: 40 },
        GridSize { cols: 120, rows: 0 },
        GridSize {
            cols: 1001,
            rows: 40,
        },
        GridSize {
            cols: 120,
            rows: 1001,
        },
    ] {
        let frame = Frame::Snapshot(Snapshot {
            pane: PaneId::new(3),
            seq: 0,
            size,
            bytes: &[],
        });
        assert!(matches!(frame.encode(), Err(FrameError::SnapshotSize)));
        let mut wire = fixtures()
            .into_iter()
            .find(|(name, _)| *name == "snapshot")
            .unwrap()
            .1;
        wire[21..23].copy_from_slice(&size.cols.to_le_bytes());
        wire[23..25].copy_from_slice(&size.rows.to_le_bytes());
        assert!(matches!(decode_exact(&wire), Err(FrameError::SnapshotSize)));
    }
    let frame = Frame::Snapshot(Snapshot {
        pane: PaneId::new(3),
        seq: 0,
        size: GridSize {
            cols: 1,
            rows: 1000,
        },
        bytes: b"not a terminal snapshot",
    });
    assert_eq!(decode_exact(&frame.encode().unwrap()).unwrap(), frame);
}

#[test]
fn json_frames_require_one_utf8_object_but_do_not_validate_envelopes() {
    for bytes in [&b"[]"[..], b"null", b"1", b"\xff", b"{", b"{}{}", b""] {
        let mut wire = ((bytes.len() + 1) as u32).to_le_bytes().to_vec();
        wire.push(0x01);
        wire.extend_from_slice(bytes);
        assert!(
            matches!(decode_exact(&wire), Err(FrameError::Json(_))),
            "{bytes:?}"
        );
    }
    // Invalid JSON-RPC is still a valid JSON object; dispatch can reply -32600.
    assert!(matches!(decode_exact(&fixtures()[0].1), Ok(Frame::Json(_))));
}

proptest! {
    #[test]
    fn binary_frames_round_trip_arbitrary_ids_sequences_and_bytes(
        kind in 0..4_u8, pane in any::<u64>(), seq in any::<u64>(),
        cols in 1..=1000_u16, rows in 1..=1000_u16,
        bytes in proptest::collection::vec(any::<u8>(), 0..4096),
    ) {
        let pane = PaneId::new(pane);
        let size = GridSize { cols, rows };
        let frame = match kind {
            0 => Frame::Output(Output { pane, seq, bytes: &bytes }),
            1 => Frame::Snapshot(Snapshot { pane, seq, size, bytes: &bytes }),
            2 => Frame::Resized(Resized { pane, seq, size }),
            _ => Frame::Input(Input { pane, bytes: &bytes }),
        };
        let wire = frame.encode().unwrap();
        prop_assert_eq!(decode_exact(&wire).unwrap(), frame);
    }
}

#[test]
fn fuzz_regression_json_number_keeps_its_value_across_frame_round_trip() {
    let frame = decode_exact(include_bytes!("fixtures/json-number.bin")).unwrap();
    let Frame::Json(object) = &frame else {
        panic!("JSON")
    };
    // Independently rounded Rust literal; the default JSON parser reparsed the
    // serialized number as the adjacent float, 7.700000000000001e33.
    assert_eq!(object["parmas"], serde_json::json!(7.7e33_f64));
    assert_eq!(decode_exact(&frame.encode().unwrap()).unwrap(), frame);
}
