//! Native process fixtures for worker bootstrap contracts.

use std::error::Error;
use std::ffi::OsString;
use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

use polaris_sdk::registry::{
    PlatformError, PlatformOps, ProcessTree, SpawnSpec, WorkerBootstrapPlatform,
};
use secrecy::SecretString;
use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::ProcessPlatform;

type TestResult = Result<(), Box<dyn Error>>;

const FIXTURE_MODE: &str = "POLARIS_BOOTSTRAP_FIXTURE_MODE";
const FIXTURE_OUTPUT: &str = "POLARIS_BOOTSTRAP_FIXTURE_OUTPUT";
const FIXTURE_LENGTH: &str = "POLARIS_BOOTSTRAP_FIXTURE_LENGTH";
const FIXTURE_BYTES: &[u8] = b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[tokio::test]
async fn bootstrap_writes_exact_prefix_retains_stdin_and_rejects_duplicate() -> TestResult {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("received.bin");
    let platform = ProcessPlatform::new();
    let tree = platform.spawn_tree(&fixture_spec(output.as_path())?)?;

    platform
        .bootstrap_tree(
            &tree,
            SecretString::from(String::from_utf8(FIXTURE_BYTES.to_vec())?),
            Duration::from_secs(2),
        )
        .await?;
    wait_for_file(output.as_path()).await?;
    let received = std::fs::read(output.as_path())?;
    assert_eq!(
        received, FIXTURE_BYTES,
        "worker must receive the exact credential prefix"
    );
    assert!(
        platform.try_wait_tree(&tree).await?.is_none(),
        "successful bootstrap must retain the live worker"
    );
    assert!(
        platform
            .bootstrap_tree(
                &tree,
                SecretString::from(String::from_utf8(FIXTURE_BYTES.to_vec())?),
                Duration::from_secs(2)
            )
            .await
            .is_err(),
        "a process tree may be bootstrapped only once"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !output.with_extension("second.bin").exists(),
        "duplicate call must not write a second credential prefix"
    );

    let _outcome = platform
        .graceful_stop(&tree, Duration::from_millis(20))
        .await?;
    assert_process_gone(tree.root_pid).await?;
    Ok(())
}

#[tokio::test]
async fn bootstrap_accepts_exactly_4096_secret_bytes() -> TestResult {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("received-maximum.bin");
    let credential = "z".repeat(4096);
    let platform = ProcessPlatform::new();
    let tree = platform.spawn_tree(&fixture_spec_with_length(
        output.as_path(),
        credential.len(),
    )?)?;

    platform
        .bootstrap_tree(
            &tree,
            SecretString::from(credential.clone()),
            Duration::from_secs(2),
        )
        .await?;
    wait_for_file(output.as_path()).await?;
    assert_eq!(
        std::fs::read(output.as_path())?,
        credential.as_bytes(),
        "the maximum accepted secret must arrive exactly"
    );
    assert!(
        platform.try_wait_tree(&tree).await?.is_none(),
        "maximum-size success must retain the worker"
    );
    let _outcome = platform
        .graceful_stop(&tree, Duration::from_millis(20))
        .await?;
    assert_process_gone(tree.root_pid).await?;
    Ok(())
}

#[tokio::test]
async fn bootstrap_rejects_mismatched_tree_before_io() -> TestResult {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("received-invalid.bin");
    let platform = ProcessPlatform::new();
    let tree = platform.spawn_tree(&fixture_spec(output.as_path())?)?;
    let mismatched = ProcessTree {
        root_pid: tree.root_pid.saturating_add(1),
        handle_id: tree.handle_id,
    };

    assert!(
        platform
            .bootstrap_tree(
                &mismatched,
                SecretString::from("credential"),
                Duration::from_secs(1)
            )
            .await
            .is_err(),
        "mismatched root PID must be rejected"
    );
    assert!(
        platform.try_wait_tree(&tree).await?.is_none(),
        "invalid handle must not stop the valid process"
    );
    assert!(!output.exists(), "invalid tree must not access child stdin");

    let _outcome = platform
        .graceful_stop(&tree, Duration::from_millis(20))
        .await?;
    assert_process_gone(tree.root_pid).await?;
    Ok(())
}

#[tokio::test]
async fn invalid_secret_bounds_leave_worker_live_and_never_write() -> TestResult {
    for credential in [String::new(), "x".repeat(4097)] {
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("received-invalid-secret.bin");
        let platform = ProcessPlatform::new();
        let tree = platform.spawn_tree(&fixture_spec(output.as_path())?)?;
        assert!(
            platform
                .bootstrap_tree(
                    &tree,
                    SecretString::from(credential),
                    Duration::from_secs(1)
                )
                .await
                .is_err(),
            "out-of-range secret length must fail closed"
        );
        assert!(
            platform.try_wait_tree(&tree).await?.is_none(),
            "invalid input must not stop a healthy worker"
        );
        assert!(
            !output.exists(),
            "invalid secret must not reach child stdin"
        );
        let _outcome = platform
            .graceful_stop(&tree, Duration::from_millis(20))
            .await?;
        assert_process_gone(tree.root_pid).await?;
    }
    Ok(())
}

#[tokio::test]
async fn invalid_deadlines_leave_worker_live_and_never_write() -> TestResult {
    for timeout in [Duration::ZERO, Duration::MAX] {
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("received-invalid-deadline.bin");
        let platform = ProcessPlatform::new();
        let tree = platform.spawn_tree(&fixture_spec(output.as_path())?)?;
        assert!(
            platform
                .bootstrap_tree(&tree, SecretString::from("credential"), timeout)
                .await
                .is_err(),
            "invalid bootstrap deadline must fail before claiming the process"
        );
        assert!(
            platform.try_wait_tree(&tree).await?.is_none(),
            "invalid deadline must not stop a healthy worker"
        );
        assert!(
            !output.exists(),
            "invalid deadline must not reach child stdin"
        );
        let _outcome = platform
            .graceful_stop(&tree, Duration::from_millis(20))
            .await?;
        assert_process_gone(tree.root_pid).await?;
    }
    Ok(())
}

