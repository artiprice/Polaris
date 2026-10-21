//! On-disk blob format v1: `[version = 1][codec][payload]`, codec `0` = raw, `1` = zstd.
//! zstd frames always declare their content size; decoding rejects frames that declare more than
//! the limit *before* allocating, so a small "zstd bomb" cannot exhaust memory.

use crate::error::CasError;

/// Current blob format version.
pub const FORMAT_VERSION: u8 = 1;

/// Payload encoding.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// Stored as is (small or incompressible data).
    Raw = 0,
    /// One zstd frame with a declared content size.
    Zstd = 1,
}

/// The codec of an encoded blob, if its header is valid.
#[must_use]
pub fn codec_of(blob: &[u8]) -> Option<Codec> {
    match blob {
        [FORMAT_VERSION, 0, ..] => Some(Codec::Raw),
        [FORMAT_VERSION, 1, ..] => Some(Codec::Zstd),
        _ => None,
    }
}

fn with_header(codec: Codec, payload: &[u8]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(payload.len().saturating_add(2));
    blob.push(FORMAT_VERSION);
    blob.push(codec as u8);
    blob.extend_from_slice(payload);
    blob
}

/// Encodes `bytes`: zstd at `level` when at least `min_compress_bytes` long and it actually
/// shrinks, raw otherwise.
///
/// # Errors
/// [`CasError::Decompress`] if the zstd encoder fails.
pub fn encode_blob(
    bytes: &[u8],
    level: i32,
    min_compress_bytes: usize,
) -> Result<Vec<u8>, CasError> {
    if bytes.len() >= min_compress_bytes {
        let compressed =
            zstd::bulk::compress(bytes, level).map_err(|e| CasError::Decompress(e.to_string()))?;
        if compressed.len() < bytes.len() {
            return Ok(with_header(Codec::Zstd, &compressed));
        }
    }
    Ok(with_header(Codec::Raw, bytes))
}

fn size_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Decodes a blob, refusing anything that would exceed `max_bytes`.
///
/// # Errors
/// [`CasError::Format`] for an invalid header, [`CasError::TooLarge`] when the payload or the
/// declared zstd content size exceeds `max_bytes`, [`CasError::Decompress`] for invalid,
/// truncated or size-undeclared zstd frames.
pub fn decode_blob(blob: &[u8], max_bytes: u64) -> Result<Vec<u8>, CasError> {
    match blob {
        [version, ..] if *version != FORMAT_VERSION => {
            Err(CasError::Format("unsupported format version"))
        }
        [_, 0, payload @ ..] => {
            let size = size_u64(payload.len());
            if size > max_bytes {
                return Err(CasError::TooLarge {
                    limit: max_bytes,
                    size,
                });
            }
            Ok(payload.to_vec())
        }
        [_, 1, payload @ ..] => {
            let declared = zstd::zstd_safe::get_frame_content_size(payload)
                .map_err(|_| CasError::Decompress("invalid zstd frame header".to_owned()))?
                .ok_or_else(|| {
                    CasError::Decompress("zstd frame without a declared content size".to_owned())
                })?;
            if declared > max_bytes {
                return Err(CasError::TooLarge {
                    limit: max_bytes,
                    size: declared,
                });
            }
            let capacity = usize::try_from(declared).map_err(|_| CasError::TooLarge {
                limit: max_bytes,
                size: declared,
            })?;
            let out = zstd::bulk::decompress(payload, capacity)
                .map_err(|e| CasError::Decompress(e.to_string()))?;
            if size_u64(out.len()) != declared {
                return Err(CasError::Decompress(
                    "decoded size differs from the declared size".to_owned(),
                ));
            }
            Ok(out)
        }
        [_, _, ..] => Err(CasError::Format("unknown codec")),
        _ => Err(CasError::Format("blob shorter than its header")),
    }
}
