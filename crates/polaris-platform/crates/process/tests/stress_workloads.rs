//! Opt-in resource and failure-mode workloads. Run with `cargo test -p polaris-platform-process
//! --test stress_workloads -- --ignored --nocapture` on the target host.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use polaris_platform_process::{ProcessPlatform, RescanReason, WatchConfig, watch};
use polaris_sdk::registry::{PlatformOps, SpawnSpec, StopOutcome};
use sysinfo::{Pid, ProcessesToUpdate, System};

const STRESS_MODE: &str = "POLARIS_STRESS_MODE";
const STRESS_DEPTH: &str = "POLARIS_STRESS_DEPTH";
const STRESS_PID_DIR: &str = "POLARIS_STRESS_PID_DIR";

#[tokio::test]
#[ignore = "spawns and stops 1,000 real child process trees"]
async fn stress_spawn_stop_one_thousand_trees() -> Result<(), Box<dyn std::error::Error>> {
    let platform = ProcessPlatform::new();
    let spec = stress_spec("idle", None, None)?;
    for iteration in 0..1_000_u32 {
        let tree = platform.spawn_tree(&spec)?;
        let outcome = platform
            .graceful_stop(&tree, Duration::from_millis(2))
            .await?;
        assert!(
            matches!(outcome, StopOutcome::Exited { .. } | StopOutcome::Killed),
            "iteration {iteration} returned {outcome:?}"
        );
        if iteration % 100 == 0 {
            println!("spawn_stop_completed={}", iteration + 1);
        }
    }
    Ok(())
}

#[tokio::test]
#[ignore = "creates a real process tree five descendants deep"]
async fn stress_kills_a_depth_five_process_tree() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let platform = ProcessPlatform::new();
    let spec = stress_spec("tree", Some("5"), Some(directory.path()))?;
    let tree = platform.spawn_tree(&spec)?;
    let pids = wait_for_tree_pids(directory.path(), 5).await?;
    let outcome = platform
        .graceful_stop(&tree, Duration::from_millis(20))
        .await?;
    assert!(matches!(
        outcome,
        StopOutcome::Exited { .. } | StopOutcome::Killed
    ));
    for pid in pids {
        assert_process_gone(pid).await?;
    }
    println!("depth_five_descendants_stopped=5");
    Ok(())
}

#[test]
#[ignore = "writes 100,000 files without draining the bounded watcher queue"]
fn stress_watcher_reports_rescan_after_hundred_thousand_file_flood()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let config = WatchConfig {
        capacity: 8,
        debounce: Duration::from_millis(10),
        ..WatchConfig::default()
    };
    let watcher = watch(directory.path(), config)?;
    for index in 0..100_000_u32 {
        std::fs::write(directory.path().join(format!("event-{index:06}.tmp")), [])?;
    }
    let result = watcher.recv_timeout(Duration::from_secs(20))?;
    assert!(
        matches!(
            result,
            Some(polaris_platform_process::FsEvent::RescanRequired {
                reason: RescanReason::QueueOverflow | RescanReason::BackendOverflow
            })
        ),
        "expected an explicit rescan after event loss, received {result:?}"
    );
    println!("watcher_flood_files=100000;rescan_required=true");
    Ok(())
}

#[test]
fn stress_tree_worker_fixture() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os(STRESS_MODE).as_deref() != Some(OsStr::new("tree")) {
        return Ok(());
    }
    let depth: u32 = std::env::var(STRESS_DEPTH)?.parse()?;
    let pid_dir = PathBuf::from(std::env::var_os(STRESS_PID_DIR).ok_or("PID directory missing")?);
    std::fs::write(
        pid_dir.join(format!("depth-{depth}.pid")),
        std::process::id().to_string(),
    )?;
    if depth > 1 {
        let child = std::process::Command::new(std::env::current_exe()?)
            .args(["--exact", "stress_tree_worker_fixture", "--nocapture"])
            .env(STRESS_MODE, "tree")
            .env(STRESS_DEPTH, (depth - 1).to_string())
            .env(STRESS_PID_DIR, &pid_dir)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let _child_pid = child.id();
    }
    std::thread::sleep(Duration::from_secs(60));
    Ok(())
}

#[test]
fn stress_idle_worker_fixture() {
    if std::env::var_os(STRESS_MODE).as_deref() == Some(OsStr::new("idle")) {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn stress_spec(
    mode: &str,
    depth: Option<&str>,
    pid_dir: Option<&Path>,
) -> Result<SpawnSpec, Box<dyn std::error::Error>> {
    let mut env = vec![(OsString::from(STRESS_MODE), OsString::from(mode))];
    if let Some(depth) = depth {
        env.push((OsString::from(STRESS_DEPTH), OsString::from(depth)));
    }
    if let Some(pid_dir) = pid_dir {
        env.push((
            OsString::from(STRESS_PID_DIR),
            pid_dir.as_os_str().to_owned(),
        ));
    }
    Ok(SpawnSpec {
        program: std::env::current_exe()?,
        args: vec![
            OsString::from("--exact"),
            OsString::from(if mode == "tree" {
                "stress_tree_worker_fixture"
            } else {
                "stress_idle_worker_fixture"
            }),
            OsString::from("--nocapture"),
        ],
        cwd: None,
        env,
    })
}

async fn wait_for_tree_pids(
    directory: &Path,
    count: u32,
) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let mut pids = Vec::new();
        for depth in 1..=count {
            let path = directory.join(format!("depth-{depth}.pid"));
            let Ok(text) = std::fs::read_to_string(path) else {
                break;
            };
            pids.push(text.trim().parse()?);
        }
        if pids.len() == count as usize {
            return Ok(pids);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("only {} of {count} descendant PIDs appeared", pids.len()).into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn assert_process_gone(pid: u32) -> Result<(), Box<dyn std::error::Error>> {
    let target = Pid::from_u32(pid);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let mut system = System::new();
        system.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
        if system.process(target).is_none() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("descendant process {pid} remained after stop").into());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
