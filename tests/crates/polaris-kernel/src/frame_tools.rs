// Unit tests of crates/polaris-kernel/src/frame_tools.rs (`tests`); compiled into that module via #[path].
use super::serialize_bounded;
use proptest::prelude::*;

proptest! {
    #[test]
    fn bounded_serialization_preserves_exact_json_or_rejects(text in ".{0,1024}", cap in 0_usize..4096) {
        let expected = serde_json::to_vec(&text);
        let result = serialize_bounded(&text, cap);
        match expected {
            Ok(bytes) if bytes.len() <= cap => {
                prop_assert!(matches!(result, Ok(actual) if actual == bytes));
            }
            Ok(_) => prop_assert!(result.is_err()),
            Err(_) => prop_assert!(result.is_err()),
        }
    }
}
