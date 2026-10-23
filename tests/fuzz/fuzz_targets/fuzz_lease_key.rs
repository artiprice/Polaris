#![no_main]
use libfuzzer_sys::fuzz_target;
use polaris_runtime_leases::LeaseKey;
use std::hash::{Hash, Hasher};

fuzz_target!(|data: &[u8]| {
    if data.len() > 16_384 {
        return;
    }
    let input = String::from_utf8_lossy(data);
    if let Ok(key) = LeaseKey::from_resolved_relative(&input) {
        let text = key.as_str();
        assert!(!text.is_empty() && !text.starts_with('/') && !text.starts_with('\\'));
        assert!(
            text.split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains(':'))
        );
        let rebuilt = LeaseKey::from_resolved_relative(text);
        assert_eq!(
            rebuilt.as_ref().ok(),
            Some(&key),
            "canonicalization is idempotent"
        );
        if let Ok(rebuilt) = rebuilt {
            let mut first = std::collections::hash_map::DefaultHasher::new();
            let mut second = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut first);
            rebuilt.hash(&mut second);
            assert_eq!(
                first.finish(),
                second.finish(),
                "equal keys have equal cached hashes"
            );
        }
    }
});
