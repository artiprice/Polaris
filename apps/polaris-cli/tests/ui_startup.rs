//! Real-binary UI startup and deterministic dump boundaries.
use clap::Parser;
use std::process::Command;

#[test]
fn default_arguments_select_interactive_mode() {
    assert!(
        polaris_cli::Args::try_parse_from(["polaris"]).is_ok(),
        "default invocation must select the interactive branch"
    );
}

#[test]
fn dump_is_deterministic_and_has_no_state_side_effects() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let state = temp.path().join("must-not-exist");
    let invoke = || {
        Command::new(env!("CARGO_BIN_EXE_polaris"))
            .arg("--state-dir")
            .arg(&state)
            .args(["--dump", "80x24"])
            .output()
    };
    let first = invoke()?;
    let second = invoke()?;
    assert!(
        first.status.success(),
        "dump failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(second.status.success(), "second dump failed");
    assert_eq!(first.stdout, second.stdout, "dump must be deterministic");
    assert!(
        !first.stdout.contains(&27),
        "plain dump contains escape bytes"
    );
    assert!(
        String::from_utf8(first.stdout)?.contains("Polaris"),
        "same renderer must paint the app header"
    );
    assert!(
        !state.exists(),
        "dump must not initialize a live backend or logging"
    );
    Ok(())
}

#[test]
fn non_tty_default_fails_without_terminal_controls() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let state = temp.path().join("must-not-exist");
    let out = Command::new(env!("CARGO_BIN_EXE_polaris"))
        .arg("--state-dir")
        .arg(&state)
        .output()?;
    assert!(!out.status.success(), "non-TTY default must fail");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("interactive terminal"),
        "actionable error missing: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.stdout.contains(&27) && !out.stderr.contains(&27),
        "non-TTY error must not emit terminal controls"
    );
    assert!(
        !state.exists(),
        "terminal validation precedes backend startup"
    );
    Ok(())
}

#[test]
fn invalid_dump_and_scenario_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
    for args in [
        vec!["--dump", "0x20"],
        vec!["--dump", "401x200"],
        vec!["--dump", "80x201"],
        vec!["--dump", "not-a-size"],
        vec!["--scenario", "welcome"],
        vec!["--dump", "80x24", "--scenario", "not-a-scenario"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_polaris"))
            .args(args)
            .output()?;
        assert!(
            !out.status.success(),
            "invalid UI options unexpectedly succeeded"
        );
        assert!(
            !out.stdout.contains(&27),
            "failed dump emitted terminal controls"
        );
    }
    Ok(())
}

#[test]
fn initial_prompt_is_prefilled_and_not_executed() -> Result<(), Box<dyn std::error::Error>> {
    let out = Command::new(env!("CARGO_BIN_EXE_polaris"))
        .args(["--dump", "80x24", "keep this draft"])
        .output()?;
    assert!(
        out.status.success(),
        "prefill dump failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8(out.stdout)?.contains("keep this draft"),
        "draft missing"
    );
    Ok(())
}
