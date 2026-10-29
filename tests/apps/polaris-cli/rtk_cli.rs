//! Exercise RTK through the real `polaris` process and piped stdin.

use std::{
    io::Write,
    process::{Command, Output, Stdio},
};

fn run_rtk(
    state: &std::path::Path,
    workspace: &std::path::Path,
    mode: &str,
    input: &[u8],
) -> Result<Output, Box<dyn std::error::Error>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_polaris"))
        .args([
            "--state-dir",
            state.to_str().ok_or("state path is not UTF-8")?,
            "--workspace",
            workspace.to_str().ok_or("workspace path is not UTF-8")?,
            "rtk",
            "--mode",
            mode,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("stdin pipe was not created")?;
    stdin.write_all(input)?;
    drop(stdin);
    Ok(child.wait_with_output()?)
}

#[test]
fn real_cli_rtk_filters_and_closes_issues_from_piped_streams()
-> Result<(), Box<dyn std::error::Error>> {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let failing =
        b"error[E0425]: cannot find value `missing` in this scope\n --> src/main.rs:1:1\n";
    let first = run_rtk(state.path(), workspace.path(), "auto", failing)?;
    assert_eq!(
        first.status.code(),
        Some(1),
        "stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stderr).contains("1 open issue"));
    let ledger_path = workspace.path().join(".polaris").join("issues.md");
    let open_ledger = std::fs::read_to_string(&ledger_path)?;
    assert!(open_ledger.contains("- [ ] error[E0425]"));
    assert!(
        state.path().join("cas").exists(),
        "raw input should be stored before filtering"
    );

    let clean = run_rtk(
        state.path(),
        workspace.path(),
        "auto",
        b"Compiling demo v0.1.0\n",
    )?;
    assert!(
        clean.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&clean.stderr)
    );
    assert!(std::fs::read_to_string(ledger_path)?.starts_with("- [x] ~~error[E0425]"));
    Ok(())
}

#[test]
fn filter_mode_does_not_touch_the_issue_ledger() -> Result<(), Box<dyn std::error::Error>> {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let output = run_rtk(
        state.path(),
        workspace.path(),
        "filter",
        b"error[E0308]: mismatched types\n --> src/main.rs:1:1\n",
    )?;
    assert_eq!(
        output.status.code(),
        Some(1),
        "filter mode reports current-stream issues: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("error[E0308]: mismatched types"));
    assert!(!workspace.path().join(".polaris").join("issues.md").exists());
    Ok(())
}
