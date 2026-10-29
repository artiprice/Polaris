//! Run pipeline (descriptor: specs/descriptors/rtk_run_pipeline.json).

use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::Command;

use jiff::Timestamp;
use polaris_storage_cas::{Cas, CasConfig, content_id};
use polaris_tools_rtk::{IssueId, LedgerFile, MAX_STREAM_BYTES, RtkError, RtkMode, run};
use serde_json::Value;

type TestResult = Result<(), Box<dyn Error>>;

const T1: &str = "2026-09-22T11:04:00Z";
const T2: &str = "2026-09-23T08:30:15Z";

const FAILING: &str = "\
   Compiling demo v0.1.0 (C:\\w\\demo)
error[E0425]: cannot find value `config` in this scope
 --> src/main.rs:2:5
  |
2 |     config.run();
  |     ^^^^^^ not found in this scope

error: could not compile `demo` (bin \"demo\") due to 1 previous error
";

const CLEAN: &str = "   Compiling demo v0.1.0 (C:\\w\\demo)\n    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.4s\n";

struct Bench {
    _dir: tempfile::TempDir,
    cas: Cas,
    file: LedgerFile,
}

fn bench() -> Result<Bench, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let cas = Cas::open(CasConfig::new(dir.path().join("cas")))?;
    let file = LedgerFile::new(dir.path().join(".polaris").join("issues.md"));
    Ok(Bench {
        _dir: dir,
        cas,
        file,
    })
}

fn at(text: &str) -> Result<Timestamp, jiff::Error> {
    text.parse()
}

#[test]
fn auto_reconciles_and_filters() -> TestResult {
    let b = bench()?;
    let out = run(RtkMode::Auto, FAILING.as_bytes(), &b.file, at(T1)?, &b.cas)?;
    assert!(
        out.filtered
            .contains("error[E0425]: cannot find value `config`")
    );
    assert!(!out.filtered.contains("could not compile"));
    let summary = out.summary.as_ref().ok_or("auto mode has a summary")?;
    assert_eq!((summary.newly_opened, summary.open.len()), (1, 1));
    assert_eq!(out.exit_code, 1);
    let ledger = fs::read_to_string(b.file.path())?;
    assert!(
        ledger.starts_with(
            "- [ ] error[E0425]: cannot find value `config` in this scope | ID: #ERR-"
        )
    );
    assert!(ledger.contains(&format!("Opened: {T1}")));

    let fixed = run(RtkMode::Auto, CLEAN.as_bytes(), &b.file, at(T2)?, &b.cas)?;
    assert_eq!(fixed.exit_code, 0);
    assert_eq!(fixed.summary.as_ref().ok_or("summary")?.newly_closed, 1);
    let ledger = fs::read_to_string(b.file.path())?;
    assert!(ledger.starts_with("- [x] ~~error[E0425]"), "got {ledger}");
    assert!(ledger.contains(&format!("Closed: {T2}")));
    Ok(())
}

#[test]
fn filter_mode_leaves_the_ledger_untouched() -> TestResult {
    let b = bench()?;
    let out = run(
        RtkMode::Filter,
        FAILING.as_bytes(),
        &b.file,
        at(T1)?,
        &b.cas,
    )?;
    assert!(out.filtered.contains("error[E0425]"));
    assert!(out.summary.is_none());
    assert!(
        !b.file.path().exists(),
        "filter mode must not create the ledger"
    );
    Ok(())
}

#[test]
fn json_mode_reports_and_leaves_the_ledger_untouched() -> TestResult {
    let b = bench()?;
    let out = run(RtkMode::Json, FAILING.as_bytes(), &b.file, at(T1)?, &b.cas)?;
    let report: Value = serde_json::from_str(&out.filtered)?;
    assert_eq!(report.get("status"), Some(&Value::from("FAILED")));
    assert_eq!(report.get("error_count"), Some(&Value::from(1)));
    assert!(out.summary.is_none());
    assert!(
        !b.file.path().exists(),
        "json mode must not create the ledger"
    );
    Ok(())
}

#[test]
fn issues_mode_prints_summary_only() -> TestResult {
    let b = bench()?;
    let out = run(
        RtkMode::Issues,
        FAILING.as_bytes(),
        &b.file,
        at(T1)?,
        &b.cas,
    )?;
    assert_eq!(out.filtered, "");
    assert!(out.summary.is_some());
    assert!(!out.view.is_empty());
    assert_eq!(out.exit_code, 1);
    assert!(b.file.path().exists());
    Ok(())
}

#[test]
fn raw_stream_is_stored_in_cas_first() -> TestResult {
    let b = bench()?;
    let out = run(RtkMode::Auto, FAILING.as_bytes(), &b.file, at(T1)?, &b.cas)?;
    assert_eq!(out.raw, content_id(FAILING.as_bytes()));
    let stored = b.cas.get(&out.raw)?;
    assert_eq!(stored.as_ref(), FAILING.as_bytes());
    Ok(())
}

