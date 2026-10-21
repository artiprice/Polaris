//! Cargo JSON diagnostics (descriptor: specs/descriptors/rtk_cargo_json_diagnostics.json).

use std::error::Error;

use polaris_tools_rtk::{
    Diagnostic, Level, Location, MAX_EXPANSION_DEPTH, MAX_STREAM_BYTES, RtkError, StreamFormat,
    parse_diagnostics, render_json_report,
};
use proptest::prelude::*;
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn Error>>;

fn span(file: &str, line: u32, column: u32, primary: bool) -> Value {
    json!({
        "file_name": file,
        "line_start": line,
        "line_end": line,
        "column_start": column,
        "column_end": column + 3,
        "is_primary": primary,
        "expansion": null
    })
}

fn compiler_message(
    level: &str,
    code: Option<&str>,
    message: &str,
    spans: Value,
    children: Value,
) -> String {
    json!({
        "reason": "compiler-message",
        "package_id": "demo 0.1.0 (path+file:///w/demo)",
        "manifest_path": "C:\\w\\demo\\Cargo.toml",
        "message": {
            "message": message,
            "code": code.map(|c| json!({"code": c, "explanation": null})),
            "level": level,
            "spans": spans,
            "children": children,
            "rendered": format!("{level}: {message}\n")
        }
    })
    .to_string()
}

fn e0425() -> String {
    compiler_message(
        "error",
        Some("E0425"),
        "cannot find value `config` in this scope",
        json!([
            span("src/lib.rs", 9, 1, false),
            span("src/main.rs", 2, 5, true)
        ]),
        json!([{
            "message": "consider importing it",
            "code": null,
            "level": "help",
            "spans": [],
            "children": [],
            "rendered": null
        }]),
    )
}

fn parse(stream: &str, fmt: StreamFormat) -> Result<Vec<Diagnostic>, RtkError> {
    parse_diagnostics(stream.as_bytes(), fmt)
}

#[test]
fn error_with_code_and_primary_span() -> TestResult {
    let diags = parse(&e0425(), StreamFormat::CargoJson)?;
    assert_eq!(diags.len(), 1);
    let d = diags.first().ok_or("no diagnostic")?;
    assert_eq!(d.level, Level::Error);
    assert_eq!(d.code.as_deref(), Some("E0425"));
    assert_eq!(d.message, "cannot find value `config` in this scope");
    assert_eq!(
        d.primary,
        Some(Location {
            file: "src/main.rs".to_owned(),
            line: 2,
            column: 5
        })
    );
    assert_eq!(d.details, vec!["help: consider importing it".to_owned()]);
    assert!(d.first_party);
    assert!(d.is_issue());
    assert_eq!(
        d.headline(),
        "error[E0425]: cannot find value `config` in this scope"
    );
    Ok(())
}

#[test]
fn macro_expansion_resolves_to_call_site() -> TestResult {
    let mut inner = span("/rustc/abc/library/core/src/macros/mod.rs", 41, 9, true);
    inner["expansion"] = json!({
        "span": span("src/main.rs", 7, 5, false),
        "macro_decl_name": "assert_eq!",
        "def_site_span": null
    });
    let line = compiler_message("error", None, "assertion failed", json!([inner]), json!([]));
    let diags = parse(&line, StreamFormat::CargoJson)?;
    let d = diags.first().ok_or("no diagnostic")?;
    let loc = d.primary.as_ref().ok_or("no location")?;
    assert_eq!(
        (loc.file.as_str(), loc.line, loc.column),
        ("src/main.rs", 7, 5)
    );
    Ok(())
}

#[test]
fn expansion_chain_is_capped() -> TestResult {
    let hops = MAX_EXPANSION_DEPTH + 8;
    let mut chain = span(&format!("f{hops}.rs"), 1, 1, true);
    for i in (0..hops).rev() {
        let mut outer = span(&format!("f{i}.rs"), 1, 1, true);
        outer["expansion"] = json!({ "span": chain });
        chain = outer;
    }
    let line = compiler_message("error", None, "deep", json!([chain]), json!([]));
    let diags = parse(&line, StreamFormat::CargoJson)?;
    let d = diags.first().ok_or("no diagnostic")?;
    let loc = d.primary.as_ref().ok_or("no location")?;
    assert_eq!(loc.file, format!("f{MAX_EXPANSION_DEPTH}.rs"));
    Ok(())
}

#[test]
fn warnings_are_not_issues() -> TestResult {
    let line = compiler_message(
        "warning",
        Some("unused_variables"),
        "unused variable: `y`",
        json!([span("src/main.rs", 3, 9, true)]),
        json!([]),
    );
    let diags = parse(&line, StreamFormat::CargoJson)?;
    let d = diags.first().ok_or("no diagnostic")?;
    assert_eq!(d.level, Level::Warning);
    assert!(!d.is_issue());
    Ok(())
}

