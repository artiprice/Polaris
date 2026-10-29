//! Fts Query Sanitising (descriptor: specs/descriptors/registry_fts_query_sanitising.json).

use std::error::Error;

use polaris_storage_registry::{fts_phrase, like_pattern};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn phrase_is_wrapped_in_double_quotes() -> TestResult {
    assert_eq!(fts_phrase("hello world"), "\"hello world\"");
    assert_eq!(fts_phrase(""), "\"\"");
    Ok(())
}

#[test]
fn embedded_quotes_are_doubled() -> TestResult {
    assert_eq!(fts_phrase("foo\"bar"), "\"foo\"\"bar\"");
    assert_eq!(
        fts_phrase("\"foo\"bar\" OR *"),
        "\"\"\"foo\"\"bar\"\" OR *\""
    );
    Ok(())
}

#[test]
fn nul_characters_are_removed() -> TestResult {
    assert_eq!(fts_phrase("a\0b"), "\"ab\"");
    assert_eq!(like_pattern("a\0b"), "%ab%");
    Ok(())
}

#[test]
fn like_pattern_escapes_wildcards() -> TestResult {
    assert_eq!(like_pattern("ab"), "%ab%");
    assert_eq!(like_pattern("50%"), "%50\\%%");
    assert_eq!(like_pattern("a_b"), "%a\\_b%");
    assert_eq!(like_pattern("c:\\x"), "%c:\\\\x%");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn prop_phrase_quotes_are_balanced(query in any::<String>()) {
        let phrase = fts_phrase(&query);
        prop_assert!(phrase.starts_with('"') && phrase.ends_with('"') && phrase.len() >= 2);
        let inner: String = phrase.chars().skip(1).take(phrase.chars().count().saturating_sub(2)).collect();
        // Every quote inside is part of a doubled pair: removing the pairs leaves no quote.
        prop_assert!(!inner.replace("\"\"", "").contains('"'));
        prop_assert!(!phrase.contains('\0'));
    }
}
