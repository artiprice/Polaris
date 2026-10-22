#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use polaris_security_automata::{DfaScanner, ScanBuf};

const MAX_HAYSTACK_BYTES: usize = 1 << 20;
static SCANNER: OnceLock<Option<DfaScanner>> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let Some(scanner) = SCANNER
        .get_or_init(|| DfaScanner::default_pack().ok())
        .as_ref()
    else {
        return;
    };
    let haystack = data.get(..data.len().min(MAX_HAYSTACK_BYTES)).unwrap_or(data);
    let mut matches = ScanBuf::with_capacity(64);
    if let Ok(count) = scanner.scan(haystack, &mut matches) {
        assert_eq!(count, matches.len());
        assert!(matches.matches().iter().all(|matched| {
            usize::try_from(matched.start).is_ok_and(|start| start <= haystack.len())
                && usize::try_from(matched.end).is_ok_and(|end| end <= haystack.len())
                && matched.start <= matched.end
        }));
        assert!(matches
            .matches()
            .windows(2)
            .all(|pair| pair[0].start <= pair[1].start));
    }
    let _any_match = scanner.any_match(haystack);
});
