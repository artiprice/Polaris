#![no_main]

use std::path::Path;

use libfuzzer_sys::fuzz_target;
use polaris_storage_sync::{Exclusion, IgnoreRules, RelPath};

const MAX_INPUT_BYTES: usize = 1 << 20;

fuzz_target!(|data: &[u8]| {
    let input = data.get(..data.len().min(MAX_INPUT_BYTES)).unwrap_or(data);
    let Ok(text) = std::str::from_utf8(input) else {
        return;
    };
    let Ok(rules) = IgnoreRules::new(Path::new("."), Some(text)) else {
        return;
    };
    for candidate in [".git/config", ".polaris/state", ".env", "src/main.rs"] {
        let Ok(path) = RelPath::new(candidate) else {
            continue;
        };
        let result = rules.classify(&path, false);
        if candidate.starts_with(".git/")
            || candidate.starts_with(".polaris/")
            || candidate == ".env"
        {
            assert_eq!(result, Exclusion::Denied);
        }
    }
});
