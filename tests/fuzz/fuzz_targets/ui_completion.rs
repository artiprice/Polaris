#![no_main]
//! Synthetic arbitrary UTF-8: shared scanner ranges, replacement ranges and result bounds.

use libfuzzer_sys::fuzz_target;
use polaris_ui_model::classify::classify;
use polaris_ui_model::completion::{MAX_RESULTS, StaticPathIndex, complete, detect_token};

fuzz_target!(|data: &[u8]| {
    let Ok(input) = core::str::from_utf8(data) else {
        return;
    };
    // Bound per-input work while keeping arbitrary Unicode, combining marks and invalid cursors.
    let input: String = input.chars().take(2048).collect();
    let index = StaticPathIndex::new(["src/main.rs", "my docs/read me.md", "é/配置.rs"]);
    let arbitrary_cursor = match data.first() {
        Some(byte) => usize::from(*byte),
        None => 0,
    };
    for cursor in [
        0,
        input.len(),
        arbitrary_cursor,
        input.len().saturating_add(1),
    ] {
        if let Some(token) = detect_token(&input, cursor) {
            assert!(
                token.start < cursor && cursor <= token.end,
                "token contains cursor"
            );
            assert!(
                input.get(token.start..token.end).is_some(),
                "valid token range"
            );
        }
        let suggestions = complete(&input, cursor, &index);
        assert!(suggestions.len() <= MAX_RESULTS, "bounded suggestions");
        for suggestion in suggestions {
            assert!(
                input.get(suggestion.range).is_some(),
                "valid replacement range"
            );
        }
    }
    let mut end = 0;
    for span in classify(&input) {
        assert!(
            span.start >= end && span.start < span.end,
            "ordered, nonempty spans"
        );
        assert!(
            input.get(span.start..span.end).is_some(),
            "valid classification range"
        );
        end = span.end;
    }
});
