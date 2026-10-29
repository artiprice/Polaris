// Unit tests of crates/polaris-kernel/src/worker_lifecycle.rs (`tests`); compiled into that module via #[path].
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::{CurrentS1PiiDetector, S1WorkerLifecycle};
use polaris_kernel_supervisor::{TaskError, WorkerLifecycle};
use polaris_sdk::registry::{
    MemoryPressure, PlatformError, PlatformOps, ProcessExit, ProcessTree, SpawnSpec,
    StopOutcome as PlatformStopOutcome, WorkerBootstrapPlatform,
};
use polaris_system_one::client::ClientConfig;

struct TestPlatform {
    bootstrap_calls: AtomicUsize,
    fail_bootstrap: bool,
}

impl PlatformOps for TestPlatform {
    fn spawn_tree(&self, _spec: &SpawnSpec) -> Result<ProcessTree, PlatformError> {
        Err(PlatformError::Unsupported("test spawn is unused"))
    }

    async fn graceful_stop(
        &self,
        _tree: &ProcessTree,
        _grace: Duration,
    ) -> Result<PlatformStopOutcome, PlatformError> {
        Ok(PlatformStopOutcome::Exited { code: Some(0) })
    }

    fn kill_tree(&self, _tree: &ProcessTree) -> Result<(), PlatformError> {
        Ok(())
    }

    async fn try_wait_tree(
        &self,
        _tree: &ProcessTree,
    ) -> Result<Option<ProcessExit>, PlatformError> {
        Ok(None)
    }

    fn memory_pressure(&self) -> Result<MemoryPressure, PlatformError> {
        Ok(MemoryPressure::Normal)
    }
}

impl WorkerBootstrapPlatform for TestPlatform {
    fn bootstrap_tree(
        &self,
        _tree: &ProcessTree,
        _secret: secrecy::SecretString,
        _timeout: Duration,
    ) -> impl std::future::Future<Output = Result<(), PlatformError>> + Send {
        self.bootstrap_calls.fetch_add(1, Ordering::Relaxed);
        let fail_bootstrap = self.fail_bootstrap;
        async move {
            if fail_bootstrap {
                Err(PlatformError::Policy(
                    "private bootstrap detail must not escape".to_owned(),
                ))
            } else {
                Ok(())
            }
        }
    }
}

fn lifecycle(
    fail_bootstrap: bool,
) -> Result<(S1WorkerLifecycle<TestPlatform>, Arc<TestPlatform>), Box<dyn std::error::Error>> {
    let platform = Arc::new(TestPlatform {
        bootstrap_calls: AtomicUsize::new(0),
        fail_bootstrap,
    });
    let client_config = ClientConfig::new(Duration::from_secs(1), 64)?;
    let lifecycle = S1WorkerLifecycle::new(
        Arc::clone(&platform),
        PathBuf::from("invalid-worker-endpoint"),
        client_config,
        Duration::from_secs(1),
    )?;
    Ok((lifecycle, platform))
}

fn tree(root_pid: u32, handle_id: u64) -> ProcessTree {
    ProcessTree {
        root_pid,
        handle_id,
    }
}

#[tokio::test]
async fn initialization_bootstraps_before_connect_and_never_publishes_failure()
-> Result<(), Box<dyn std::error::Error>> {
    let (lifecycle, platform) = lifecycle(false)?;
    let error = lifecycle
        .initialize(tree(17, 71))
        .await
        .err()
        .ok_or("invalid endpoint unexpectedly initialized")?;
    assert_eq!(
        error.to_string(),
        "supervised task failed: S1 worker local connection failed",
        "connection failures must be sanitized"
    );
    assert_eq!(platform.bootstrap_calls.load(Ordering::Relaxed), 1);
    assert!(lifecycle.connection()?.is_none());
    Ok(())
}

#[tokio::test]
async fn bootstrap_errors_are_sanitized_and_do_not_publish_connections()
-> Result<(), Box<dyn std::error::Error>> {
    let (lifecycle, platform) = lifecycle(true)?;
    let error = lifecycle
        .initialize(tree(18, 72))
        .await
        .err()
        .ok_or("failed bootstrap unexpectedly initialized")?;
    assert_eq!(
        error.to_string(),
        "supervised task failed: S1 worker credential bootstrap failed",
        "platform details must not escape the adapter"
    );
    assert!(!error.to_string().contains("private bootstrap detail"));
    assert_eq!(platform.bootstrap_calls.load(Ordering::Relaxed), 1);
    assert!(lifecycle.connection()?.is_none());
    Ok(())
}

#[test]
fn retiring_an_old_generation_does_not_clear_the_new_generation() -> Result<(), TaskError> {
    let (lifecycle, _) =
        lifecycle(false).map_err(|_| TaskError::fatal("test lifecycle construction failed"))?;
    let old_tree = tree(19, 73);
    let new_tree = tree(20, 74);
    lifecycle.begin_generation(old_tree)?;
    lifecycle.begin_generation(new_tree)?;

    lifecycle.retire(old_tree);

    let latest = super::lock_state(&lifecycle.latest_tree)?;
    assert_eq!(
        *latest,
        Some(new_tree),
        "old retirement must preserve the new generation"
    );
    assert!(
        !super::generation_matches(*latest, old_tree),
        "old generation must be fenced from publication"
    );
    assert!(
        super::generation_matches(*latest, new_tree),
        "new generation must remain publishable"
    );
    Ok(())
}

#[test]
fn readiness_observer_receives_generation_transitions() -> Result<(), Box<dyn std::error::Error>> {
    let (lifecycle, _) = lifecycle(false)?;
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed_by_callback = Arc::clone(&observed);
    lifecycle.set_readiness_observer(Arc::new(move |ready| {
        if let Ok(mut values) = observed_by_callback.lock() {
            values.push(ready);
        }
    }))?;
    lifecycle.report_readiness(false);
    lifecycle.report_readiness(true);
    lifecycle.report_readiness(false);
    let values = observed
        .lock()
        .map_err(|_| "readiness test lock poisoned")?;
    assert_eq!(
        values.as_slice(),
        [false, true, false],
        "every host transition must fence readiness"
    );
    Ok(())
}

#[tokio::test]
async fn current_detector_refuses_to_reuse_a_missing_worker_client()
-> Result<(), Box<dyn std::error::Error>> {
    let (lifecycle, _) = lifecycle(false)?;
    let detector = CurrentS1PiiDetector::new(Arc::new(lifecycle));
    let error =
        polaris_sdk::system_one::PiiSpanDetector::detect_pii_spans(&detector, "masked input")
            .await
            .err()
            .ok_or("detector unexpectedly succeeded without an active worker")?;
    assert!(
        matches!(error, polaris_sdk::registry::DecisionError::Unavailable(_)),
        "missing generation must return unavailable"
    );
    Ok(())
}
