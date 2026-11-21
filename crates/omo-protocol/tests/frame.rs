use std::io::Cursor;

use omo_protocol::frame::{FrameError, MAX_FRAME_LEN, decode_frame, encode_frame};

#[test]
fn frame_round_trip() {
    let body = br#"{"kind":"hello","body":{"v":1}}"#;
    let encoded = encode_frame(body).expect("encode");
    let decoded = decode_frame(&mut Cursor::new(encoded)).expect("decode");
    assert_eq!(decoded, body);
}

#[test]
fn frame_larger_than_16_mib_is_rejected() {
    let body = vec![b'x'; MAX_FRAME_LEN + 1];
    assert!(matches!(
        encode_frame(&body),
        Err(FrameError::TooLarge { .. })
    ));
}

#[test]
fn truncated_body_is_rejected() {
    let mut frame = Vec::from((4u32).to_le_bytes());
    frame.extend_from_slice(b"ab");
    assert!(matches!(
        decode_frame(&mut Cursor::new(frame)),
        Err(FrameError::TruncatedBody {
            expected: 4,
            actual: 2
        })
    ));
}

#[test]
fn truncated_length_prefix_is_rejected() {
    assert!(matches!(
        decode_frame(&mut Cursor::new([1u8, 2])),
        Err(FrameError::TruncatedLength { actual: 2 })
    ));
}
