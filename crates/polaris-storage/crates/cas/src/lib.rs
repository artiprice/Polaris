//! # polaris-storage-cas
//!
//! Content-addressable blob store under `~/.polaris/cas/<ab>/<cd>/<blake3-hex>`: file versions,
//! diffs, tool outputs, logs, and the byte-exact backing of `§ref:<12hex>` folding
//! (`polaris_expand` returns the original bytes).
//!
//! - Ids are BLAKE3 ([`content_id`]); [`fast_fingerprint`] (xxh3-64) is for dirty checks only.
//! - Blobs use format v1 ([`codec`]): raw or zstd with a declared size, bounded on decode.
//! - Writes are atomic (temp + fsync + rename) with a bounded retry for files held by antivirus
//!   scanners ([`atomic`]); a second `put` of the same bytes never rewrites the blob.
//! - Short references: [`Cas::resolve`] and [`Cas::short_ref_checked`] detect ambiguity, so the fold
//!   layer can fall back to the full 64-hex id.
//! - Hot path (zero allocation, PLAN ADR-003): [`content_id`], [`fast_fingerprint`], [`fold_ref`],
//!   and `polaris_sdk::ids::ShortRef::of`.

pub mod atomic;
pub mod codec;
mod error;
#[cfg(feature = "kani")]
mod kani_proofs;
mod store;

pub use error::CasError;
pub use store::{Cas, CasConfig, GcPolicy, GcReport};

use polaris_sdk::ids::{Cid, FastFingerprint};

/// Length of a folded reference `§ref:<12hex>` in bytes (`§` is two bytes in UTF-8).
pub const FOLD_REF_LEN: usize = 18;

/// BLAKE3 content id of in-memory bytes. Hot path: no allocation.
#[must_use]
pub fn content_id(bytes: &[u8]) -> Cid {
    Cid::from_bytes(*blake3::hash(bytes).as_bytes())
}

/// Hash a bounded stream using fixed-size chunks and BLAKE3's audited Rayon implementation.
/// At most 8 MiB is buffered; this is an I/O path, outside the allocation-free bus/hash hot path.
/// # Errors
/// Input exceeds `max_bytes`, reading fails, or a reader violates its buffer-length contract.
pub fn content_id_reader(reader: impl std::io::Read, max_bytes: u64) -> Result<Cid, CasError> {
    use std::io::Read;
    let capacity = usize::try_from(max_bytes.saturating_add(1).min(8 << 20))
        .map_err(|_| CasError::Format("stream buffer size cannot be represented"))?;
    let mut buffer = vec![0; capacity];
    let mut reader = reader.take(max_bytes.saturating_add(1));
    let mut size = 0_u64;
    let mut hasher = blake3::Hasher::new();
    loop {
        let count = match reader.read(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        if count == 0 {
            break;
        }
        let count_u64 = u64::try_from(count).map_err(|_| CasError::TooLarge {
            limit: max_bytes,
            size: u64::MAX,
        })?;
        size = size.checked_add(count_u64).ok_or(CasError::TooLarge {
            limit: max_bytes,
            size: u64::MAX,
        })?;
        if size > max_bytes {
            return Err(CasError::TooLarge {
                limit: max_bytes,
                size,
            });
        }
        let chunk = buffer.get(..count).ok_or(CasError::Format(
            "reader returned more bytes than its buffer",
        ))?;
        hasher.update_rayon(chunk);
    }
    Ok(Cid::from_bytes(*hasher.finalize().as_bytes()))
}

/// xxh3-64 fingerprint for cheap dirty checks (not collision resistant). Hot path: no allocation.
#[must_use]
pub fn fast_fingerprint(bytes: &[u8]) -> FastFingerprint {
    FastFingerprint::new(xxhash_rust::xxh3::xxh3_64(bytes))
}

const fn hex_digit(nibble: u8) -> u8 {
    if nibble < 10 {
        b'0' + nibble
    } else {
        b'a' + (nibble - 10)
    }
}

/// Writes `§ref:<12hex>` for `cid` into a caller buffer and returns the number of bytes written
/// ([`FOLD_REF_LEN`]). Hot path: no allocation.
pub fn fold_ref(cid: &Cid, out: &mut [u8; FOLD_REF_LEN]) -> usize {
    // Fixed-size destructuring keeps bounds explicit and the universal Kani proof tractable.
    // The iterator-chain formulation exceeded the verifier's 3 GiB memory budget.
    let [first, second, third, fourth, fifth, sixth, ..] = *cid.as_bytes();
    *out = [
        0xc2,
        0xa7,
        b'r',
        b'e',
        b'f',
        b':',
        hex_digit(first >> 4),
        hex_digit(first & 0x0f),
        hex_digit(second >> 4),
        hex_digit(second & 0x0f),
        hex_digit(third >> 4),
        hex_digit(third & 0x0f),
        hex_digit(fourth >> 4),
        hex_digit(fourth & 0x0f),
        hex_digit(fifth >> 4),
        hex_digit(fifth & 0x0f),
        hex_digit(sixth >> 4),
        hex_digit(sixth & 0x0f),
    ];
    FOLD_REF_LEN
}