#[test]
fn exit_code_reflects_ledger_not_build() -> TestResult {
    let b = bench()?;
    let id = IssueId::from_message("error: stale problem");
    fs::create_dir_all(b.file.path().parent().ok_or("no parent")?)?;
    fs::write(
        b.file.path(),
        format!("# notes\n- [ ] error: stale problem | ID: {id} | Opened: 2026-01-01T00:00:00Z\n"),
    )?;

    // A stream with only warnings (or nothing at all) closes the issue: exit 0.
    let warn_only = "warning: unused variable: `y`\n --> src/main.rs:3:9\n";
    let out = run(
        RtkMode::Auto,
        warn_only.as_bytes(),
        &b.file,
        at(T1)?,
        &b.cas,
    )?;
    assert_eq!(out.exit_code, 0);
    assert!(
        fs::read_to_string(b.file.path())?.starts_with("# notes\n- [x] ~~error: stale problem~~")
    );

    // A stream with an unrelated error: exit 1 even though the stale one stays closed.
    let out = run(RtkMode::Auto, FAILING.as_bytes(), &b.file, at(T2)?, &b.cas)?;
    assert_eq!(out.exit_code, 1);

    // An empty stream closes everything (the ledger only mirrors what was piped in).
    let out = run(RtkMode::Auto, b"", &b.file, at(T2)?, &b.cas)?;
    assert_eq!(out.exit_code, 0);
    Ok(())
}

#[test]
fn agent_view_carries_the_honesty_note_and_locations() -> TestResult {
    let b = bench()?;
    let out = run(RtkMode::Auto, FAILING.as_bytes(), &b.file, at(T1)?, &b.cas)?;
    let view = &out.view;
    assert!(view.contains("1 open"), "got {view}");
    assert!(view.contains("src/main.rs:2"), "got {view}");
    assert!(
        view.contains("error[E0425]: cannot find value `config`"),
        "got {view}"
    );
    assert!(view.contains("last stream piped in"), "got {view}");

    let clean = run(RtkMode::Auto, CLEAN.as_bytes(), &b.file, at(T2)?, &b.cas)?;
    assert!(clean.view.contains("0 open"), "got {}", clean.view);
    assert!(clean.view.contains("last stream piped in"));
    Ok(())
}

#[test]
fn oversized_stream_is_refused_before_storing() -> TestResult {
    let b = bench()?;
    let huge = vec![0u8; MAX_STREAM_BYTES + 1];
    let err = run(RtkMode::Auto, &huge, &b.file, at(T1)?, &b.cas)
        .err()
        .ok_or("expected an error")?;
    assert!(
        matches!(err, RtkError::StreamTooLarge { .. }),
        "got {err:?}"
    );
    assert!(!b.file.path().exists());
    assert!(!b.cas.contains(&content_id(&huge))?);
    Ok(())
}

const BROKEN_MAIN: &str = "fn main() {\n    let x: u32 = \"a\";\n    println!(\"{x}\");\n}\n";
const FIXED_MAIN: &str = "fn main() {\n    let x: u32 = 1;\n    println!(\"{x}\");\n}\n";

/// Runs `cargo build` in the fixture crate and returns stdout followed by stderr.
fn cargo_build(root: &Path, format: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let output = Command::new(cargo)
        .current_dir(root)
        .args([
            "build",
            "--offline",
            "--color",
            "never",
            "--message-format",
            format,
        ])
        .env("CARGO_TARGET_DIR", root.join("target"))
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .output()?;
    let mut stream = output.stdout;
    stream.extend_from_slice(&output.stderr);
    Ok(stream)
}

#[test]
fn real_cargo_failure_round_trip() -> TestResult {
    let crate_dir = tempfile::tempdir()?;
    let root = crate_dir.path();
    fs::create_dir(root.join("src"))?;
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )?;
    fs::write(root.join("src").join("main.rs"), BROKEN_MAIN)?;

    let b = bench()?;
    let json_stream = cargo_build(root, "json")?;
    let out = run(RtkMode::Auto, &json_stream, &b.file, at(T1)?, &b.cas)?;
    assert_eq!(out.exit_code, 1, "filtered view was: {}", out.filtered);
    let ledger = fs::read_to_string(b.file.path())?;
    assert!(
        ledger.contains("error[E0308]: mismatched types"),
        "ledger: {ledger}"
    );
    assert_eq!(ledger.lines().count(), 1, "ledger: {ledger}");

    // The human-readable stream of the same failure yields the same issue id.
    let human_stream = cargo_build(root, "human")?;
    let b2 = bench()?;
    let human = run(RtkMode::Auto, &human_stream, &b2.file, at(T1)?, &b2.cas)?;
    assert_eq!(human.exit_code, 1, "filtered view was: {}", human.filtered);
    assert_eq!(
        fs::read_to_string(b2.file.path())?
            .lines()
            .map(|l| l.split(" | Opened").next().unwrap_or(l).to_owned())
            .collect::<Vec<_>>(),
        ledger
            .lines()
            .map(|l| l.split(" | Opened").next().unwrap_or(l).to_owned())
            .collect::<Vec<_>>(),
        "JSON and human streams must open the same issue"
    );

    // Fix the code and rerun: the issue is closed with a timestamp, exit 0.
    fs::write(root.join("src").join("main.rs"), FIXED_MAIN)?;
    let fixed_stream = cargo_build(root, "json")?;
    let fixed = run(RtkMode::Auto, &fixed_stream, &b.file, at(T2)?, &b.cas)?;
    assert_eq!(fixed.exit_code, 0);
    let ledger = fs::read_to_string(b.file.path())?;
    assert!(
        ledger.starts_with("- [x] ~~error[E0308]: mismatched types~~"),
        "ledger: {ledger}"
    );
    assert!(
        ledger.contains(&format!("Closed: {T2}")),
        "ledger: {ledger}"
    );
    Ok(())
}
