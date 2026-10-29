// Unit tests of apps/polaris-cli/src/app.rs (`tests`); compiled into that module via #[path].
use super::{Action, Args};
use clap::Parser;

#[test]
fn status_json_is_a_headless_cli_action() -> Result<(), clap::Error> {
    let args = Args::try_parse_from(["polaris", "status", "--json"])?;
    assert!(
        matches!(args.command, Some(Action::Status { json: true })),
        "status --json must route to the headless status action"
    );
    Ok(())
}

#[test]
fn task_command_accepts_explicit_profile_and_goal() -> Result<(), clap::Error> {
    let args = Args::try_parse_from([
        "polaris",
        "task",
        "--profile",
        "local.toml",
        "inspect the current workspace",
    ])?;
    assert!(
        matches!(
            args.command,
            Some(Action::Task { ref profile, ref task })
                if profile.as_deref() == Some(std::path::Path::new("local.toml"))
                    && task == "inspect the current workspace"
        ),
        "task must parse as its dedicated non-UI action"
    );
    let hosted = Args::try_parse_from([
        "polaris",
        "--env-file",
        ".env",
        "task",
        "summarize the workspace",
    ])?;
    assert!(
        matches!(hosted.command, Some(Action::Task { profile: None, .. })),
        "without --profile the provider chain serves the task"
    );
    assert_eq!(
        hosted.env_file.as_deref(),
        Some(std::path::Path::new(".env"))
    );
    Ok(())
}
