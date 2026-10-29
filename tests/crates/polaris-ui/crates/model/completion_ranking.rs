//! Completion Ranking (descriptor: specs/descriptors/uimodel_completion_ranking.json).

use std::cell::Cell;
use std::collections::BTreeSet;
use std::error::Error;

use polaris_kernel_directives::names::{DIRECTIVES, SLASH_COMMANDS};
use polaris_ui_model::completion::{
    CompletionKind, DIRECTIVE_HELP, INDEX_FETCH_LIMIT, MAX_RESULTS, MatchTier, PathHit, PathIndex,
    SLASH_HELP, StaticPathIndex, complete, detect_token, match_tier,
};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn empty_index() -> StaticPathIndex {
    StaticPathIndex::new(Vec::<String>::new())
}

fn index(paths: &[&str]) -> StaticPathIndex {
    StaticPathIndex::new(paths.iter().map(|p| (*p).to_owned()))
}

fn labels(input: &str, cursor: usize, paths: &impl PathIndex) -> Vec<String> {
    complete(input, cursor, paths)
        .into_iter()
        .map(|s| s.label)
        .collect()
}

/// An index that ignores the query and records the limit it was given.
struct SpyIndex {
    paths: Vec<String>,
    limit_seen: Cell<usize>,
}

impl PathIndex for SpyIndex {
    fn query(&self, _q: &str, limit: usize) -> Vec<PathHit> {
        self.limit_seen.set(limit);
        self.paths
            .iter()
            .take(limit)
            .map(|p| PathHit {
                path: p.clone(),
                is_dir: false,
            })
            .collect()
    }
}

#[test]
fn slash_inside_path_is_not_a_token() -> TestResult {
    for (input, cursor) in [
        ("a/b", 3),
        ("http://x", 8),
        ("/usr/bin", 8),
        ("see /model", 10),
        (" /model", 7),
        ("src/ma", 6),
    ] {
        assert!(
            detect_token(input, cursor).is_none(),
            "{input:?} must not be a token"
        );
        assert!(complete(input, cursor, &empty_index()).is_empty());
    }
    let token = detect_token("/model", 6).ok_or("slash command token")?;
    assert_eq!(token.kind, CompletionKind::Command);
    assert_eq!((token.start, token.end, token.query), (0, 6, "model"));
    let bare = detect_token("/", 1).ok_or("bare slash")?;
    assert_eq!(bare.query, "");
    Ok(())
}

#[test]
fn dollar_and_at_need_token_boundary() -> TestResult {
    let cases: [(&str, usize, Option<(CompletionKind, usize, &str)>); 18] = [
        ("$web", 4, Some((CompletionKind::Directive, 0, "web"))),
        ("use $web", 8, Some((CompletionKind::Directive, 4, "web"))),
        ("x($web", 6, Some((CompletionKind::Directive, 2, "web"))),
        ("x[$web", 6, Some((CompletionKind::Directive, 2, "web"))),
        ("a$web", 5, None),
        ("$5", 2, None),
        ("$HOME", 5, None),
        ("$", 1, Some((CompletionKind::Directive, 0, ""))),
        (
            "price $5 and $web",
            17,
            Some((CompletionKind::Directive, 13, "web")),
        ),
        ("@src", 4, Some((CompletionKind::Mention, 0, "src"))),
        ("see @src", 8, Some((CompletionKind::Mention, 4, "src"))),
        ("(@src", 5, Some((CompletionKind::Mention, 1, "src"))),
        ("dev@x.com", 9, None),
        ("mail dev@x.com", 14, None),
        ("@", 1, Some((CompletionKind::Mention, 0, ""))),
        ("a @", 3, Some((CompletionKind::Mention, 2, ""))),
        ("$websearch", 4, Some((CompletionKind::Directive, 0, "web"))),
        ("see https://a.b/@x", 18, None),
    ];
    for (input, cursor, expected) in cases {
        let got = detect_token(input, cursor).map(|t| (t.kind, t.start, t.query));
        assert_eq!(got, expected, "{input:?} @ {cursor}");
    }
    let mid = detect_token("$websearch", 4).ok_or("mid-token cursor")?;
    assert_eq!(mid.end, 10);
    Ok(())
}

#[test]
fn code_span_is_not_a_token() -> TestResult {
    assert!(detect_token("`$web", 5).is_none());
    assert!(detect_token("```\n$web", 8).is_none());
    assert!(detect_token("`a` $web", 8).is_some());
    assert!(detect_token("`@src` @src", 11).is_some());
    Ok(())
}

#[test]
fn cursor_off_char_boundary_returns_nothing() {
    assert!(detect_token("é$web", 1).is_none());
    assert!(detect_token("é$web", 99).is_none());
    assert!(complete("é", 1, &empty_index()).is_empty());
    assert!(complete("$web", 99, &empty_index()).is_empty());
    assert!(detect_token("", 0).is_none());
    assert!(detect_token("😀@a", 5).is_none());
    assert!(detect_token("😀 @a", 7).is_some());
}

