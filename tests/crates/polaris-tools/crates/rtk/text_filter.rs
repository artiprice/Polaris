//! Text filter (descriptor: specs/descriptors/rtk_text_filter.json).

use std::error::Error;

use polaris_tools_rtk::{
    Diagnostic, IssueId, Level, Location, StreamFormat, ThirdPartyMatcher, filter_text,
    filter_text_with, parse_diagnostics,
};
use proptest::prelude::*;
use serde_json::json;

type TestResult = Result<(), Box<dyn Error>>;

const E0425: &str = "\
error[E0425]: cannot find value `config` in this scope
 --> src/main.rs:2:5
  |
2 |     config.run();
  |     ^^^^^^ not found in this scope
  |
  = note: names are case sensitive
  = help: consider importing it

";

const E0308_WITH_DEP_NOTES: &str = "\
   Compiling demo v0.1.0 (C:\\w\\demo)
error[E0308]: mismatched types
 --> src\\main.rs:4:18
  |
4 |     let x: u32 = \"a\";
  |            ---   ^^^ expected `u32`, found `&str`
  |
note: required by a bound in `foo`
 --> C:\\Users\\x\\.cargo\\registry\\src\\index.crates.io-1949cf8c6b5b557f\\dep-1.0.0\\src\\lib.rs:5:1
note: see C:\\Users\\x\\.cargo\\git\\checkouts\\repo-abc\\1234\\src\\lib.rs
note: see /home/x/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/dep/src/lib.rs
  = help: keep this one

