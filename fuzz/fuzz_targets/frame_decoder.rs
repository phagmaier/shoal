#![no_main]

use libfuzzer_sys::fuzz_target;
use shoal_proto::{Frame, FrameError, Message, decode, decode_exact};

fuzz_target!(|data: &[u8]| {
    // Exercise both the EOF contract and fragmented/concatenated stream parsing.
    let _ = decode_exact(data);
    let mut remaining = data;
    while let Ok(Some((frame, consumed))) = decode(remaining) {
        assert!(consumed > 4 && consumed <= remaining.len());
        match frame.encode() {
            Ok(encoded) => {
                let restored = decode_exact(&encoded).expect("invariant: encoded frame decodes");
                assert_eq!(restored, frame);
            }
            // Canonical JSON numbers can occupy more bytes than the input
            // spelling. Re-encoding a near-limit object may legitimately fail.
            Err(FrameError::TooLarge) => assert!(matches!(frame, Frame::Json(_))),
            Err(error) => panic!("invariant: decoded frame failed to encode: {error}"),
        }
        if let Frame::Json(object) = frame
            && let Ok(message) = Message::from_object(object)
        {
            let value = serde_json::to_value(&message).expect("invariant: message encodes");
            let _: Message = serde_json::from_value(value).expect("invariant: message decodes");
            match message {
                Message::Request(request) => {
                    let _ = request.call();
                }
                Message::Notification(notification) => {
                    let _ = notification.notification();
                }
                Message::Response(_) => {}
            }
        }
        remaining = &remaining[consumed..];
    }
});