#[test]
fn commands_come_from_directives_names() {
    let got = labels("/", 1, &empty_index());
    let expected: BTreeSet<String> = SLASH_COMMANDS.iter().map(|n| format!("/{n}")).collect();
    assert_eq!(got.len(), SLASH_COMMANDS.len().min(MAX_RESULTS));
    assert_eq!(got.iter().cloned().collect::<BTreeSet<_>>(), expected);
    let mut sorted = got.clone();
    sorted.sort();
    assert_eq!(got, sorted, "an empty query lists names lexicographically");
    assert_eq!(
        labels("/mo", 3, &empty_index()).first().map(String::as_str),
        Some("/model")
    );
    assert_eq!(
        labels("/ex", 3, &empty_index()).first().map(String::as_str),
        Some("/exit")
    );
    assert!(labels("/zzzz", 5, &empty_index()).is_empty());
}

#[test]
fn directives_come_from_directives_names() {
    let got = labels("$", 1, &empty_index());
    let expected: BTreeSet<String> = DIRECTIVES.iter().map(|n| format!("${n}")).collect();
    assert_eq!(got.len(), DIRECTIVES.len().min(MAX_RESULTS));
    assert_eq!(got.iter().cloned().collect::<BTreeSet<_>>(), expected);
    assert_eq!(
        labels("use $web", 8, &empty_index())
            .first()
            .map(String::as_str),
        Some("$websearch")
    );
}

#[test]
fn help_tables_cover_exactly_the_names() {
    let slash: Vec<&str> = SLASH_HELP.iter().map(|h| h.name).collect();
    let directive: Vec<&str> = DIRECTIVE_HELP.iter().map(|h| h.name).collect();
    assert_eq!(
        slash.iter().collect::<BTreeSet<_>>().len(),
        slash.len(),
        "duplicate slash help"
    );
    assert_eq!(
        directive.iter().collect::<BTreeSet<_>>().len(),
        directive.len(),
        "duplicate directive help"
    );
    assert_eq!(
        slash.iter().copied().collect::<BTreeSet<_>>(),
        SLASH_COMMANDS.iter().copied().collect::<BTreeSet<_>>()
    );
    assert_eq!(
        directive.iter().copied().collect::<BTreeSet<_>>(),
        DIRECTIVES.iter().copied().collect::<BTreeSet<_>>()
    );
    for h in SLASH_HELP.iter().chain(DIRECTIVE_HELP.iter()) {
        assert!(
            !h.description.trim().is_empty(),
            "{} has no description",
            h.name
        );
    }
}

#[test]
fn suggestions_carry_the_help_text() -> TestResult {
    let first = complete("/model", 6, &empty_index())
        .into_iter()
        .next()
        .ok_or("a /model suggestion")?;
    let help = SLASH_HELP
        .iter()
        .find(|h| h.name == "model")
        .ok_or("model help")?;
    assert_eq!(first.description, help.description);
    assert_eq!(first.insert, "/model");
    assert_eq!(first.kind, CompletionKind::Command);
    Ok(())
}

#[test]
fn match_tier_classifies_prefix_substring_fuzzy() {
    assert_eq!(match_tier("main", "src/main.rs"), Some(MatchTier::Prefix));
    assert_eq!(match_tier("main", "main/lib.rs"), Some(MatchTier::Prefix));
    assert_eq!(match_tier("src/ma", "src/main.rs"), Some(MatchTier::Prefix));
    assert_eq!(match_tier("MAIN", "src/main.rs"), Some(MatchTier::Prefix));
    assert_eq!(
        match_tier("main", "src/domain.rs"),
        Some(MatchTier::Substring)
    );
    assert_eq!(match_tier("main", "src/m_a_i_n.rs"), Some(MatchTier::Fuzzy));
    assert_eq!(match_tier("main", "docs/readme.md"), None);
    assert_eq!(match_tier("", "anything"), Some(MatchTier::Prefix));
    assert!(MatchTier::Prefix < MatchTier::Substring && MatchTier::Substring < MatchTier::Fuzzy);
}

#[test]
fn prefix_beats_substring_beats_fuzzy() {
    let paths = [
        "src/m_a_i_n.rs",
        "docs/readme.md",
        "src/domain.rs",
        "src/main.rs",
        "main/lib.rs",
    ];
    let expected = [
        "main/lib.rs",
        "src/main.rs",
        "src/domain.rs",
        "src/m_a_i_n.rs",
    ];
    assert_eq!(labels("@main", 5, &index(&paths)), expected);
    // The order does not depend on the order in which the index returns hits.
    let mut reversed = paths;
    reversed.reverse();
    assert_eq!(labels("@main", 5, &index(&reversed)), expected);
    // An index that returns non-matching paths gets them filtered out.
    let spy = SpyIndex {
        paths: paths.iter().map(|p| (*p).to_owned()).collect(),
        limit_seen: Cell::new(0),
    };
    assert_eq!(labels("@main", 5, &spy), expected);
}