error: could not compile `demo` (bin \"demo\") due to 1 previous error
";

fn filter(stream: &str) -> Result<String, Box<dyn Error>> {
    Ok(filter_text(stream.as_bytes())?)
}

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

#[test]
fn registry_paths_dropped_on_windows_separators() -> TestResult {
    let out = filter(E0308_WITH_DEP_NOTES)?;
    assert!(out.contains("error[E0308]: mismatched types"), "got {out}");
    assert!(out.contains("--> src\\main.rs:4:18"), "got {out}");
    assert!(out.contains("= help: keep this one"), "got {out}");
    assert!(!out.contains("registry"), "got {out}");
    assert!(!out.contains(".cargo\\git"), "got {out}");
    assert!(!out.contains("Compiling"), "got {out}");
    Ok(())
}

#[test]
fn custom_cargo_home_is_honoured() -> TestResult {
    let stream = "\
error[E0599]: no method named `go`
 --> src/main.rs:1:1
note: defined here
 --> D:\\tools\\cargo-home\\registry\\src\\x\\lib.rs:1:1
";
    let custom = ThirdPartyMatcher::new(Some("D:\\tools\\cargo-home"));
    let out = filter_text_with(stream.as_bytes(), &custom)?;
    assert!(!out.contains("cargo-home"), "got {out}");
    let plain = ThirdPartyMatcher::new(None);
    let out = filter_text_with(stream.as_bytes(), &plain)?;
    assert!(
        out.contains("cargo-home"),
        "without the custom home it is kept: {out}"
    );
    Ok(())
}

#[test]
fn first_party_path_mentioning_registry_is_kept() -> TestResult {
    let stream = "\
error[E0432]: unresolved import `x`
 --> my-registry/src/lib.rs:3:1
note: also see
 --> crates/registry/src/x.rs:1:1
";
    let out = filter(stream)?;
    assert!(out.contains("my-registry/src/lib.rs:3:1"), "got {out}");
    assert!(out.contains("crates/registry/src/x.rs:1:1"), "got {out}");
    Ok(())
}

#[test]
fn warning_blocks_dropped_when_errors_exist() -> TestResult {
    let stream = format!("warning: unused variable: `y`\n --> src/main.rs:3:9\n\n{E0425}");
    let out = filter(&stream)?;
    assert!(out.contains("error[E0425]"), "got {out}");
    assert!(!out.contains("unused variable"), "got {out}");
    Ok(())
}

#[test]
fn warning_blocks_kept_when_no_errors() -> TestResult {
    let out = filter(
        "warning: unused variable: `y`\n --> src/main.rs:3:9\n  |\n3 |  let y = 1;\n  |      ^ help\n",
    )?;
    assert!(out.contains("warning: unused variable: `y`"), "got {out}");
    assert!(out.contains("--> src/main.rs:3:9"), "got {out}");
    Ok(())
}

#[test]
fn could_not_compile_line_not_a_block() -> TestResult {
    let only = "error: could not compile `demo` (bin \"demo\") due to 1 previous error\nerror: aborting due to 1 previous error\n";
    assert_eq!(filter(only)?.trim(), "");
    assert!(parse_diagnostics(only.as_bytes(), StreamFormat::Text)?.is_empty());
    let with_error = format!("{E0425}{only}");
    let out = filter(&with_error)?;
    assert!(out.contains("error[E0425]"), "got {out}");
    assert!(!out.contains("could not compile"), "got {out}");
    assert!(!out.contains("aborting due to"), "got {out}");
    Ok(())
}

#[test]
fn consecutive_duplicates_collapsed() -> TestResult {
    let stream = "\
error[E0308]: mismatched types
 --> src/main.rs:4:18
  = note: expected `u32`
  = note: expected `u32`
  = help: try again

error[E0308]: mismatched types
 --> src/main.rs:4:18
  = note: expected `u32`
  = note: expected `u32`
  = help: try again
";
    let out = filter(stream)?;
    assert_eq!(
        count(&out, "error[E0308]: mismatched types"),
        1,
        "got {out}"
    );
    assert_eq!(count(&out, "= note: expected `u32`"), 1, "got {out}");
    assert_eq!(count(&out, "= help: try again"), 1, "got {out}");
    Ok(())
}

#[test]
fn code_frames_dropped() -> TestResult {
    let out = filter(E0425)?;
    assert!(!out.contains('|'), "got {out}");
    assert!(!out.contains('^'), "got {out}");
    assert!(!out.contains("config.run"), "got {out}");
    assert!(out.contains("--> src/main.rs:2:5"), "got {out}");
    Ok(())
}

#[test]
fn download_and_compile_noise_dropped() -> TestResult {
    let stream = format!(
        "    Updating crates.io index\n Downloading crates ...\n  Downloaded foo v1.0.0 (registry `crates-io`)\n   Compiling foo v1.0.0\n{E0425}    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.2s\n"
    );
    let out = filter(&stream)?;
    for noise in [
        "Updating",
        "Downloading",
        "Downloaded",
        "Compiling",
        "Finished",
    ] {
        assert!(!out.contains(noise), "{noise} leaked: {out}");
    }
    assert!(out.contains("error[E0425]"), "got {out}");
    Ok(())
}

#[test]
fn note_continuation_lines_are_kept() -> TestResult {
    let stream = "\
error[E0308]: mismatched types
 --> src/main.rs:4:18
  |
  = note: expected struct `Vec<u8>`
             found struct `Vec<i8>`
";
    let out = filter(stream)?;
    assert!(out.contains("found struct `Vec<i8>`"), "got {out}");
    Ok(())
}

#[test]
fn escape_sequences_are_stripped_from_the_view() -> TestResult {
    let stream = "\u{1b}[1m\u{1b}[38;5;9merror[E0425]\u{1b}[0m: cannot find value `x`\n --> src/main.rs:2:5\n  = note: \u{1b}]0;pwned\u{7}be careful\n";
    let out = filter(stream)?;
    assert!(
        !out.contains('\u{1b}') && !out.contains('\u{7}'),
        "got {out:?}"
    );
    assert!(
        out.contains("error[E0425]: cannot find value `x`"),
        "got {out:?}"
    );
    assert!(out.contains("= note: be careful"), "got {out:?}");
    Ok(())
}

fn json_e0425() -> String {
    json!({
        "reason": "compiler-message",
        "message": {
            "message": "cannot find value `config` in this scope",
            "code": {"code": "E0425", "explanation": null},
            "level": "error",
            "spans": [{
                "file_name": "src/main.rs", "line_start": 2, "line_end": 2,
                "column_start": 5, "column_end": 11, "is_primary": true, "expansion": null
            }],
            "children": [{
                "message": "consider importing it", "code": null, "level": "help",
                "spans": [], "children": [], "rendered": null
            }],
            "rendered": "error[E0425]: ..."
        }
    })
    .to_string()
}

#[test]
fn json_stream_is_rendered_as_text() -> TestResult {
    let stream = format!("{}\n", json_e0425());
    let out = filter(&stream)?;
    assert!(
        out.contains("error[E0425]: cannot find value `config` in this scope"),
        "got {out}"
    );
    assert!(out.contains("--> src/main.rs:2:5"), "got {out}");
    assert!(out.contains("= help: consider importing it"), "got {out}");
    assert!(!out.contains("reason"), "got {out}");
    Ok(())
}

#[test]
fn text_parse_extracts_code_message_and_location() -> TestResult {
    let stream = "\
error[E0308]: mismatched types
 --> C:\\w\\demo\\src\\main.rs:10:5
  |
10|     let x: u32 = \"a\";
  |
  = note: expected `u32`
  = help: try this
";
    let diags = parse_diagnostics(stream.as_bytes(), StreamFormat::Text)?;
    let d = diags.first().ok_or("no diagnostic")?;
    assert_eq!(d.level, Level::Error);
    assert_eq!(d.code.as_deref(), Some("E0308"));
    assert_eq!(d.message, "mismatched types");
    assert_eq!(
        d.primary,
        Some(Location {
            file: "C:\\w\\demo\\src\\main.rs".to_owned(),
            line: 10,
            column: 5
        })
    );
    assert_eq!(
        d.details,
        vec![
            "note: expected `u32`".to_owned(),
            "help: try this".to_owned()
        ]
    );
    assert!(d.first_party);
    Ok(())
}

fn headlines(diags: &[Diagnostic]) -> Vec<String> {
    diags.iter().map(Diagnostic::headline).collect()
}

#[test]
fn json_and_text_streams_agree_on_headline() -> TestResult {
    let from_json = parse_diagnostics(json_e0425().as_bytes(), StreamFormat::CargoJson)?;
    let from_text = parse_diagnostics(E0425.as_bytes(), StreamFormat::Text)?;
    assert_eq!(headlines(&from_json), headlines(&from_text));
    let a = from_json.first().ok_or("json diagnostic missing")?;
    let b = from_text.first().ok_or("text diagnostic missing")?;
    assert_eq!(
        IssueId::from_message(&a.headline()),
        IssueId::from_message(&b.headline())
    );
    assert_eq!(a.primary, b.primary);
    Ok(())
}

#[test]
fn mixed_stream_merges_json_and_text_once() -> TestResult {
    let stream = format!("   Compiling demo v0.1.0\n{}\n{E0425}", json_e0425());
    let diags = parse_diagnostics(stream.as_bytes(), StreamFormat::Mixed)?;
    assert_eq!(diags.len(), 1, "got {diags:?}");
    let out = filter(&stream)?;
    assert_eq!(count(&out, "error[E0425]"), 1, "got {out}");
    Ok(())
}

fn input_line() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("error[E0425]: cannot find value `x` in this scope".to_owned()),
        Just("warning: unused variable: `y`".to_owned()),
        Just(" --> src/main.rs:2:5".to_owned()),
        Just(" --> C:\\Users\\x\\.cargo\\registry\\src\\a\\b.rs:1:1".to_owned()),
        Just("  |     ^^^^ not found".to_owned()),
        Just("  = note: expected `u32`".to_owned()),
        Just("  = help: try this".to_owned()),
        Just("note: something".to_owned()),
        Just("   Compiling demo v0.1.0".to_owned()),
        Just("error: could not compile `demo`".to_owned()),
        Just(String::new()),
        "[ -z]{0,40}",
    ]
}

proptest! {
    #[test]
    fn prop_filter_never_invents_text(lines in proptest::collection::vec(input_line(), 0..40)) {
        let input = lines.join("\n");
        let out = filter_text(input.as_bytes()).map_err(|e| TestCaseError::fail(e.to_string()))?;
        for out_line in out.lines().filter(|l| !l.is_empty()) {
            prop_assert!(
                lines.iter().any(|l| l.contains(out_line)),
                "invented line {:?}",
                out_line
            );
        }
    }

    #[test]
    fn prop_filter_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let out = filter_text(&bytes).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let has_escape = out.contains('\u{1b}');
        let has_cr = out.contains('\r');
        prop_assert!(!has_escape);
        prop_assert!(!has_cr);
    }
}
