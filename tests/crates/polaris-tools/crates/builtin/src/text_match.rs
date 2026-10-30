// Unit tests of crates/polaris-tools/crates/builtin/src/text_match.rs (`tests`); compiled into that module via #[path].
use super::{MatchLevel, TextMatchError, apply_hunks, replace_text};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn exact_match_is_preferred_and_must_be_unique() -> TestResult {
    let edited = replace_text("let a = 1;\nlet b = 2;\n", "b = 2", "b = 3", false)?;
    assert_eq!(edited.text, "let a = 1;\nlet b = 3;\n");
    assert_eq!(edited.level, MatchLevel::Exact);
    let error = replace_text("x\nx\n", "x", "y", false);
    assert!(
        matches!(&error, Err(TextMatchError::Ambiguous { count: 2, lines }) if lines == "1, 2"),
        "{error:?}"
    );
    let all = replace_text("x\nx\n", "x", "y", true)?;
    assert_eq!((all.text.as_str(), all.count), ("y\ny\n", 2));
    Ok(())
}

#[test]
fn trailing_whitespace_and_crlf_are_absorbed() -> TestResult {
    let source = "fn main() {  \r\n    run();\r\n}\r\n";
    let edited = replace_text(
        source,
        "fn main() {\n    run();\n",
        "fn main() {\n    go();\n",
        false,
    )?;
    assert_eq!(edited.level, MatchLevel::TrailingWhitespace);
    assert_eq!(edited.text, "fn main() {\r\n    go();\r\n}\r\n");
    Ok(())
}

#[test]
fn indentation_and_punctuation_levels_apply_in_order() -> TestResult {
    let source = "impl A {\n        call(x);\n}\n";
    let edited = replace_text(source, "call(x);", "call(y);", false)?;
    assert_eq!(
        edited.level,
        MatchLevel::Exact,
        "a substring still matches exactly"
    );
    let edited = replace_text(source, "\tcall(x);\n", "        call(y);\n", false)?;
    assert_eq!(edited.level, MatchLevel::Indentation);
    assert_eq!(edited.text, "impl A {\n        call(y);\n}\n");
    let quoted = "say(\"it’s — fine…\");\n";
    let edited = replace_text(quoted, "say(\"it's - fine...\");", "say(\"ok\");", false)?;
    assert_eq!(edited.level, MatchLevel::Punctuation);
    assert_eq!(edited.text, "say(\"ok\");\n");
    Ok(())
}

#[test]
fn relaxed_levels_still_require_one_location() {
    let source = "  a();\n    a();\n";
    let error = replace_text(source, "a();\n", "b();\n", false);
    assert!(
        matches!(error, Err(TextMatchError::Ambiguous { count: 2, .. })),
        "{error:?}"
    );
}

#[test]
fn missing_text_points_at_the_closest_lines() -> TestResult {
    let source = "fn alpha() {}\nfn parse_input(text: &str) -> Value {\n    todo!()\n}\n";
    let error = replace_text(
        source,
        "fn parse_input(text: String) -> Value {",
        "x",
        false,
    );
    let Err(TextMatchError::NotFound { hint }) = error else {
        return Err(format!("expected NotFound, got {error:?}").into());
    };
    assert!(hint.contains("line 2"), "{hint}");
    assert!(
        hint.contains("    2| fn parse_input(text: &str) -> Value {"),
        "{hint}"
    );
    assert!(matches!(
        replace_text("a", "", "b", false),
        Err(TextMatchError::Empty)
    ));
    Ok(())
}

const NUMBERS: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\n";

#[test]
fn hunks_are_found_by_content_not_by_line_numbers() -> TestResult {
    // The header claims line 1 and wrong counts; the old lines are at line 4.
    let patch = "--- a/n.txt\n+++ b/n.txt\n@@ -1,9 +1,9 @@\n three\n-four\n+FOUR\n five\n";
    let edited = apply_hunks(NUMBERS, patch)?;
    assert_eq!(edited.text, "one\ntwo\nthree\nFOUR\nfive\nsix\nseven\n");
    assert_eq!(edited.level, MatchLevel::Exact);
    assert_eq!(edited.count, 1);
    Ok(())
}

