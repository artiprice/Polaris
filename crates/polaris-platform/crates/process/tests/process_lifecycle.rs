//! Real native process-tree contract tests using the integration test binary as a worker.

use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::time::Duration;

use polaris_platform_process::ProcessPlatform;
use polaris_sdk::registry::{PlatformOps, SpawnSpec, StopOutcome};
use sysinfo::{Pid, ProcessesToUpdate, System};

type TestResult = Result<(), Box<dyn Error>>;

const ROLE: &str = "POLARIS_TREE_FIXTURE_ROLE";
const PID_FILE: &str = "POLARIS_TREE_FIXTURE_PID_FILE";

#[tokio::test]
async fn native_process_tree_stops_descendants_and_scrubs_inherited_environment() -> TestResult {
    let directory = tempfile::tempdir()?;
    let pid_path = directory.path().join("grandchild.pid");
    let spec = SpawnSpec {
        program: std::env::current_exe()?,
        args: vec![
            OsString::from("--exact"),
            OsString::from("worker_fixture"),
            OsString::from("--nocapture"),
        ],
        cwd: None,
        env: vec![
            (OsString::from(ROLE), OsString::from("worker")),
            (OsString::from(PID_FILE), pid_path.as_os_str().to_owned()),
        ],
    };
    let platform = ProcessPlatform::new();
    let tree = platform.spawn_tree(&spec)?;
    let grandchild_pid = wait_for_pid(&pid_path).await?;
    assert!(platform.try_wait_tree(&tree).await?.is_none());

    let stopped = platform
        .graceful_stop(&tree, Duration::from_millis(20))
        .await?;
    assert!(matches!(
        stopped,
        StopOutcome::Exited { .. } | StopOutcome::Killed
    ));
    assert!(platform.try_wait_tree(&tree).await.is_err());
    assert_process_gone(grandchild_pid).await?;
    Ok(())
}

#[tokio::test]
async fn dropping_final_platform_owner_kills_the_process_group_or_job() -> TestResult {
    let directory = tempfile::tempdir()?;
    let pid_path = directory.path().join("drop-grandchild.pid");
    let platform = ProcessPlatform::new();
    let tree = platform.spawn_tree(&worker_spec(pid_path.as_path())?)?;
    let grandchild_pid = wait_for_pid(&pid_path).await?;
    assert!(platform.try_wait_tree(&tree).await?.is_none());

    drop(platform);
    assert_process_gone(grandchild_pid).await?;
    Ok(())
}

