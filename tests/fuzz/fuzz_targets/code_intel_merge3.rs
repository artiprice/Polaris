#![no_main]

use libfuzzer_sys::fuzz_target;
use polaris_code_intel::{Lang, MergeOutcome, ParseLimits, ParserPool, merge3};
use std::time::Duration;

const MAX_INPUT_BYTES: usize = 64 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() < 3 {
        return;
    }
    let payload = data.get(2..).unwrap_or_default();
    let first = usize::from(data.first().copied().unwrap_or_default()) % (payload.len() + 1);
    let second = usize::from(data.get(1).copied().unwrap_or_default()) % (payload.len() + 1);
    let (middle, end) = if first <= second {
        (first, second)
    } else {
        (second, first)
    };
    let base = payload.get(..middle).unwrap_or_default();
    let ours = payload.get(middle..end).unwrap_or_default();
    let theirs = payload.get(end..).unwrap_or_default();
    if [base.len(), ours.len(), theirs.len()]
        .iter()
        .any(|len| *len > MAX_INPUT_BYTES)
    {
        return;
    }

    let pool = ParserPool::with_limits(ParseLimits {
        max_source_bytes: MAX_INPUT_BYTES,
        timeout: Duration::from_millis(50),
    });
    for lang in Lang::ALL {
        let (Ok(base), Ok(ours), Ok(theirs)) = (
            pool.parse(lang, base, None),
            pool.parse(lang, ours, None),
            pool.parse(lang, theirs, None),
        ) else {
            continue;
        };
        if let Ok(MergeOutcome::Merged(_)) = merge3(&base, &ours, &theirs) {}
    }
});
