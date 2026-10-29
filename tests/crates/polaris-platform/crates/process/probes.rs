//! Native read-only probe runner contracts: static argv, bounded output and deadline cleanup.

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::time::Duration;

use polaris_platform_process::{EnvPolicy, run_probe, run_probe_with_env};

const PROBE_MODE: &str = "POLARIS_PROBE_FIXTURE";

#[test]
fn run_probe_captures_version_without_a_shell() -> Result<(), Box<dyn std::error::Error>> {
    let program = std::env::current_exe()?;
    let cwd = program.parent().ok_or("test executable has no parent")?;
    let result = run_probe(
        &program,
        &[
            String::from("--exact"),
            String::from("probe_worker_fixture"),
            String::from("--nocapture"),
        ],
        cwd,
        Duration::from_secs(5),
        4096,
    )?;
    assert_eq!(result.exit_code, Some(0));
    assert!(!result.timed_out);
    assert!(!result.stdout.is_empty());
    assert!(!result.stdout_truncated);
    assert!(
        result
            .stdout
            .windows(b"RUSTUP_AUTO_INSTALL=0".len())
            .any(|window| window == b"RUSTUP_AUTO_INSTALL=0")
    );
    assert!(
        result
            .stdout
            .windows(b"CARGO_NET_OFFLINE=true".len())
            .any(|window| window == b"CARGO_NET_OFFLINE=true")
    );
    Ok(())
}

#[test]
fn probe_output_is_capped_while_pipes_continue_draining() -> Result<(), Box<dyn std::error::Error>>
{
    let program = std::env::current_exe()?;
    let cwd = program.parent().ok_or("test executable has no parent")?;
    let environment = fixture_environment("large-output")?;
    let result = run_probe_with_env(
        &program,
        &[
            String::from("--exact"),
            String::from("probe_worker_fixture"),
            String::from("--nocapture"),
        ],
        cwd,
        Duration::from_secs(10),
        128,
        &environment,
    )?;
    assert_eq!(result.exit_code, Some(0));
    assert!(!result.timed_out);
    assert!(result.stdout_truncated);
    assert!(result.stderr_truncated);
    assert!(result.stdout.len() <= 128);
    assert!(result.stderr.len() <= 128);
    Ok(())
}

#[test]
fn probe_deadline_kills_the_owned_child() -> Result<(), Box<dyn std::error::Error>> {
    let program = std::env::current_exe()?;
    let cwd = program.parent().ok_or("test executable has no parent")?;
    let environment = fixture_environment("sleep")?;
    let result = run_probe_with_env(
        &program,
        &[
            String::from("--exact"),
            String::from("probe_worker_fixture"),
            String::from("--nocapture"),
        ],
        cwd,
        Duration::from_millis(100),
        64,
        &environment,
    )?;
    assert!(result.timed_out);
    Ok(())
}

#[test]
fn probe_worker_fixture() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::var_os(PROBE_MODE).as_deref() {
        Some(value) if value == OsStr::new("large-output") => {
            let chunk = vec![b'X'; 1024 * 1024];
            std::io::stdout().lock().write_all(&chunk)?;
            std::io::stderr().lock().write_all(&chunk)?;
            Ok(())
        }
        Some(value) if value == OsStr::new("sleep") => {
            std::thread::sleep(Duration::from_secs(30));
            Ok(())
        }
        _ => {
            writeln!(
                std::io::stdout().lock(),
                "RUSTUP_AUTO_INSTALL={}",
                std::env::var("RUSTUP_AUTO_INSTALL").unwrap_or_default()
            )?;
            writeln!(
                std::io::stdout().lock(),
                "CARGO_NET_OFFLINE={}",
                std::env::var("CARGO_NET_OFFLINE").unwrap_or_default()
            )?;
            Ok(())
        }
    }
}

fn fixture_environment(mode: &str) -> Result<EnvPolicy, Box<dyn std::error::Error>> {
    Ok(EnvPolicy::new(vec![(
        OsString::from(PROBE_MODE),
        OsString::from(mode),
    )])?)
}
