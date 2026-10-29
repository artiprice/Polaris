// Unit tests of crates/polaris-kernel/src/frame_prompt.rs (`tests`); compiled into that module via #[path].
use super::bounded_json;
use proptest::prelude::*;

proptest! {
    #[test]
    fn prompt_encoded_limit_matches_exact_escaped_json(text in ".{0,2048}", cap in 0_usize..8192) {
        let expected = serde_json::to_vec(&text);
        let measured = bounded_json(&text, cap);
        match expected {
            Ok(bytes) => prop_assert_eq!(measured.is_ok(), bytes.len() <= cap),
            Err(_) => prop_assert!(measured.is_err()),
        }
    }
}
