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
///
/// Startup initialization uses its separately supplied startup deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthPolicy {
    /// Delay between checks; must be positive.
    pub interval: Duration,
    /// Upper bound on one probe; must be positive.
    pub timeout: Duration,
}

/// Startup and generation retirement for a worker with a real health transport.
///
/// Initialization runs once for every spawned process, before any health probe. Implementors
/// must publish a usable connection only after bootstrap and protocol validation succeed.
/// Retirement must synchronously revoke only the matching generation; it runs on normal exit,
/// startup failure, cancellation, and task abort. It must not block, panic, or disclose secrets.
pub trait WorkerLifecycle: HealthProbe {
    /// Initialize this exact owned process before health monitoring starts.
    fn initialize(
        &self,
        tree: ProcessTree,
    ) -> impl std::future::Future<Output = Result<(), TaskError>> + Send;

    /// Revoke a matching published connection without revoking a newer generation.
    fn retire(&self, tree: ProcessTree);
}

impl<S: EventStore + 'static> Supervisor<S> {
    /// Supervise an initialized worker with startup, health, and process ownership kept together.
    ///
    /// Each restart gets a new initialization call. The process guard is installed before the
    /// initializer is polled, and the retirement guard invalidates its connection before the
    /// process guard's emergency stop. A cleanup deadline is not proof of OS termination; an
    /// unconfirmed stop is fatal and cannot trigger another automatic process generation.
    ///
    /// # Errors
    /// Rejects zero polling/health/startup limits, startup or stop grace above 120 seconds, and
    /// normal supervisor registration failures.
    pub fn spawn_worker_with_lifecycle<P, L>(
        &self,
        platform: Arc<P>,
        spec: WorkerSpec,
        lifecycle: Arc<L>,
        policy: HealthPolicy,
        startup_timeout: Duration,
    ) -> Result<crate::task::ChildId, SupervisorError>
    where
        P: PlatformOps + 'static,
        L: WorkerLifecycle + 'static,
    {
        if spec.poll_interval.is_zero()
            || policy.interval.is_zero()
            || policy.timeout.is_zero()
            || startup_timeout.is_zero()
            || startup_timeout > Duration::from_secs(120)
            || spec.stop_grace > Duration::from_secs(120)
        {
            return Err(SupervisorError::Config(
                "invalid initialized worker deadlines",
            ));
        }
        let task = spec.task;
        self.spawn_task(task, move |cancellation| {
            let platform = Arc::clone(&platform);
            let lifecycle = Arc::clone(&lifecycle);
            let spec = spec.clone();
            async move {
                let tree = platform
                    .spawn_tree(&spec.process)
                    .map_err(worker_platform_error)?;
                let mut process_guard = WorkerProcessGuard::new(Arc::clone(&platform), tree);
                let mut generation_guard = WorkerGenerationGuard {
                    lifecycle: Arc::clone(&lifecycle),
                    tree,
                    armed: true,
                };
                let initialized = tokio::select! {
                    biased;
                    () = cancellation.cancelled() => None,
                    result = tokio::time::timeout(startup_timeout, lifecycle.initialize(tree)) => {
                        Some(match result {
                            Ok(result) => result,
                            Err(_) => Err(TaskError::transient("worker initialization timed out")),
                        })
                    }
                };
                match initialized {
                    None => {
                        generation_guard.retire();
                        stop_owned_worker(
                            platform.as_ref(),
                            tree,
                            spec.stop_grace,
                            &mut process_guard,
                        )
                        .await?;
                        return Ok(());
                    }
                    Some(Err(error)) => {
                        generation_guard.retire();
                        stop_owned_worker(
                            platform.as_ref(),
                            tree,
                            spec.stop_grace,
                            &mut process_guard,
                        )
                        .await?;
                        return Err(error);
                    }
                    Some(Ok(())) => {}
                }
                let outcome = {
                    let worker = monitor_worker(
                        Arc::clone(&platform),
                        tree,
                        cancellation.clone(),
                        spec.poll_interval,
                        spec.stop_grace,
                        &mut process_guard,
                    );
                    tokio::select! {
                        biased;
                        () = cancellation.cancelled() => None,
                        result = worker => Some(Ok(result)),
                        failure = health_failure(lifecycle.as_ref(), tree, policy) => Some(Err(failure)),
                    }
                };
                generation_guard.retire();
                match outcome {
                    None => {
                        stop_owned_worker(
                            platform.as_ref(),
                            tree,
                            spec.stop_grace,
                            &mut process_guard,
                        )
                        .await
                    }
                    Some(Ok(result)) => {
                        if process_guard.armed {
                            stop_owned_worker(
                                platform.as_ref(),
                                tree,
                                spec.stop_grace,
                                &mut process_guard,
                            )
                            .await?;
                        }
                        result
                    }
                    Some(Err(failure)) => {
                        stop_owned_worker(
                            platform.as_ref(),
                            tree,
                            spec.stop_grace,
                            &mut process_guard,
                        )
                        .await?;
                        Err(failure)
                    }
                }
            }
        })
    }

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

struct WorkerGenerationGuard<L: WorkerLifecycle> {
    lifecycle: Arc<L>,
    tree: ProcessTree,
    armed: bool,
}

impl<L: WorkerLifecycle> WorkerGenerationGuard<L> {
    fn retire(&mut self) {
        if self.armed {
            self.armed = false;
            self.lifecycle.retire(self.tree);
        }
    }
}

impl<L: WorkerLifecycle> Drop for WorkerGenerationGuard<L> {
    fn drop(&mut self) {
        self.retire();
    }
}

async fn stop_owned_worker<P: PlatformOps>(
    platform: &P,
    tree: ProcessTree,
    grace: Duration,
    guard: &mut WorkerProcessGuard<P>,
) -> Result<(), TaskError> {
    let budget = grace
        .checked_add(Duration::from_millis(100))
        .ok_or_else(|| TaskError::fatal("worker cleanup deadline overflow"))?;
    match tokio::time::timeout(budget, platform.graceful_stop(&tree, grace)).await {
        Ok(Ok(_)) | Ok(Err(PlatformError::NotFound(_))) => {
            guard.disarm();
            Ok(())
        }
        Ok(Err(_)) | Err(_) => Err(TaskError::fatal(
            "worker cleanup did not confirm termination",
        )),
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
                return stop_owned_worker(
                    platform.as_ref(),
                    tree,
                    stop_grace,
                    process_guard,
                ).await;
            }
            () = tokio::time::sleep(poll_interval) => {
                match platform.try_wait_tree(&tree).await {
                    Ok(Some(exit)) => {
                        process_guard.disarm();
                        return worker_exit(exit.code);
                    }
                    Ok(None) => {}
                      Err(PlatformError::NotFound(_)) => {
                          // This exact tree was spawned by this monitor. The adapter's
                          // emergency kill path may already have retired its record;
                          // restart policy still governs replacement of that generation.
                          process_guard.disarm();
                          return Err(TaskError::transient("owned worker process record was retired"));
                      }
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
#[path = "../../../../../tests/crates/polaris-kernel/crates/supervisor/src/worker.rs"]
mod exit_tests;

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
