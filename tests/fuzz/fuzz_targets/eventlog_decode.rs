//! Fuzz target `eventlog_decode` (`polaris-storage-eventlog`): arbitrary stored bytes must decode
//! to `Ok` or `Err` without panicking, and every decoded record re-encodes and decodes unchanged.
#![no_main]

use libfuzzer_sys::fuzz_target;
use polaris_storage_eventlog::codec::{decode_record, encode_record};

fuzz_target!(|data: &[u8]| {
    if let Ok(record) = decode_record(data) {
        let again = encode_record(&record)
            .ok()
            .and_then(|bytes| decode_record(&bytes).ok());
        assert_eq!(again, Some(record));
    }
});
