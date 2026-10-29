//! Message Size Guard (descriptor: specs/descriptors/proto_message_size_guard.json).

use std::error::Error;

use polaris_proto::limits::{
    MAX_CHUNK_BYTES, MAX_MESSAGE_BYTES, check_message_len, decode_checked, encode_checked,
};
use polaris_proto::{ProtoError, pb};
use prost::Message;

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn at_limit_is_accepted() -> TestResult {
    check_message_len(MAX_MESSAGE_BYTES, MAX_MESSAGE_BYTES)?;
    check_message_len(10, 10)?;
    Ok(())
}

#[test]
fn over_limit_is_rejected() {
    assert_eq!(
        check_message_len(MAX_MESSAGE_BYTES + 1, MAX_MESSAGE_BYTES),
        Err(ProtoError::Oversize {
            len: MAX_MESSAGE_BYTES + 1,
            limit: MAX_MESSAGE_BYTES
        })
    );
    assert_eq!(
        check_message_len(usize::MAX, MAX_MESSAGE_BYTES),
        Err(ProtoError::Oversize {
            len: usize::MAX,
            limit: MAX_MESSAGE_BYTES
        })
    );
}

#[test]
fn limit_is_configurable() -> TestResult {
    check_message_len(100, 100)?;
    check_message_len(100, 1000)?;
    assert!(check_message_len(100, 99).is_err());
    assert!(check_message_len(1, 0).is_err());
    Ok(())
}

#[test]
fn zero_length_is_accepted() -> TestResult {
    check_message_len(0, MAX_MESSAGE_BYTES)?;
    check_message_len(0, 0)?;
    Ok(())
}

#[test]
fn gateway_constants_match_pitfall_w22() {
    assert_eq!(MAX_MESSAGE_BYTES, 1024 * 1024);
    assert_eq!(MAX_CHUNK_BYTES, 896 * 1024);
    // Framing (field tags, length prefixes, the oneof wrapper) of one chunk frame is tiny.
    let frame = pb::ExecServerFrame {
        kind: Some(pb::exec_server_frame::Kind::Stdout(vec![
            0;
            MAX_CHUNK_BYTES
        ])),
    };
    assert!(frame.encoded_len() <= MAX_MESSAGE_BYTES);
    assert!(frame.encoded_len() > MAX_CHUNK_BYTES);
}

#[test]
fn encode_checked_rejects_before_encoding() -> TestResult {
    let cid = pb::Cid {
        digest: vec![9; 32],
    };
    let ok = encode_checked(&cid, 64)?;
    assert_eq!(ok, cid.encode_to_vec());
    // The message encodes to 34 bytes: 33 is one too few.
    assert_eq!(
        encode_checked(&cid, 33),
        Err(ProtoError::Oversize { len: 34, limit: 33 })
    );
    encode_checked(&cid, 34)?;
    let big = pb::ExecServerFrame {
        kind: Some(pb::exec_server_frame::Kind::Stdout(vec![
            1;
            MAX_MESSAGE_BYTES
        ])),
    };
    assert!(matches!(
        encode_checked(&big, MAX_MESSAGE_BYTES),
        Err(ProtoError::Oversize { .. })
    ));
    Ok(())
}

#[test]
fn decode_checked_rejects_before_decoding() -> TestResult {
    let cid = pb::Cid {
        digest: vec![5; 32],
    };
    let bytes = cid.encode_to_vec();
    let back: pb::Cid = decode_checked(&bytes, bytes.len())?;
    assert_eq!(back, cid);
    let refused = decode_checked::<pb::Cid>(&bytes, bytes.len() - 1);
    assert_eq!(
        refused,
        Err(ProtoError::Oversize {
            len: bytes.len(),
            limit: bytes.len() - 1
        })
    );
    // Larger than the limit and not even valid protobuf: still Oversize, proving the length is
    // checked first.
    let junk = vec![0xFF; MAX_MESSAGE_BYTES + 1];
    assert!(matches!(
        decode_checked::<pb::Cid>(&junk, MAX_MESSAGE_BYTES),
        Err(ProtoError::Oversize { .. })
    ));
    // Within the limit, invalid protobuf is a Decode error.
    assert!(matches!(
        decode_checked::<pb::Cid>(&[0xFF; 8], 64),
        Err(ProtoError::Decode(_))
    ));
    Ok(())
}