#[test]
fn several_hunks_apply_in_order_and_blank_context_may_lose_its_space() -> TestResult {
    let source = "fn a() {}\n\nfn b() {}\n\nfn c() {}\n";
    let patch = "@@ -1,3 +1,3 @@\n-fn a() {}\n+fn a() { run(); }\n\n@@ -5 +5 @@\n-fn c() {}\n+fn c() { stop(); }\n";
    let edited = apply_hunks(source, patch)?;
    assert_eq!(
        edited.text,
        "fn a() { run(); }\n\nfn b() {}\n\nfn c() { stop(); }\n"
    );
    assert_eq!(edited.count, 2);
    Ok(())
}

#[test]
fn relaxed_context_keeps_the_source_bytes() -> TestResult {
    let source = "impl A {\r\n    fn go(&self) {\r\n        old();\r\n    }\r\n}\r\n";
    let patch = "@@ -2,3 +2,3 @@\n fn go(&self) {\n-old();\n+new();\n }\n";
    let edited = apply_hunks(source, patch)?;
    assert_eq!(edited.level, MatchLevel::Indentation);
    // Context lines keep their indentation and CRLF; the added line takes CRLF too.
    assert_eq!(
        edited.text,
        "impl A {\r\n    fn go(&self) {\r\nnew();\r\n    }\r\n}\r\n"
    );
    Ok(())
}

#[test]
fn untouched_lines_keep_mixed_endings_and_a_missing_final_newline() -> TestResult {
    let source = "a\r\nb\nc";
    let edited = apply_hunks(source, "@@ -2 +2 @@\n-b\n+B\n")?;
    assert_eq!(edited.text, "a\r\nB\nc");
    let appended = apply_hunks(source, "@@ -3 +3,2 @@\n c\n+d\n")?;
    assert_eq!(appended.text, "a\r\nb\nc\r\nd", "no final newline is kept");
    let terminated = apply_hunks(
        source,
        "@@ -3 +3 @@\n-c\n\\ No newline at end of file\n+c\n",
    )?;
    assert_eq!(
        terminated.text, "a\r\nb\nc\r\n",
        "the marker adds the final newline"
    );
    Ok(())
}

#[test]
fn equal_candidates_need_the_claimed_line_or_more_context() -> TestResult {
    let source = "x = 1\ny = 2\nx = 1\n";
    let tied = apply_hunks(source, "@@ -3 +3 @@\n-x = 1\n+x = 3\n")?;
    assert_eq!(tied.text, "x = 1\ny = 2\nx = 3\n");
    let error = apply_hunks(source, "@@ -9 +9 @@\n-x = 1\n+x = 3\n")
        .err()
        .ok_or("expected an ambiguity")?;
    assert!(
        error.to_string().contains("match 2 places (lines 1, 3)"),
        "{error}"
    );
    Ok(())
}

#[test]
fn bad_patches_explain_what_to_fix() -> TestResult {
    let missing = apply_hunks(NUMBERS, "@@ -1 +1 @@\n-four score\n+40\n")
        .err()
        .ok_or("expected a miss")?;
    let message = missing.to_string();
    assert!(
        message.contains("were not found") && message.contains("    4| four"),
        "{message}"
    );
    let two_files = apply_hunks(
        NUMBERS,
        "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-one\n+1\n--- a/y\n+++ b/y\n@@ -1 +1 @@\n-a\n+b\n",
    )
    .err()
    .ok_or("expected a refusal")?;
    assert!(two_files.to_string().contains("one file"), "{two_files}");
    let second = apply_hunks(NUMBERS, "@@ -1 +1 @@\n-one\n+1\n@@ -5 +5 @@\n-nine\n+9\n")
        .err()
        .ok_or("expected a miss in the second hunk")?;
    assert!(second.to_string().starts_with("hunk 2 of 2:"), "{second}");
    assert!(apply_hunks(NUMBERS, "just text").is_err());
    Ok(())
}
