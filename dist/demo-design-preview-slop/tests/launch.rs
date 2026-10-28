use std::process::{Command, Output};

fn launch(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_polaris"))
        .env(
            "POLARIS_CONFIG_DIR",
            std::env::current_dir()
                .unwrap()
                .join("target")
                .join(format!("launch-config-{}", std::process::id())),
        )
        .args(args)
        .output()
        .expect("preview binary should start")
}

#[test]
fn preview_from_a_command_runner_renders_a_labeled_static_screen() {
    let output = launch(&["--preview"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stdout.contains("Polaris v0.1.0alpha"));
    assert!(!stdout.contains("UI PREVIEW"));
    assert!(stdout.contains("Nemotron Super"));
    assert!(!stdout.contains("Describe a coding task"));
    assert!(stdout.contains("[ auto mode on ]"));
    assert!(
        !stdout.contains('\u{1b}'),
        "piped output must not contain terminal escapes"
    );
    assert!(stderr.contains("static 80x24 preview"));
    assert!(stderr.contains("keyboard input is unavailable"));
}

#[test]
fn fallback_keeps_the_requested_scenario_and_ascii_mode() {
    let output = launch(&[
        "--preview",
        "--scenario",
        "usage-demo",
        "--ascii",
        "--color",
        "never",
    ]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("DEMO DATA"));
    assert!(stdout.is_ascii());
    assert!(!stdout.contains('\u{1b}'));
}

#[test]
fn explicit_dump_does_not_emit_the_automatic_fallback_notice() {
    let output = launch(&["--scenario", "settings", "--dump", "60x20"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Settings"));
    assert_eq!(stdout.lines().count(), 20);
    assert!(output.stderr.is_empty());
}

#[test]
fn explicitly_interactive_mode_rejects_pipes_with_an_actionable_message() {
    let output = launch(&["--preview", "--interactive"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("--interactive requires terminal input and output"));
    assert!(stderr.contains("static preview"));
}

#[test]
fn contradictory_launch_modes_are_reported_as_invalid_arguments() {
    let output = launch(&["--interactive", "--dump", "80x24"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be combined"));
}

#[test]
fn invalid_arguments_still_fail_instead_of_becoming_successful_static_previews() {
    for args in [
        &["--scenario", "missing"][..],
        &["--dump", "0x24"][..],
        &["--unknown"][..],
    ] {
        let output = launch(args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn account_email_is_a_local_header_label_and_does_not_claim_a_connection() {
    let output = launch(&[
        "--account-email",
        "display-only@example.invalid",
        "--scenario",
        "status",
        "--dump",
        "100x35",
    ]);
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("display-only@example.invalid"));
    assert!(text.contains("Not connected"));
    assert!(text.contains("Provider is not configured"));
}

#[test]
fn key_trace_is_opt_in_creates_a_new_file_and_never_overwrites_existing_data() {
    let path = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!("key-trace-launch-{}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let output = launch(&["--key-trace", path.to_str().unwrap()]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires terminal input and output"));
    let trace = std::fs::read_to_string(&path).unwrap();
    assert!(trace.contains("\"event\":\"bootstrap\""));
    assert!(trace.contains("\"stdin_tty\":false"));
    assert!(trace.contains("\"stdout_tty\":false"));
    assert!(!trace.contains("\"event\":\"start\""));
    let second = launch(&["--key-trace", path.to_str().unwrap()]);
    assert!(!second.status.success());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), trace);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn key_trace_requires_a_path_and_cannot_be_silently_ignored_in_dump_mode() {
    for args in [
        &["--key-trace"][..],
        &["--key-trace", "unused.jsonl", "--dump", "80x24"][..],
    ] {
        let output = launch(args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!output.stderr.is_empty());
    }
}

#[test]
fn color_depth_help_and_values_are_explicit_and_invalid_values_fail() {
    let help = launch(&["--help"]);
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("--color-depth auto|truecolor|ansi")
    );
    for depth in ["auto", "truecolor", "ansi"] {
        let out = launch(&["--color-depth", depth, "--dump", "80x24"]);
        assert!(out.status.success(), "{depth}: {:?}", out.stderr);
        assert!(!out.stdout.contains(&27));
    }
    for args in [
        &["--color-depth"][..],
        &["--color-depth", "256"][..],
        &["--color-depth", "always"][..],
    ] {
        let out = launch(args);
        assert!(!out.status.success());
        assert!(
            String::from_utf8(out.stderr)
                .unwrap()
                .contains("--color-depth requires auto, truecolor, or ansi")
        );
    }
}
