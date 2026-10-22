//! History Search (descriptor: specs/descriptors/registry_history_search.json).

use std::error::Error;

use polaris_sdk::contracts::Timestamp;
use polaris_sdk::ids::SessionId;
use polaris_sdk::registry::ChatRole;
use polaris_storage_registry::{
    MAX_SEARCH_LIMIT, MAX_SEARCH_QUERY_BYTES, Registry, RegistryError, SNIPPET_CHARS,
};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn ts(seconds: i64) -> Result<Timestamp, Box<dyn Error>> {
    Ok(Timestamp::from_second(seconds)?)
}

fn registry_with(texts: &[&str]) -> Result<(tempfile::TempDir, Registry), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let registry = Registry::open(dir.path())?;
    for (i, text) in texts.iter().enumerate() {
        registry.index_message(
            SessionId::from_u128(1),
            ChatRole::User,
            text,
            ts(i64::try_from(i)?)?,
        )?;
    }
    Ok((dir, registry))
}

fn texts_of(registry: &Registry, query: &str) -> Result<Vec<String>, Box<dyn Error>> {
    Ok(registry
        .search_history(query, 100)?
        .into_iter()
        .map(|h| h.snippet)
        .collect())
}

#[test]
fn finds_substring_of_three_chars() -> TestResult {
    let (_dir, registry) = registry_with(&["the quick brown fox", "lazy dog"])?;
    assert_eq!(texts_of(&registry, "ick")?, vec!["the quick brown fox"]);
    assert_eq!(texts_of(&registry, "QUICK")?, vec!["the quick brown fox"]);
    assert_eq!(texts_of(&registry, "wn f")?, vec!["the quick brown fox"]);
    assert!(texts_of(&registry, "ixx")?.is_empty());
    Ok(())
}

#[test]
fn short_query_uses_like_fallback() -> TestResult {
    let (_dir, registry) = registry_with(&["go to x", "ab", "nothing"])?;
    assert_eq!(texts_of(&registry, "go")?, vec!["go to x"]);
    assert_eq!(texts_of(&registry, "x")?, vec!["go to x"]);
    assert_eq!(texts_of(&registry, "ab")?, vec!["ab"]);
    assert!(texts_of(&registry, "zz")?.is_empty());
    Ok(())
}

#[test]
fn quote_in_query_is_escaped() -> TestResult {
    let (_dir, registry) = registry_with(&["say \"hello\" there", "foo\"bar baz", "plain"])?;
    assert_eq!(
        texts_of(&registry, "\"hello\"")?,
        vec!["say \"hello\" there"]
    );
    assert_eq!(texts_of(&registry, "foo\"bar")?, vec!["foo\"bar baz"]);
    // The classic injection shape finds nothing and, above all, is not a syntax error.
    assert!(texts_of(&registry, "\"foo\\\"bar\" OR *")?.is_empty());
    // A lone quote is a short query: LIKE finds the two messages that contain one.
    assert_eq!(texts_of(&registry, "\"")?.len(), 2);
    assert!(texts_of(&registry, "\"\"\"")?.is_empty());
    Ok(())
}

#[test]
fn fts_operators_are_literal() -> TestResult {
    let (_dir, registry) = registry_with(&[
        "xxx OR zzz",
        "xxx alone",
        "zzz alone",
        "NEAR(a b) call",
        "text:abc",
        "abc without column",
        "star * here",
    ])?;
    assert_eq!(texts_of(&registry, "xxx OR zzz")?, vec!["xxx OR zzz"]);
    assert_eq!(texts_of(&registry, "xxx AND zzz")?.len(), 0);
    assert_eq!(texts_of(&registry, "NEAR(a")?, vec!["NEAR(a b) call"]);
    assert_eq!(texts_of(&registry, "text:abc")?, vec!["text:abc"]);
    assert_eq!(texts_of(&registry, "*")?, vec!["star * here"]);
    assert_eq!(texts_of(&registry, "xxx*")?.len(), 0);
    assert_eq!(texts_of(&registry, "-zzz")?.len(), 0);
    Ok(())
}

#[test]
fn like_wildcards_are_literal() -> TestResult {
    let (_dir, registry) = registry_with(&["100% sure", "plain", "snake_case"])?;
    assert_eq!(texts_of(&registry, "%")?, vec!["100% sure"]);
    assert_eq!(texts_of(&registry, "_")?, vec!["snake_case"]);
    assert_eq!(texts_of(&registry, "\\")?.len(), 0);
    Ok(())
}

#[test]
fn limit_is_enforced() -> TestResult {
    let dir = tempfile::tempdir()?;
    let registry = Registry::open(dir.path())?;
    let total = usize::try_from(MAX_SEARCH_LIMIT)? + 20;
    for i in 0..total {
        registry.index_message(
            SessionId::from_u128(1),
            ChatRole::User,
            &format!("needle number {i}"),
            ts(i64::try_from(i)?)?,
        )?;
    }
    assert_eq!(registry.search_history("needle", 5)?.len(), 5);
    assert_eq!(registry.search_history("needle", 0)?.len(), 0);
    assert_eq!(
        registry.search_history("needle", u32::MAX)?.len(),
        usize::try_from(MAX_SEARCH_LIMIT)?,
        "limit is capped"
    );
    // The short-query path honours the limit too.
    assert_eq!(registry.search_history("ne", 3)?.len(), 3);
    Ok(())
}

