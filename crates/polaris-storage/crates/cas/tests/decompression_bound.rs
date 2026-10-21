//! Decompression Bound (descriptor: specs/descriptors/cas_decompression_bound.json).

use std::error::Error;
use std::io::Write;

use polaris_storage_cas::CasError;
use polaris_storage_cas::codec::{Codec, FORMAT_VERSION, decode_blob, encode_blob};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn zstd_blob(payload: Vec<u8>) -> Vec<u8> {
    let mut blob = vec![FORMAT_VERSION, Codec::Zstd as u8];
    blob.extend(payload);
    blob
}

#[test]
fn declared_size_over_limit_rejected() -> TestResult {
    let blob = encode_blob(&vec![0u8; 1024 * 1024], 3, 64)?;
    assert!(
        blob.len() < 1024,
        "1 MiB of zeros compresses to a tiny frame ({} bytes)",
        blob.len()
    );
    assert!(matches!(
        decode_blob(&blob, 64 * 1024),
        Err(CasError::TooLarge { limit: 65_536, .. })
    ));
    assert_eq!(decode_blob(&blob, 1024 * 1024)?.len(), 1024 * 1024);
    Ok(())
}

#[test]
fn unknown_content_size_rejected() -> TestResult {
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3)?;
    encoder.include_contentsize(false)?;
    encoder.write_all(b"no declared size in this frame")?;
    let frame = encoder.finish()?;
    assert!(matches!(
        decode_blob(&zstd_blob(frame), 1 << 20),
        Err(CasError::Decompress(_))
    ));
    Ok(())
}

#[test]
fn truncated_frame_is_error() -> TestResult {
    let blob = encode_blob("abc".repeat(10_000).as_bytes(), 3, 64)?;
    let cut = blob.get(..blob.len() / 2).ok_or("cut")?;
    assert!(decode_blob(cut, 1 << 20).is_err());
    Ok(())
}

#[test]
fn unknown_codec_or_version_is_error() {
    assert!(matches!(
        decode_blob(&[FORMAT_VERSION, 9, 1, 2], 1 << 20),
        Err(CasError::Format(_))
    ));
    assert!(matches!(
        decode_blob(&[2, Codec::Raw as u8, 1, 2], 1 << 20),
        Err(CasError::Format(_))
    ));
    assert!(matches!(
        decode_blob(&[], 1 << 20),
        Err(CasError::Format(_))
    ));
    assert!(matches!(
        decode_blob(&[FORMAT_VERSION], 1 << 20),
        Err(CasError::Format(_))
    ));
}

#[test]
fn empty_payload_ok() -> TestResult {
    assert_eq!(
        decode_blob(&[FORMAT_VERSION, Codec::Raw as u8], 1 << 20)?,
        Vec::<u8>::new()
    );
    assert_eq!(
        decode_blob(&encode_blob(b"", 3, 64)?, 1 << 20)?,
        Vec::<u8>::new()
    );
    Ok(())
}

proptest! {
    #[test]
    fn prop_decode_never_panics(blob in prop::collection::vec(any::<u8>(), 0..512)) {
        let _ = decode_blob(&blob, 1 << 16);
    }

    #[test]
    fn prop_encode_decode_roundtrip(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let blob = encode_blob(&bytes, 3, 64);
        prop_assert!(blob.is_ok());
        if let Ok(blob) = blob {
            prop_assert_eq!(decode_blob(&blob, 1 << 16).ok(), Some(bytes));
        }
    }
}
