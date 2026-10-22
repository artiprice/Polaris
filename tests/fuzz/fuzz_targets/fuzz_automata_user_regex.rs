#![no_main]

use libfuzzer_sys::fuzz_target;
use polaris_security_automata::{
    Category, CompileLimits, RuleSetBuilder, ScanBuf, UserRegexLimits, validate_user_regex,
};

const MAX_PATTERN_BYTES: usize = 16 * 1024;
const MAX_HAYSTACK_BYTES: usize = 64 * 1024;

fuzz_target!(|data: &[u8]| {
    let split = data.len().min(MAX_PATTERN_BYTES);
    let pattern_bytes = data.get(..split).unwrap_or(data);
    let pattern = String::from_utf8_lossy(pattern_bytes);
    if let Ok(validated) = validate_user_regex(&pattern, &UserRegexLimits::default()) {
        let mut builder = RuleSetBuilder::default();
        if builder
            .add("fuzzed", validated.as_str(), Category::Secret)
            .is_err()
        {
            return;
        }
        let Ok(scanner) = builder.build(&CompileLimits::default()) else {
            return;
        };
        let haystack = data
            .get(split..)
            .unwrap_or_default()
            .get(..data.len().saturating_sub(split).min(MAX_HAYSTACK_BYTES))
            .unwrap_or_default();
        let mut matches = ScanBuf::with_capacity(64);
        if let Ok(count) = scanner.scan(haystack, &mut matches) {
            assert_eq!(count, matches.len());
            assert!(matches.matches().iter().all(|matched| {
                usize::try_from(matched.start).is_ok_and(|start| start <= haystack.len())
                    && usize::try_from(matched.end).is_ok_and(|end| end <= haystack.len())
                    && matched.start <= matched.end
            }));
        }
    }
});