#[test]
fn hits_are_newest_first() -> TestResult {
    let dir = tempfile::tempdir()?;
    let registry = Registry::open(dir.path())?;
    for at in [30, 10, 20] {
        registry.index_message(
            SessionId::from_u128(1),
            ChatRole::User,
            &format!("shared word at {at}"),
            ts(at)?,
        )?;
    }
    let order: Vec<Timestamp> = registry
        .search_history("shared", 10)?
        .iter()
        .map(|h| h.at)
        .collect();
    assert_eq!(order, vec![ts(30)?, ts(20)?, ts(10)?]);
    Ok(())
}

#[test]
fn blank_query_returns_nothing() -> TestResult {
    let (_dir, registry) = registry_with(&["something"])?;
    assert!(registry.search_history("", 10)?.is_empty());
    assert!(registry.search_history("\0", 10)?.is_empty());
    Ok(())
}

#[test]
fn snippet_is_a_window_around_the_match() -> TestResult {
    let long = format!("{} needle {}", "a".repeat(500), "b".repeat(500));
    let (_dir, registry) = registry_with(&[&long, "short needle text"])?;
    let hits = registry.search_history("needle", 10)?;
    assert_eq!(hits.len(), 2);
    let short = hits.first().ok_or("no hit")?;
    assert_eq!(short.snippet, "short needle text");
    let windowed = hits.get(1).ok_or("no second hit")?;
    assert!(windowed.snippet.chars().count() <= SNIPPET_CHARS);
    assert!(windowed.snippet.contains("needle"));
    Ok(())
}

#[test]
fn unicode_lowercase_expansion_does_not_shift_snippet_window() -> TestResult {
    let text = format!(
        "{}{}needle{}",
        "a".repeat(40),
        "İ".repeat(60),
        "b".repeat(220)
    );
    let (_dir, registry) = registry_with(&[&text])?;
    let hits = registry.search_history("needle", 1)?;
    let hit = hits.first().ok_or("missing Unicode snippet hit")?;
    assert!(
        hit.snippet.contains("needle"),
        "snippet should include match: {:?}",
        hit.snippet
    );
    assert_eq!(
        hit.snippet
            .chars()
            .take_while(|character| *character == 'İ')
            .count(),
        40,
        "the 40-character lead window must be measured in source text"
    );
    Ok(())
}

#[test]
fn oversize_query_is_rejected_before_queueing() -> TestResult {
    let (_dir, registry) = registry_with(&["bounded query"])?;
    let query = "x".repeat(MAX_SEARCH_QUERY_BYTES + 1);
    assert!(matches!(
        registry.search_history(&query, 1),
        Err(RegistryError::BadValue(_))
    ));
    assert!(matches!(
        registry.search_history_for_session(SessionId::from_u128(1), &query, 1),
        Err(RegistryError::BadValue(_))
    ));
    Ok(())
}

#[test]
fn session_scope_is_applied_before_search_limit() -> TestResult {
    let dir = tempfile::tempdir()?;
    let registry = Registry::open(dir.path())?;
    let target = SessionId::from_u128(1);
    for (session, at, text) in [
        (target, 1, "shared needle target older"),
        (SessionId::from_u128(2), 10, "shared needle other newest"),
        (SessionId::from_u128(3), 9, "shared needle other next"),
    ] {
        registry.index_message(session, ChatRole::User, text, ts(at)?)?;
    }
    let fts = registry.search_history_for_session(target, "needle", 1)?;
    assert_eq!(fts.len(), 1);
    assert_eq!(fts.first().map(|hit| hit.session), Some(target));
    assert!(
        fts.first()
            .is_some_and(|hit| hit.snippet.contains("target"))
    );

    let short = registry.search_history_for_session(target, "ne", 1)?;
    assert_eq!(short.len(), 1);
    assert_eq!(short.first().map(|hit| hit.session), Some(target));
    Ok(())
}

/// A substring of `text` with at least 3 chars, chosen by the two selectors.
fn substring_of(text: &str, a: usize, b: usize) -> String {
    let n = text.chars().count();
    let start = a % n.saturating_sub(2).max(1);
    let room = n.saturating_sub(start).saturating_sub(2).max(1);
    let len = 3 + b % room;
    text.chars().skip(start).take(len).collect()
}

fn finds_every_substring(text: &str, a: usize, b: usize) -> TestResult {
    let (_dir, registry) = registry_with(&["unrelated filler text"])?;
    let target = SessionId::from_u128(42);
    registry.index_message(target, ChatRole::User, text, ts(500)?)?;
    let query = substring_of(text, a, b);
    let hits = registry.search_history(&query, 100)?;
    assert!(
        hits.iter().any(|h| h.session == target),
        "indexed text {text:?} not found by its substring {query:?}"
    );
    Ok(())
}

fn never_errors(query: &str) -> TestResult {
    let (_dir, registry) = registry_with(&["alpha beta gamma", "say \"hi\" OR bye *"])?;
    registry.search_history(query, 10)?;
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]
    #[test]
    fn prop_indexed_text_found_by_any_substring(
        text in "\\PC{3,60}",
        a in 0usize..64,
        b in 0usize..64,
    ) {
        let result = finds_every_substring(&text, a, b);
        prop_assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn prop_search_never_errors(query in any::<String>()) {
        let result = never_errors(&query);
        prop_assert!(result.is_ok(), "{result:?}");
    }
}