#[test]
fn quoted_mention_for_spaces() -> TestResult {
    let idx = index(&["my docs/read me.md", "src/lib.rs"]);
    let got = complete("see @r", 6, &idx);
    let inserts: Vec<&str> = got.iter().map(|s| s.insert.as_str()).collect();
    assert_eq!(inserts, ["@\"my docs/read me.md\"", "@src/lib.rs"]);
    assert_eq!(
        got.first().map(|s| s.label.as_str()),
        Some("my docs/read me.md")
    );
    assert!(got.iter().all(|s| s.kind == CompletionKind::Mention));
    // Typing inside an already opened quote keeps working.
    let quoted = complete("@\"my d", 6, &idx);
    assert_eq!(
        quoted.first().map(|s| (s.insert.as_str(), s.range.clone())),
        Some(("@\"my docs/read me.md\"", 0..6))
    );
    // A path without whitespace stays bare.
    assert!(!got.iter().any(|s| s.insert.starts_with("@\"src")));
    Ok(())
}

#[test]
fn file_results_capped_at_100() {
    let paths: Vec<String> = (0..1000).map(|i| format!("src/file{i:04}.rs")).collect();
    let idx = StaticPathIndex::new(paths.clone());
    let got = labels("@file", 5, &idx);
    assert_eq!(got.len(), 100);
    assert_eq!(got.first().map(String::as_str), Some("src/file0000.rs"));
    assert_eq!(got.last().map(String::as_str), Some("src/file0099.rs"));
    let spy = SpyIndex {
        paths,
        limit_seen: Cell::new(0),
    };
    assert_eq!(labels("@file", 5, &spy).len(), 100);
    assert_eq!(spy.limit_seen.get(), INDEX_FETCH_LIMIT);
    assert!(INDEX_FETCH_LIMIT >= MAX_RESULTS);
    assert_eq!(MAX_RESULTS, 100);
}

#[test]
fn ties_sorted_lexicographically() {
    let a = index(&["b/z1", "a/z1", "c/z1", "B/z1"]);
    let b = index(&["c/z1", "B/z1", "a/z1", "b/z1"]);
    let expected = ["B/z1", "a/z1", "b/z1", "c/z1"];
    assert_eq!(labels("@z", 2, &a), expected);
    assert_eq!(labels("@z", 2, &b), expected);
}

#[test]
fn suggestion_range_replaces_the_token() -> TestResult {
    let first = complete("use $web now", 8, &empty_index())
        .into_iter()
        .next()
        .ok_or("directive suggestion")?;
    assert_eq!(first.insert, "$websearch");
    assert_eq!(first.range, 4..8);
    let mid = complete("use $websearch(x) now", 8, &empty_index())
        .into_iter()
        .next()
        .ok_or("mid-token suggestion")?;
    assert_eq!(
        mid.range,
        4..14,
        "the token extends past the cursor, arguments excluded"
    );
    let idx = index(&["my file.txt"]);
    let input = "open @\"my file.txt\" now";
    let in_quote = complete(input, 12, &idx)
        .into_iter()
        .next()
        .ok_or("quoted suggestion")?;
    assert_eq!(in_quote.range, 5..19);
    Ok(())
}

#[test]
fn static_index_ranks_and_limits() {
    let idx = index(&["b.rs", "a.rs", "c.txt", "zzz"]);
    assert_eq!(idx.query("rs", 10).len(), 2);
    assert_eq!(idx.query("", 3).len(), 3);
    assert_eq!(idx.query("", 0).len(), 0);
    assert!(idx.query("q", 10).is_empty());
    assert!(idx.query("", 10).iter().all(|h| !h.is_dir));
}

proptest! {
    #[test]
    fn prop_never_more_than_100(
        paths in proptest::collection::vec("[a-z/]{1,12}", 0..300),
        q in "[a-z/]{0,6}",
        sigil in prop_oneof![Just('@'), Just('$'), Just('/')],
    ) {
        let idx = StaticPathIndex::new(paths);
        let input = format!("{sigil}{q}");
        let got = complete(&input, input.len(), &idx);
        prop_assert!(got.len() <= 100);
    }

    #[test]
    fn prop_detect_never_panics_and_ranges_are_boundaries(
        input in prop_oneof![
            any::<String>(),
            "[a-z$@/ ()\\[\\]`\"\u{e9}\u{301}\u{1F600}]{0,24}",
        ],
        cursor in 0usize..80,
    ) {
        if let Some(t) = detect_token(&input, cursor) {
            prop_assert!(input.is_char_boundary(t.start));
            prop_assert!(input.is_char_boundary(t.end));
            prop_assert!(t.start < cursor && cursor <= t.end);
            prop_assert!(t.end <= input.len());
        }
        for s in complete(&input, cursor, &StaticPathIndex::new(["a b", "c"])) {
            prop_assert!(input.is_char_boundary(s.range.start));
            prop_assert!(input.is_char_boundary(s.range.end));
        }
    }
}
