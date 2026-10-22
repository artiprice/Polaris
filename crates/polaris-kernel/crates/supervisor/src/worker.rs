//! Supervision of OS worker process trees through the SDK platform boundary.

use std::sync::Arc;
use std::time::Duration;

use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::registry::{PlatformError, PlatformOps, ProcessTree, SpawnSpec};
use tokio_util::sync::CancellationToken;

use crate::task::{Supervisor, SupervisorError, TaskError, TaskSpec};
use polaris_sdk::registry::EventStore;

/// Process and restart settings for a supervised worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerSpec {
    /// Actor restart policy and observable worker kind.
    pub task: TaskSpec,
    /// Program, arguments, working directory, and complete child environment allowlist.
    pub process: SpawnSpec,
    /// Interval between process-exit observations.
    pub poll_interval: Duration,
    /// Grace period given to the worker before the platform forcibly stops its tree.
    pub stop_grace: Duration,
}

/// Caller-owned real worker health transport (for example a protocol client).
/// Tests may inject failures; production supplies its actual backend without a success fallback.
pub trait HealthProbe: Send + Sync {
    /// Confirm that this exact owned worker is responsive, or return a typed failure.
    fn check(
        &self,
        tree: ProcessTree,
    ) -> impl std::future::Future<Output = Result<(), TaskError>> + Send;
}

/// Limits for a worker's health checks, independent of process-exit polling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthPolicy {
    /// Delay between checks; must be positive.
    pub interval: Duration,
    /// Upper bound on one probe; must be positive.
    pub timeout: Duration,
}

impl<S: EventStore + 'static> Supervisor<S> {
    /// Supervise a native worker with a real, caller-injected responsiveness probe.
    /// A failed or timed-out probe terminates the exact owned tree before the restart policy runs.
    /// The caller's in-flight protocol requests observe that transport closure.
    /// # Errors
    /// Zero health limits, invalid worker poll interval or normal supervisor registration failure.
    pub fn spawn_worker_with_health<P, H>(
        &self,
        platform: Arc<P>,
        spec: WorkerSpec,
        health: Arc<H>,
        policy: HealthPolicy,
    ) -> Result<crate::task::ChildId, SupervisorError>
    where
        P: PlatformOps + 'static,
        H: HealthProbe + 'static,
    {
        if policy.interval.is_zero() || policy.timeout.is_zero() || spec.poll_interval.is_zero() {
            return Err(SupervisorError::Config(
                "worker health limits must be non-zero",
            ));
        }
        let task = spec.task;
        self.spawn_task(task, move |cancellation| {
            let platform = Arc::clone(&platform);
            let health = Arc::clone(&health);
            let spec = spec.clone();
            async move {
                let tree = platform
                    .spawn_tree(&spec.process)
                    .map_err(worker_platform_error)?;
                let mut guard = WorkerProcessGuard::new(Arc::clone(&platform), tree);
                // Keep the guard outside both futures. Cancelling either future cannot detach
                // the process ownership; a panic or failed stop still reaches its Drop backstop.
                let outcome = {
                    let worker = monitor_worker(
                        Arc::clone(&platform),
                        tree,
                        cancellation,
                        spec.poll_interval,
                        spec.stop_grace,
                        &mut guard,
                    );
                    tokio::select! {
                        biased;
                        result = worker => Ok(result),
                        failure = health_failure(health.as_ref(), tree, policy) => Err(failure),
                    }
                };
                match outcome {
                    Ok(result) => result,
                    Err(failure) => {
                        let stop_budget =
                            spec.stop_grace.saturating_add(Duration::from_millis(100));
                        if let Ok(Ok(_)) = tokio::time::timeout(
                            stop_budget,
                            platform.graceful_stop(&tree, spec.stop_grace),
                        )
                        .await
                        {
                            guard.disarm();
                        }
                        Err(failure)
                    }
                }
            }
        })
    }

    /// Starts and supervises a native worker process tree.
    ///
    /// Each restart uses the same immutable spawn specification. Cancellation requests a
    /// graceful tree stop and the platform contract must force-stop the tree after the grace
    /// period. The process handle is retained by the platform adapter throughout this lifecycle.
    ///
    /// # Errors
    /// Invalid poll interval or any error from normal supervisor registration.
    pub fn spawn_worker<P>(
        &self,
        platform: Arc<P>,
        spec: WorkerSpec,
    ) -> Result<crate::task::ChildId, SupervisorError>
    where
        P: PlatformOps + 'static,
    {
        if spec.poll_interval.is_zero() {
            return Err(SupervisorError::Config(
                "worker poll interval must be non-zero",
            ));
        }
        let task = spec.task;
        let process = spec.process;
        let poll_interval = spec.poll_interval;
        let stop_grace = spec.stop_grace;
        self.spawn_task(task, move |cancellation| {
            let platform = Arc::clone(&platform);
            let process = process.clone();
            async move {
                let tree = platform
                    .spawn_tree(&process)
                    .map_err(worker_platform_error)?;
                let mut process_guard = WorkerProcessGuard::new(Arc::clone(&platform), tree);
                monitor_worker(
                    Arc::clone(&platform),
                    tree,
                    cancellation,
                    poll_interval,
                    stop_grace,
                    &mut process_guard,
                )
                .await
            }
        })
    }
}

