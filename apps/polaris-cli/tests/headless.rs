//! Exercise the real application binary with isolated state.
use std::process::Command;

#[test]
fn doctor_installs_aws_lc_provider() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_polaris"))
        .args([
            "--state-dir",
            root.path().to_str().ok_or("path encoding")?,
            "doctor",
            "--json",
        ])
        .output()?;
    // Doctor exits non-zero while required host setup is missing, but always reports first.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() || stderr.contains("required setup prerequisites are missing"),
        "doctor failed unexpectedly: {stderr}"
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(
        report
            .get("crypto_provider")
            .and_then(serde_json::Value::as_str),
        Some("aws-lc-rs")
    );
    Ok(())
}

#[test]
fn run_commits_and_replays_directive() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_polaris"))
        .args([
            "--state-dir",
            root.path().to_str().ok_or("path encoding")?,
            "--workspace",
            workspace.path().to_str().ok_or("path encoding")?,
            "run",
            "$websearch(off)",
        ])
        .output()?;
    assert!(
        output.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::str::from_utf8(&output.stdout)?;
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert!(
        lines
            .iter()
            .any(|event| event.to_string().contains("directive_applied")),
        "expected a real committed directive event"
    );
    Ok(())
}

#[test]
fn unsupported_task_is_not_reported_as_done() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_polaris"))
        .args([
            "--state-dir",
            root.path().to_str().ok_or("path encoding")?,
            "run",
            "invent a model response",
        ])
        .output()?;
    assert!(
        !output.status.success(),
        "P1 cannot synthesize model completion"
    );
    Ok(())
}
