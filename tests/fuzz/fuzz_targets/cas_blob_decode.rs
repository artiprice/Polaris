//! Fuzz target `cas_blob_decode` (`polaris-storage-cas`): arbitrary on-disk blob bytes must decode
//! to `Ok` or `Err` without panicking or allocating past the limit; successful decodes respect the
//! limit, and anything encoded decodes back byte-exact.
#![no_main]

use libfuzzer_sys::fuzz_target;
use polaris_storage_cas::codec::{decode_blob, encode_blob};

const LIMIT: u64 = 1 << 20;

fuzz_target!(|data: &[u8]| {
    if let Ok(decoded) = decode_blob(data, LIMIT) {
        assert!(u64::try_from(decoded.len()).unwrap_or(u64::MAX) <= LIMIT);
    }
    if let Ok(blob) = encode_blob(data, 3, 64) {
        assert_eq!(decode_blob(&blob, LIMIT).ok().as_deref(), Some(data));
    }
});