#[cfg(windows)]
#[tokio::test]
async fn killing_owner_without_rust_drop_closes_job_and_kills_descendants() -> TestResult {
    let directory = tempfile::tempdir()?;
    let pid_path = directory.path().join("owner-kill-grandchild.pid");
    let child = std::process::Command::new(std::env::current_exe()?)
        .args(["--exact", "job_owner_fixture", "--nocapture"])
        .env(ROLE, "owner")
        .env(PID_FILE, &pid_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    struct OwnedFixture(std::process::Child);
    impl Drop for OwnedFixture {
        fn drop(&mut self) {
            let _kill = self.0.kill();
            let _wait = self.0.wait();
        }
    }
    let mut owner = OwnedFixture(child);
    let grandchild_pid = wait_for_pid(&pid_path).await?;
    // TerminateProcess bypasses the fixture's Rust destructors, as does Task Manager End Task.
    owner.0.kill()?;
    let _status = owner.0.wait()?;
    assert_process_gone(grandchild_pid).await?;
    Ok(())
}

#[cfg(windows)]
#[tokio::test]
async fn job_owner_fixture() -> TestResult {
    if std::env::var_os(ROLE).as_deref() != Some(OsStr::new("owner")) {
        return Ok(());
    }
    let pid_path = PathBuf::from(std::env::var_os(PID_FILE).ok_or("PID file env missing")?);
    let platform = ProcessPlatform::new();
    let _tree = platform.spawn_tree(&worker_spec(&pid_path)?)?;
    tokio::time::sleep(Duration::from_secs(60)).await;
    Ok(())
}

#[tokio::test]
async fn worker_path_must_be_absolute_and_unknown_handles_are_rejected() -> TestResult {
    let platform = ProcessPlatform::new();
    let relative = SpawnSpec {
        program: PathBuf::from("relative-worker"),
        ..SpawnSpec::default()
    };
    assert!(platform.spawn_tree(&relative).is_err());
    let unknown = polaris_sdk::registry::ProcessTree {
        root_pid: 1,
        handle_id: 1,
    };
    assert!(platform.try_wait_tree(&unknown).await.is_err());
    Ok(())
}

#[test]
fn secret_environment_keys_and_duplicate_names_are_rejected() -> TestResult {
    let platform = ProcessPlatform::new();
    let program = std::env::current_exe()?;
    let secret = SpawnSpec {
        program: program.clone(),
        env: vec![(OsString::from("NEBIUS_API_KEY"), OsString::from("value"))],
        ..SpawnSpec::default()
    };
    assert!(matches!(
        platform.spawn_tree(&secret),
        Err(polaris_sdk::registry::PlatformError::Policy(_))
    ));
    let duplicate = SpawnSpec {
        program,
        env: vec![
            (OsString::from("Worker_Mode"), OsString::from("first")),
            (OsString::from("worker_mode"), OsString::from("second")),
        ],
        ..SpawnSpec::default()
    };
    assert!(matches!(
        platform.spawn_tree(&duplicate),
        Err(polaris_sdk::registry::PlatformError::Policy(_))
    ));
    Ok(())
}

async fn wait_for_pid(path: &std::path::Path) -> Result<u32, Box<dyn Error>> {
    let deadline = tokio::time::Instant::now()
        .checked_add(Duration::from_secs(5))
        .ok_or("worker PID deadline exceeds timer range")?;
    loop {
        if let Ok(text) = std::fs::read_to_string(path) {
            return Ok(text.trim().parse()?);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("worker did not publish descendant PID".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn worker_spec(pid_path: &std::path::Path) -> Result<SpawnSpec, Box<dyn Error>> {
    Ok(SpawnSpec {
        program: std::env::current_exe()?,
        args: vec![
            OsString::from("--exact"),
            OsString::from("worker_fixture"),
            OsString::from("--nocapture"),
        ],
        cwd: None,
        env: vec![
            (OsString::from(ROLE), OsString::from("worker")),
            (OsString::from(PID_FILE), pid_path.as_os_str().to_owned()),
        ],
    })
}

async fn assert_process_gone(pid: u32) -> TestResult {
    let target = Pid::from_u32(pid);
    let deadline = tokio::time::Instant::now()
        .checked_add(Duration::from_secs(5))
        .ok_or("process exit deadline exceeds timer range")?;
    loop {
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
        if system.process(target).is_none() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("descendant process {pid} remained after tree stop").into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[test]
fn worker_fixture() -> TestResult {
    if std::env::var_os(ROLE).as_deref() != Some(OsStr::new("worker")) {
        return Ok(());
    }
    assert_eq!(
        std::env::vars_os().count(),
        2,
        "only explicit worker env is inherited"
    );
    let pid_path = std::env::var_os(PID_FILE).ok_or("PID file env missing")?;
    let grandchild = std::process::Command::new(std::env::current_exe()?)
        .arg("--exact")
        .arg("grandchild_fixture")
        .arg("--nocapture")
        .env(ROLE, "grandchild")
        .spawn()?;
    let _child_id = grandchild.id();
    std::thread::sleep(Duration::from_millis(150));
    std::thread::sleep(Duration::from_secs(60));
    let _pid_path = pid_path;
    Ok(())
}

#[test]
fn grandchild_fixture() -> TestResult {
    if std::env::var_os(ROLE).as_deref() != Some(OsStr::new("grandchild")) {
        return Ok(());
    }
    let pid_path = std::env::var_os(PID_FILE).ok_or("PID file env missing")?;
    std::fs::write(PathBuf::from(pid_path), std::process::id().to_string())?;
    std::thread::sleep(Duration::from_secs(60));
    Ok(())
}