#[tokio::test]
async fn bootstrap_timeout_waiting_for_child_lock_stops_owned_process() -> TestResult {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("received-timeout.bin");
    let platform = ProcessPlatform::new();
    let tree = platform.spawn_tree(&fixture_spec(output.as_path())?)?;
    let (child, _state) = platform.record(&tree)?;
    let child_guard = child.lock().await;

    let result = tokio::time::timeout(
        Duration::from_secs(1),
        platform.bootstrap_tree(
            &tree,
            SecretString::from("credential"),
            Duration::from_millis(20),
        ),
    )
    .await?;
    assert!(
        result.is_err(),
        "child-lock wait must count toward the bootstrap deadline"
    );
    drop(child_guard);
    assert_tree_exited(&platform, &tree).await?;
    Ok(())
}

#[tokio::test]
async fn dropping_pending_bootstrap_future_stops_owned_process_after_lock_release() -> TestResult {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("received-cancelled.bin");
    let platform = ProcessPlatform::new();
    let tree = platform.spawn_tree(&fixture_spec(output.as_path())?)?;
    let (child, _state) = platform.record(&tree)?;
    let child_guard = child.lock().await;
    let mut future = Box::pin(platform.bootstrap_tree(
        &tree,
        SecretString::from("credential"),
        Duration::from_secs(5),
    ));
    let did_wait = std::future::poll_fn(|context| {
        let poll = Future::poll(Pin::as_mut(&mut future), context);
        Poll::Ready(poll.is_pending())
    })
    .await;
    assert!(did_wait, "bootstrap must wait on the retained child lock");
    drop(future);
    drop(child_guard);
    assert_tree_exited(&platform, &tree).await?;
    Ok(())
}

#[tokio::test]
async fn kill_tree_reports_pending_when_lock_contended_and_cleanup_stops_process() -> TestResult {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("received-kill-pending.bin");
    let platform = ProcessPlatform::new();
    let tree = platform.spawn_tree(&fixture_spec(output.as_path())?)?;
    let (child, _state) = platform.record(&tree)?;
    let child_guard = child.lock().await;

    let result = platform.kill_tree(&tree);
    assert!(
        matches!(result, Err(PlatformError::Io(message)) if message.contains("pending")),
        "contended kill_tree must report that termination is pending and unconfirmed"
    );
    drop(child_guard);
    assert_tree_exited(&platform, &tree).await?;
    Ok(())
}

#[test]
fn stdin_worker_fixture() -> TestResult {
    if std::env::var_os(FIXTURE_MODE).as_deref() != Some(std::ffi::OsStr::new("stdin")) {
        return Ok(());
    }
    let output =
        PathBuf::from(std::env::var_os(FIXTURE_OUTPUT).ok_or("fixture output path missing")?);
    let expected_length = std::env::var(FIXTURE_LENGTH)?.parse::<usize>()?;
    let mut stdin = std::io::stdin().lock();
    let mut received = vec![0; expected_length];
    stdin.read_exact(&mut received)?;
    std::fs::write(&output, received)?;
    let mut duplicate = vec![0; expected_length];
    if stdin.read_exact(&mut duplicate).is_ok() {
        std::fs::write(output.with_extension("second.bin"), duplicate)?;
    }
    std::thread::sleep(Duration::from_secs(60));
    Ok(())
}

fn fixture_spec(output: &Path) -> Result<SpawnSpec, Box<dyn Error>> {
    fixture_spec_with_length(output, FIXTURE_BYTES.len())
}

fn fixture_spec_with_length(
    output: &Path,
    expected_length: usize,
) -> Result<SpawnSpec, Box<dyn Error>> {
    Ok(SpawnSpec {
        program: std::env::current_exe()?,
        args: vec![
            OsString::from("--exact"),
            OsString::from("bootstrap_tests::stdin_worker_fixture"),
            OsString::from("--nocapture"),
        ],
        cwd: None,
        env: vec![
            (OsString::from(FIXTURE_MODE), OsString::from("stdin")),
            (
                OsString::from(FIXTURE_OUTPUT),
                output.as_os_str().to_owned(),
            ),
            (
                OsString::from(FIXTURE_LENGTH),
                OsString::from(expected_length.to_string()),
            ),
        ],
    })
}

async fn wait_for_file(path: &Path) -> TestResult {
    let deadline = tokio::time::Instant::now()
        .checked_add(Duration::from_secs(5))
        .ok_or("fixture file deadline exceeds timer range")?;
    loop {
        if path.exists() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("worker did not record bootstrap bytes".into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn assert_process_gone(root_pid: u32) -> TestResult {
    let target = Pid::from_u32(root_pid);
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
            return Err(format!("worker process {root_pid} remained after forced cleanup").into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn assert_tree_exited(platform: &ProcessPlatform, tree: &ProcessTree) -> TestResult {
    let deadline = tokio::time::Instant::now()
        .checked_add(Duration::from_secs(5))
        .ok_or("owned process exit deadline exceeds timer range")?;
    loop {
        if platform.try_wait_tree(tree).await?.is_some() {
            return assert_process_gone(tree.root_pid).await;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("owned process did not report exit after forced cleanup".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
