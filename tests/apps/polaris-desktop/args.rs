//! Desktop Main (descriptor: specs/descriptors/ui_desktop_main.json).

use std::error::Error;
use std::ffi::OsString;
use std::path::PathBuf;

use polaris_desktop::{DesktopArgs, DesktopError, parse_args};

type TestResult = Result<(), Box<dyn Error>>;

fn parse(args: &[&str], env: Option<&str>) -> Result<DesktopArgs, DesktopError> {
    parse_args(
        args.iter().map(OsString::from),
        env.map(OsString::from),
        PathBuf::from("C:/work"),
        Some(PathBuf::from("C:/Users/me/AppData/Local")),
    )
}

fn usage_message(result: Result<DesktopArgs, DesktopError>) -> Result<String, Box<dyn Error>> {
    match result {
        Err(DesktopError::Usage(message)) => Ok(message),
        Err(other) => Err(format!("expected a usage error, got {other}").into()),
        Ok(args) => Err(format!("expected a usage error, got {args:?}").into()),
    }
}

#[test]
fn defaults_use_cwd_and_local_app_data() -> TestResult {
    let args = parse(&[], None)?;
    assert_eq!(args.workspace, PathBuf::from("C:/work"));
    assert_eq!(
        args.log_dir,
        PathBuf::from("C:/Users/me/AppData/Local")
            .join("Polaris")
            .join("logs")
    );
    assert!(!args.reduced_motion, "motion is on by default");
    let no_app_data = parse_args(Vec::<OsString>::new(), None, PathBuf::from("C:/work"), None)?;
    assert_eq!(
        no_app_data.log_dir,
        PathBuf::from("C:/work").join(".polaris").join("logs")
    );
    Ok(())
}

#[test]
fn static_flag_and_env_enable_reduced_motion() -> TestResult {
    assert!(parse(&["--static"], None)?.reduced_motion, "--static");
    assert!(parse(&[], Some("1"))?.reduced_motion, "env 1");
    assert!(parse(&[], Some("yes"))?.reduced_motion, "env yes");
    assert!(!parse(&[], Some("0"))?.reduced_motion, "env 0");
    assert!(!parse(&[], Some(""))?.reduced_motion, "env empty");
    Ok(())
}

#[test]
fn workspace_and_log_dir_flags_override_defaults() -> TestResult {
    let args = parse(&["--workspace", "D:/repo", "--log-dir", "D:/logs"], None)?;
    assert_eq!(args.workspace, PathBuf::from("D:/repo"));
    assert_eq!(args.log_dir, PathBuf::from("D:/logs"));
    assert!(usage_message(parse(&["--workspace"], None))?.contains("--workspace needs"));
    assert!(usage_message(parse(&["--log-dir"], None))?.contains("--log-dir needs"));
    assert_eq!(
        parse(&[], None)?.state_dir,
        None,
        "state dir defaults to the workspace store"
    );
    let with_state = parse(&["--state-dir", "D:/state"], None)?;
    assert_eq!(
        with_state.state_dir,
        Some(PathBuf::from("D:/state")),
        "--state-dir"
    );
    assert!(usage_message(parse(&["--state-dir"], None))?.contains("--state-dir needs"));
    Ok(())
}

#[test]
fn connect_takes_a_daemon_session_and_credential_file() -> TestResult {
    let attached = parse(
        &[
            "--connect",
            "polaris-main",
            "--session",
            "42",
            "--token-file",
            "D:/polaris/client.token",
        ],
        None,
    )?;
    assert_eq!(
        attached.connect,
        Some(polaris_desktop::ConnectArgs {
            endpoint: String::from("polaris-main"),
            session: String::from("42"),
            token_file: PathBuf::from("D:/polaris/client.token"),
        })
    );
    assert_eq!(parse(&[], None)?.connect, None);
    assert!(usage_message(parse(&["--connect"], None))?.contains("--connect needs"));
    let partial = usage_message(parse(&["--connect", "polaris-main"], None))?;
    assert!(partial.contains("go together"), "{partial}");
    let orphan = usage_message(parse(&["--token-file", "t"], None))?;
    assert!(orphan.contains("go together"), "{orphan}");
    Ok(())
}

#[test]
fn unknown_flag_is_rejected() -> TestResult {
    let message = usage_message(parse(&["--fullscreen"], None))?;
    assert!(
        message.contains("unknown argument `--fullscreen`"),
        "{message}"
    );
    assert!(message.contains("usage: polaris-desktop"), "{message}");
    Ok(())
}
