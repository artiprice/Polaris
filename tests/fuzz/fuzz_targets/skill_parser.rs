#![no_main]

use libfuzzer_sys::fuzz_target;
use polaris_ext_skills::{MAX_SKILL_BYTES, parse_skill};

fuzz_target!(|data: &[u8]| {
    let input = data.get(..data.len().min(MAX_SKILL_BYTES)).unwrap_or(data);
    let Ok(source) = std::str::from_utf8(input) else {
        return;
    };
    let Ok(parsed) = parse_skill(source) else {
        return;
    };

    assert!(parsed.body.len() <= MAX_SKILL_BYTES);
    assert_eq!(
        parsed.source_blake3.as_bytes(),
        blake3::hash(source.as_bytes()).as_bytes()
    );
    assert_eq!(
        parsed.body_blake3.as_bytes(),
        blake3::hash(parsed.body.as_bytes()).as_bytes()
    );
});
