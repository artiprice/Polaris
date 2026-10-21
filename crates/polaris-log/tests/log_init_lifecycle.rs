//! Log Init Lifecycle (descriptor: specs/descriptors/log_log_init_lifecycle.json).
//!
//! `init` installs a process-wide subscriber, so every test that needs it runs in a child process
//! (this same test binary re-invoked with an environment variable naming the scenario). The
//! in-process tests use `start`, which never touches global state.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

use polaris_log::{LOG_FILE_NAME, LogConfig, LogError, init, start};
use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::registry::{RedactStats, Redactor};

type TestResult = Result<(), Box<dyn Error>>;
type Mutation = Box<dyn Fn(&mut LogConfig)>;

const SCENARIO_ENV: &str = "POLARIS_LOG_TEST_SCENARIO";
const DIR_ENV: &str = "POLARIS_LOG_TEST_DIR";
const FLUSH: Duration = Duration::from_secs(20);

struct Passthrough;

impl Redactor for Passthrough {
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
        out.extend_from_slice(input);
        RedactStats {
            bytes_in: input.len(),
            replacements: 0,
        }
    }
}

fn config(dir: &Path, tui_mode: bool) -> LogConfig {
    let mut cfg = LogConfig::new(dir);
    cfg.level = "info".to_string();
    cfg.tui_mode = tui_mode;
    cfg
}

fn run_child(scenario: &str, dir: &Path) -> Result<Output, Box<dyn Error>> {
    let exe = std::env::current_exe()?;
    let out = Command::new(exe)
        .args([
            "--exact",
            "child_scenario",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(SCENARIO_ENV, scenario)
        .env(DIR_ENV, dir)
        .output()?;
    Ok(out)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn expect_success(out: &Output) -> TestResult {
    if out.status.success() {
        return Ok(());
    }
    Err(format!(
        "child failed: {}\nstdout:\n{}\nstderr:\n{}",
        out.status,
        text(&out.stdout),
        text(&out.stderr)
    )
    .into())
}

/// Child side of the process-level scenarios. Run directly (without the environment variable) it
/// does nothing; the parent tests below assert on the child's exit status and output.
#[test]
fn child_scenario() -> TestResult {
    let Some(scenario) = std::env::var_os(SCENARIO_ENV) else {
        return Ok(());
    };
    let dir = std::path::PathBuf::from(std::env::var_os(DIR_ENV).ok_or("no dir")?);
    match scenario.to_str() {
        Some("double_init") => {
            let first = dir.join("first");
            let second = dir.join("second");
            let guard = init(config(&first, true), Passthrough)?;
            let again = init(config(&second, true), Passthrough);
            if !matches!(again, Err(LogError::AlreadyInitialised)) {
                return Err("second init was not AlreadyInitialised".into());
            }
            if second.exists() {
                return Err("failed second init created its directory".into());
            }
            tracing::info!("global subscriber is live");
            if !guard.flush(FLUSH) {
                return Err("flush timed out".into());
            }
            let written = fs::read_to_string(first.join(LOG_FILE_NAME))?;
            if !written.contains("global subscriber is live") {
                return Err(format!("global event missing from file: {written:?}").into());
            }
            // Dropping the guard does not allow a re-install: the global slot stays taken.
            drop(guard);
            let third = init(config(&dir.join("third"), true), Passthrough);
            if !matches!(third, Err(LogError::AlreadyInitialised)) {
                return Err("init after guard drop was not AlreadyInitialised".into());
            }
            Ok(())
        }
        Some(mode @ ("quiet" | "console")) => {
            let guard = init(config(&dir, mode == "quiet"), Passthrough)?;
            tracing::info!("MARKER-{mode}-line");
            tracing::error!("MARKER-{mode}-error");
            drop(guard);
            Ok(())
        }
        _ => Err("unknown scenario".into()),
    }
}

#[test]
fn second_init_is_error() -> TestResult {
    let dir = tempfile::tempdir()?;
    expect_success(&run_child("double_init", dir.path())?)
}

#[test]
fn tui_mode_never_writes_stdout() -> TestResult {
    let dir = tempfile::tempdir()?;
    let out = run_child("quiet", dir.path())?;
    expect_success(&out)?;
    assert!(!text(&out.stdout).contains("MARKER-quiet"), "stdout leaked");
    assert!(!text(&out.stderr).contains("MARKER-quiet"), "stderr leaked");
    // The file sink still received both records.
    let file = fs::read_to_string(dir.path().join(LOG_FILE_NAME))?;
    assert!(file.contains("MARKER-quiet-line") && file.contains("MARKER-quiet-error"));
    Ok(())
}

#[test]
fn console_mode_writes_stderr_only() -> TestResult {
    // Control for the test above: the same probe does see output when the console is enabled.
    let dir = tempfile::tempdir()?;
    let out = run_child("console", dir.path())?;
    expect_success(&out)?;
    assert!(text(&out.stderr).contains("MARKER-console-line"));
    assert!(
        !text(&out.stdout).contains("MARKER-console"),
        "stdout must stay clean"
    );
    Ok(())
}

#[test]
fn bad_filter_is_error() -> TestResult {
    let dir = tempfile::tempdir()?;
    let logs = dir.path().join("logs");
    let mut cfg = config(&logs, true);
    cfg.level = "polaris=loud".to_string();
    assert!(matches!(
        start(cfg, Passthrough).map(|_| ()),
        Err(LogError::BadFilter(_))
    ));
    assert!(!logs.exists(), "a rejected config must not touch the disk");
    Ok(())
}

#[test]
fn bad_config_is_error() -> TestResult {
    let dir = tempfile::tempdir()?;
    let bad: Vec<(&str, Mutation)> = vec![
        ("ring_capacity", Box::new(|c| c.ring_capacity = 0)),
        ("rotate_bytes", Box::new(|c| c.rotate_bytes = 0)),
        ("keep_files", Box::new(|c| c.keep_files = 0)),
        ("queue_capacity", Box::new(|c| c.queue_capacity = 0)),
        ("max_line_bytes", Box::new(|c| c.max_line_bytes = 10)),
        ("dir", Box::new(|c| c.dir = std::path::PathBuf::new())),
    ];
    for (name, mutate) in &bad {
        let mut cfg = config(&dir.path().join("logs"), true);
        mutate(&mut cfg);
        let result = start(cfg, Passthrough).map(|_| ());
        assert!(
            matches!(result, Err(LogError::BadConfig(_))),
            "{name}: {result:?}"
        );
    }
    Ok(())
}

#[test]
fn guard_drop_flushes() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (guard, dispatch) = start(config(dir.path(), true), Passthrough)?;
    tracing::dispatcher::with_default(&dispatch, || {
        for i in 0..50 {
            tracing::info!("pending-{i}");
        }
    });
    drop(guard);
    let file = fs::read_to_string(dir.path().join(LOG_FILE_NAME))?;
    assert_eq!(file.lines().count(), 50);
    assert!(file.contains("pending-49"));
    Ok(())
}

#[test]
fn guard_flush_makes_lines_visible_before_drop() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (guard, dispatch) = start(config(dir.path(), true), Passthrough)?;
    tracing::dispatcher::with_default(&dispatch, || tracing::warn!("visible-now"));
    assert!(guard.flush(FLUSH));
    let file = fs::read_to_string(dir.path().join(LOG_FILE_NAME))?;
    assert!(file.contains("visible-now"));
    Ok(())
}

#[test]
fn rings_see_records_without_any_flush() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (guard, dispatch) = start(config(dir.path(), true), Passthrough)?;
    tracing::dispatcher::with_default(&dispatch, || tracing::info!(k = 1, "sync"));
    let tui = guard.tui_ring().snapshot();
    let tele = guard.telemetry_ring().snapshot();
    assert_eq!(tui.len(), 1);
    assert_eq!(tele.len(), 1);
    assert_eq!(tui.first().map(|l| l.message.as_str()), Some("sync k=1"));
    Ok(())
}

