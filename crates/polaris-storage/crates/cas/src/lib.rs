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
mod store;

pub use error::CasError;
pub use store::{Cas, CasConfig, GcPolicy, GcReport};

use polaris_sdk::ids::{Cid, FastFingerprint, SHORT_REF_PREFIX};

/// Length of a folded reference `§ref:<12hex>` in bytes (`§` is two bytes in UTF-8).
pub const FOLD_REF_LEN: usize = 18;

/// BLAKE3 content id of in-memory bytes. Hot path: no allocation.
#[must_use]
pub fn content_id(bytes: &[u8]) -> Cid {
    Cid::from_bytes(*blake3::hash(bytes).as_bytes())
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
    let hex = cid
        .as_bytes()
        .iter()
        .take(6)
        .flat_map(|b| [hex_digit(b >> 4), hex_digit(b & 0x0f)]);
    let bytes = SHORT_REF_PREFIX.bytes().chain(hex);
    let mut written = 0;
    for (slot, byte) in out.iter_mut().zip(bytes) {
        *slot = byte;
        written += 1;
    }
    written
}
