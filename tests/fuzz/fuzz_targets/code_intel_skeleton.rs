#![no_main]

use libfuzzer_sys::fuzz_target;
use polaris_code_intel::{Lang, ParseLimits, ParserPool, skeleton};
use std::time::Duration;

fuzz_target!(|data: &[u8]| {
    let pool = ParserPool::with_limits(ParseLimits {
        max_source_bytes: 64 * 1024,
        timeout: Duration::from_millis(50),
    });
    for lang in Lang::ALL {
        if let Ok(parsed) = pool.parse(lang, data, None) {
            let _ = skeleton(&parsed);
        }
    }
});