async fn health_failure<H: HealthProbe>(
    health: &H,
    tree: ProcessTree,
    policy: HealthPolicy,
) -> TaskError {
    loop {
        tokio::time::sleep(policy.interval).await;
        match tokio::time::timeout(policy.timeout, health.check(tree)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return error,
            Err(_) => return TaskError::transient("worker health probe timed out"),
        }
    }
}

async fn monitor_worker<P: PlatformOps + 'static>(
    platform: Arc<P>,
    tree: polaris_sdk::registry::ProcessTree,
    cancellation: CancellationToken,
    poll_interval: Duration,
    stop_grace: Duration,
    process_guard: &mut WorkerProcessGuard<P>,
) -> Result<(), TaskError> {
    loop {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                match platform.graceful_stop(&tree, stop_grace).await {
                    Ok(_) | Err(PlatformError::NotFound(_)) => {
                        process_guard.disarm();
                        return Ok(());
                    }
                    Err(_) => match platform.graceful_stop(&tree, Duration::ZERO).await {
                        Ok(_) | Err(PlatformError::NotFound(_)) => {
                            process_guard.disarm();
                            return Ok(());
                        }
                        Err(error) => return Err(worker_platform_error(error)),
                    },
                }
            }
            () = tokio::time::sleep(poll_interval) => {
                match platform.try_wait_tree(&tree).await {
                    Ok(Some(exit)) => {
                        process_guard.disarm();
                        return worker_exit(exit.code);
                    }
                    Ok(None) => {}
                    Err(error) => return Err(worker_platform_error(error)),
                }
            }
        }
    }
}

fn worker_exit(code: Option<i32>) -> Result<(), TaskError> {
    match code {
        Some(0) => Ok(()),
        // Unix signal termination has no code; Windows exception NTSTATUS has its high bit set
        // and is exposed through the signed SDK exit code.
        None => Err(TaskError::crashed("worker terminated by a signal")),
        Some(code) if code < 0 => Err(TaskError::crashed(format!(
            "worker OS exception status {code}"
        ))),
        Some(code) => Err(TaskError::transient(format!(
            "worker process exited with code {code}"
        ))),
    }
}

#[cfg(test)]
mod exit_tests {
    use super::worker_exit;
    use crate::ExitClass;

    #[test]
    fn clean_worker_exit_is_success_and_signal_or_ntstatus_is_a_crash() {
        assert!(worker_exit(Some(0)).is_ok());
        for code in [None, Some(i32::from_le_bytes([5, 0, 0, 192]))] {
            let Err(error) = worker_exit(code) else {
                panic!("crash was accepted as clean");
            };
            assert_eq!(error.exit_class(), ExitClass::Crashed);
        }
        let Err(error) = worker_exit(Some(17)) else {
            panic!("failure was accepted as clean");
        };
        assert_eq!(error.exit_class(), ExitClass::Failed);
    }
}

struct WorkerProcessGuard<P: PlatformOps> {
    platform: Arc<P>,
    tree: ProcessTree,
    armed: bool,
}

impl<P: PlatformOps> WorkerProcessGuard<P> {
    fn new(platform: Arc<P>, tree: ProcessTree) -> Self {
        Self {
            platform,
            tree,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl<P: PlatformOps> Drop for WorkerProcessGuard<P> {
    fn drop(&mut self) {
        if self.armed
            && let Err(error) = self.platform.kill_tree(&self.tree)
        {
            tracing::error!(
                pid = self.tree.root_pid,
                error = %error,
                "failed to force-stop worker tree during task cancellation"
            );
        }
    }
}

fn worker_platform_error(error: PlatformError) -> TaskError {
    match error.kind() {
        ErrorKind::Transient => TaskError::transient(error.to_string()),
        ErrorKind::Permanent | ErrorKind::Policy | ErrorKind::Fatal => {
            TaskError::fatal(error.to_string())
        }
    }
}
