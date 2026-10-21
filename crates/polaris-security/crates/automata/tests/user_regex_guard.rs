//! User Regex Guard (descriptor: specs/descriptors/automata_user_regex_guard.json).

use std::error::Error;

use polaris_security_automata::{AutomataError, UserRegexLimits, validate_user_regex};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn rejects_oversize_pattern() {
    let limits = UserRegexLimits {
        max_len: 64,
        ..UserRegexLimits::default()
    };
    let long = "a".repeat(65);
    assert!(matches!(
        validate_user_regex(&long, &limits),
        Err(AutomataError::TooLarge { .. })
    ));
}

#[test]
fn rejects_deep_nesting() {
    let deep = format!("{}a{}", "(".repeat(40), ")".repeat(40));
    let limits = UserRegexLimits {
        nest_limit: 16,
        ..UserRegexLimits::default()
    };
    assert!(matches!(
        validate_user_regex(&deep, &limits),
        Err(AutomataError::TooNested)
    ));
}

#[test]
fn accepts_simple_pattern() -> TestResult {
    let ok = validate_user_regex(r"fn\s+[a-z_][a-z0-9_]*\(", &UserRegexLimits::default())?;
    assert_eq!(ok.as_str(), r"fn\s+[a-z_][a-z0-9_]*\(");
    Ok(())
}

#[test]
fn rejects_huge_counted_repetition() {
    let limits = UserRegexLimits::default();
    assert!(matches!(
        validate_user_regex("a{100000}", &limits),
        Err(AutomataError::RepetitionTooLarge { limit: 1000 })
    ));
    // Each count is small; the product (125,000) is what the structural guard must catch
    // (not merely the later compile-size check).
    assert!(
        matches!(
            validate_user_regex("(?:(?:a{50}){50}){50}", &limits),
            Err(AutomataError::RepetitionTooLarge { limit: 1000 })
        ),
        "nested products explode"
    );
}

#[test]
fn rejects_compile_blowup() {
    // Each piece is small, but the Unicode word class repeated many times compiles to a huge NFA.
    let limits = UserRegexLimits {
        nfa_size_bytes: 1 << 16,
        ..UserRegexLimits::default()
    };
    assert!(matches!(
        validate_user_regex(r"\w{40}", &limits),
        Err(AutomataError::TooLarge { .. })
    ));
}

proptest! {
    #[test]
    fn prop_guard_never_panics(pattern in ".{0,80}") {
        let _ = validate_user_regex(&pattern, &UserRegexLimits::default());
    }
}