#[test]
fn non_json_lines_are_skipped() -> TestResult {
    let other = compiler_message("error", None, "second", json!([]), json!([]));
    let stream = format!(
        "   Compiling demo v0.1.0\n{}\nnot json {{\n{{\"truncated\":\n{other}\n",
        e0425()
    );
    let diags = parse(&stream, StreamFormat::CargoJson)?;
    assert_eq!(diags.len(), 2);
    Ok(())
}

#[test]
fn deeply_nested_json_is_rejected_not_overflowed() -> TestResult {
    let open = format!(
        r#"{{"reason":"compiler-message","message":{{"message":"m","level":"error","spans":{}"#,
        "[".repeat(200_000)
    );
    let closed = format!(
        r#"{{"reason":"compiler-message","extra":{}{}}}"#,
        "[".repeat(100_000),
        "]".repeat(100_000)
    );
    let stream = format!("{open}\n{closed}\n{}\n", e0425());
    let handle = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            parse_diagnostics(stream.as_bytes(), StreamFormat::CargoJson).map_err(|e| e.to_string())
        })?;
    let diags = handle
        .join()
        .map_err(|_| "parser thread panicked or overflowed")??;
    assert_eq!(diags.len(), 1, "only the sane line survives");
    Ok(())
}

#[test]
fn aborting_summary_is_dropped() -> TestResult {
    let stream = format!(
        "{}\n{}\n",
        e0425(),
        compiler_message(
            "error",
            None,
            "aborting due to 2 previous errors",
            json!([]),
            json!([])
        )
    );
    let diags = parse(&stream, StreamFormat::CargoJson)?;
    assert_eq!(diags.len(), 1);
    let stream = compiler_message(
        "error",
        None,
        "aborting due to previous error",
        json!([]),
        json!([]),
    );
    assert!(parse(&stream, StreamFormat::CargoJson)?.is_empty());
    Ok(())
}

#[test]
fn third_party_span_is_flagged_but_still_an_issue() -> TestResult {
    let dep = "C:\\Users\\x\\.cargo\\registry\\src\\index.crates.io-1949cf8c6b5b557f\\dep-1.0.0\\src\\lib.rs";
    let line = compiler_message(
        "error",
        Some("E0599"),
        "no method named `go` found",
        json!([span(dep, 12, 4, true)]),
        json!([]),
    );
    let diags = parse(&line, StreamFormat::CargoJson)?;
    let d = diags.first().ok_or("no diagnostic")?;
    assert!(!d.first_party);
    assert!(d.is_issue(), "a broken dependency still breaks the build");
    Ok(())
}

#[test]
fn duplicate_diagnostics_collapse() -> TestResult {
    let stream = format!("{}\n{}\n{}\n", e0425(), e0425(), e0425());
    assert_eq!(parse(&stream, StreamFormat::CargoJson)?.len(), 1);
    Ok(())
}

#[test]
fn json_report_lists_errors_only() -> TestResult {
    let warning = compiler_message("warning", None, "unused import", json!([]), json!([]));
    let stream = format!("{warning}\n{}\n", e0425());
    let diags = parse(&stream, StreamFormat::CargoJson)?;
    let report = render_json_report(&diags)?;
    insta::assert_snapshot!(report, @r#"
    {
      "status": "FAILED",
      "error_count": 1,
      "errors": [
        {
          "code": "E0425",
          "message": "cannot find value `config` in this scope",
          "location": {
            "file": "src/main.rs",
            "line": 2,
            "column": 5
          },
          "details": [
            "help: consider importing it"
          ]
        }
      ]
    }
    "#);
    Ok(())
}

#[test]
fn json_report_for_no_errors_is_success() -> TestResult {
    let report: Value = serde_json::from_str(&render_json_report(&[])?)?;
    assert_eq!(report.get("status"), Some(&json!("SUCCESS")));
    assert_eq!(report.get("error_count"), Some(&json!(0)));
    assert_eq!(report.get("errors"), Some(&json!([])));
    Ok(())
}

#[test]
fn oversized_stream_is_refused() -> TestResult {
    let stream = vec![0u8; MAX_STREAM_BYTES + 1];
    let err = parse_diagnostics(&stream, StreamFormat::Text)
        .err()
        .ok_or("expected an error")?;
    assert!(
        matches!(err, RtkError::StreamTooLarge { .. }),
        "got {err:?}"
    );
    Ok(())
}

proptest! {
    #[test]
    fn prop_parse_never_panics_on_arbitrary_lines(
        text in ".{0,300}",
        with_prefix in any::<bool>(),
    ) {
        let text = if with_prefix {
            format!("{{\"reason\":\"compiler-message\",\"message\":{text}")
        } else {
            text
        };
        for fmt in [StreamFormat::CargoJson, StreamFormat::Text, StreamFormat::Mixed] {
            prop_assert!(parse_diagnostics(text.as_bytes(), fmt).is_ok());
        }
    }
}