#[test]
fn start_does_not_touch_global_state() -> TestResult {
    let dir = tempfile::tempdir()?;
    let first = start(config(&dir.path().join("a"), true), Passthrough)?;
    let second = start(config(&dir.path().join("b"), true), Passthrough)?;
    // A fresh thread has no scoped default, so it sees the global one: none must be installed.
    let global_is_unset = std::thread::spawn(|| {
        tracing::dispatcher::get_default(|d| d.is::<tracing::subscriber::NoSubscriber>())
    })
    .join()
    .map_err(|_| "probe thread panicked")?;
    assert!(global_is_unset);
    drop((first, second));
    Ok(())
}

#[test]
fn level_filter_applies() -> TestResult {
    let dir = tempfile::tempdir()?;
    let mut cfg = config(dir.path(), true);
    cfg.level = "warn".to_string();
    let (guard, dispatch) = start(cfg, Passthrough)?;
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!("hidden");
        tracing::warn!("shown");
    });
    let lines = guard.tui_ring().snapshot();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines.first().map(|l| l.message.as_str()), Some("shown"));
    Ok(())
}

#[test]
fn errors_classify_through_has_kind() {
    assert_eq!(LogError::AlreadyInitialised.kind(), ErrorKind::Permanent);
    assert_eq!(
        LogError::BadFilter(String::new()).kind(),
        ErrorKind::Permanent
    );
    assert_eq!(
        LogError::BadConfig(String::new()).kind(),
        ErrorKind::Permanent
    );
    assert_eq!(
        LogError::SinkInit(String::new()).kind(),
        ErrorKind::Transient
    );
    assert_eq!(
        LogError::Io(std::io::Error::other("x")).kind(),
        ErrorKind::Transient
    );
}
