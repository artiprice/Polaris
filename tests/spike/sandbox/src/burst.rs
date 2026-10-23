//! Bounded lifecycle owner; fake transport results never establish live gateway behavior.
use polaris_kernel_reaper::{
    Disposition, Locator, Reaper, ResourceKind, Teardown, TeardownContext, TeardownError,
};
use polaris_sdk::{
    ids::CommandId,
    registry::{ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSpec},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{sync::Semaphore, task::JoinSet};

/// Checked workload limits.
#[derive(Debug, Clone, Copy)]
pub struct BurstLimits {
    /// Maximum simultaneously owned create/exec/destroy lifecycles.
    pub concurrent: usize,
    /// Number of cycles per worker slot.
    pub cycles: usize,
    /// Bounded deadline for each operation and a queued permit.
    pub deadline: Duration,
}
impl Default for BurstLimits {
    fn default() -> Self {
        Self {
            concurrent: 20,
            cycles: 5,
            deadline: Duration::from_secs(180),
        }
    }
}
/// Observed typed failure associated with one cycle.
#[derive(Debug)]
pub struct CycleFailure {
    /// Zero-based cycle index.
    pub cycle: usize,
    /// Failed lifecycle operation.
    pub stage: &'static str,
    /// Actual transport, deadline or journal failure.
    pub error: SpikeError,
}
/// Measured counts and remaining ownership after all workers joined.
#[derive(Debug, Default)]
pub struct BurstReport {
    /// Requested cycles.
    pub requested: usize,
    /// Confirmed successful creates.
    pub created: usize,
    /// Confirmed execs with exit zero.
    pub executed: usize,
    /// Confirmed destroys, including cleanup after ambiguous create failure.
    pub destroyed: usize,
    /// Peak observed active lifecycle owners.
    pub peak_concurrent: usize,
    /// Actual duration of every create attempt, in microseconds.
    pub create_latency_us: Vec<u64>,
    /// Every failed stage; a nonempty list prevents acceptance.
    pub failures: Vec<CycleFailure>,
    /// Remaining reaper entries, including failed teardown records.
    pub remaining_resources: usize,
}
/// Explicit lifecycle failures.
#[derive(Debug, thiserror::Error)]
pub enum SpikeError {
    /// Invalid workload before side effects.
    #[error("invalid workload: {0}")]
    Configuration(&'static str),
    /// Backend operation failed.
    #[error(transparent)]
    Backend(#[from] SandboxError),
    /// Ownership/journal failure.
    #[error(transparent)]
    Reaper(#[from] polaris_kernel_reaper::ReaperError),
    /// Bounded operation expired.
    #[error("operation deadline exceeded")]
    Deadline,
    /// Response or owner violated the lifecycle contract.
    #[error("workload integrity: {0}")]
    Integrity(&'static str),
}
struct Remote<B> {
    backend: Arc<B>,
    handle: SandboxHandle,
}
impl<B: SandboxBackend> Teardown for Remote<B> {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        self.backend
            .destroy(&self.handle)
            .await
            .map_err(|error| TeardownError::Other(error.to_string()))?;
        Ok(Disposition::Clean)
    }
    async fn force(&mut self, _ctx: TeardownContext) -> Result<(), TeardownError> {
        self.backend
            .destroy(&self.handle)
            .await
            .map_err(|error| TeardownError::Other(error.to_string()))
    }
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
#[derive(Default)]
struct Cycle {
    created: bool,
    executed: bool,
    destroyed: bool,
    latency: Option<u64>,
    failures: Vec<CycleFailure>,
}
impl Cycle {
    fn failed(&mut self, cycle: usize, stage: &'static str, error: impl Into<SpikeError>) {
        self.failures.push(CycleFailure {
            cycle,
            stage,
            error: error.into(),
        });
    }
}
/// Admit bounded lifecycles, retain their ownership, and join every worker even after failures.
/// The caller must retain this future until it returns, then explicitly reap any reported
/// unresolved resources. Journal entries are released only on confirmed cleanup or a typed
/// refusal of ownership (`AlreadyExists`). Names are generated privately after admission.
/// # Errors
/// Invalid limits or a failed worker; typed backend failures are retained in the returned report.
pub async fn run_sandboxes<B: SandboxBackend + 'static>(
    backend: Arc<B>,
    reaper: Arc<Reaper>,
    spec: SandboxSpec,
    limits: BurstLimits,
    command: ExecRequest,
) -> Result<BurstReport, SpikeError> {
    if !(1..=64).contains(&limits.concurrent)
        || !(1..=100).contains(&limits.cycles)
        || limits.deadline.is_zero()
        || limits.deadline > Duration::from_secs(900)
        || command.argv.is_empty()
        || spec.name.is_some()
    {
        return Err(SpikeError::Configuration(
            "bounded positive limits, argv and no shared name required",
        ));
    }
    let requested = limits
        .concurrent
        .checked_mul(limits.cycles)
        .ok_or(SpikeError::Configuration("cycle overflow"))?;
    if requested > 2000 {
        return Err(SpikeError::Configuration("at most 2000 cycles"));
    }
    let permits = Arc::new(Semaphore::new(limits.concurrent));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut workers = JoinSet::new();
    for cycle in 0..requested {
        let backend = Arc::clone(&backend);
        let reaper = Arc::clone(&reaper);
        let permits = Arc::clone(&permits);
        let active = Arc::clone(&active);
        let peak = Arc::clone(&peak);
        let spec = spec.clone();
        let command = command.clone();
        workers.spawn(async move {
            let permit = match tokio::time::timeout(limits.deadline, permits.acquire_owned()).await
            {
                Ok(Ok(permit)) => permit,
                _ => {
                    let mut result = Cycle::default();
                    result.failed(cycle, "admission", SpikeError::Deadline);
                    return result;
                }
            };
            let now = active.fetch_add(1, Ordering::SeqCst).saturating_add(1);
            peak.fetch_max(now, Ordering::SeqCst);
            let _active = Active(&active);
            let _permit = permit;
            lifecycle(backend, reaper, spec, limits.deadline, command, cycle).await
        });
    }
    let mut report = BurstReport {
        requested,
        ..BurstReport::default()
    };
    let mut failed_worker = false;
    while let Some(joined) = workers.join_next().await {
        match joined {
            Ok(result) => {
                report.created = report.created.saturating_add(usize::from(result.created));
                report.executed = report.executed.saturating_add(usize::from(result.executed));
                report.destroyed = report
                    .destroyed
                    .saturating_add(usize::from(result.destroyed));
                if let Some(latency) = result.latency {
                    report.create_latency_us.push(latency);
                }
                report.failures.extend(result.failures);
            }
            Err(_) => failed_worker = true,
        }
    }
    report.peak_concurrent = peak.load(Ordering::SeqCst);
    report.remaining_resources = reaper.leak_report().entries.len();
    if failed_worker {
        return Err(SpikeError::Integrity(
            "worker failed; inspect retained reaper journal",
        ));
    }
    Ok(report)
}
async fn lifecycle<B: SandboxBackend + 'static>(
    backend: Arc<B>,
    reaper: Arc<Reaper>,
    mut spec: SandboxSpec,
    deadline: Duration,
    command: ExecRequest,
    cycle: usize,
) -> Cycle {
    let mut result = Cycle::default();
    let suffix: String = CommandId::new_v7()
        .as_uuid()
        .simple()
        .to_string()
        .chars()
        .rev()
        .take(16)
        .collect();
    let handle = match SandboxHandle::new(&format!("sb-{suffix}")) {
        Ok(handle) => handle,
        Err(_) => {
            result.failed(
                cycle,
                "identity",
                SpikeError::Integrity("invalid generated identity"),
            );
            return result;
        }
    };
    spec.name = Some(handle.as_str().to_owned());
    let registration = match reaper.register_journaled(
        ResourceKind::Sandbox,
        Locator::Label(handle.as_str().to_owned()),
        Remote {
            backend: Arc::clone(&backend),
            handle: handle.clone(),
        },
    ) {
        Ok(id) => id,
        Err(error) => {
            result.failed(cycle, "journal", error);
            return result;
        }
    };
    let start = Instant::now();
    let creation = tokio::time::timeout(deadline, backend.create(&spec)).await;
    result.latency = u64::try_from(start.elapsed().as_micros()).ok();
    let mut collision = false;
    match creation {
        Ok(Ok(created)) if created == handle => {
            result.created = true;
            match tokio::time::timeout(deadline, backend.exec(&handle, &command)).await {
                Ok(Ok(output)) if output.exit_code == 0 => result.executed = true,
                Ok(Ok(_)) => {
                    result.failed(cycle, "exec", SpikeError::Integrity("nonzero exec status"))
                }
                Ok(Err(error)) => result.failed(cycle, "exec", error),
                Err(_) => result.failed(cycle, "exec", SpikeError::Deadline),
            }
        }
        Ok(Ok(actual)) => {
            match reaper.register_journaled(
                ResourceKind::Sandbox,
                Locator::Label(actual.as_str().to_owned()),
                Remote {
                    backend: Arc::clone(&backend),
                    handle: actual,
                },
            ) {
                Ok(_) => result.failed(
                    cycle,
                    "create",
                    SpikeError::Integrity("backend changed requested identity"),
                ),
                Err(error) => result.failed(cycle, "journal_actual_identity", error),
            }
        }
        Ok(Err(SandboxError::AlreadyExists(detail))) => {
            collision = true;
            result.failed(cycle, "create", SandboxError::AlreadyExists(detail));
        }
        Ok(Err(error)) => result.failed(cycle, "create", error),
        Err(_) => result.failed(cycle, "create", SpikeError::Deadline),
    }
    if !collision {
        match tokio::time::timeout(deadline, backend.destroy(&handle)).await {
            Ok(Ok(())) => result.destroyed = true,
            Ok(Err(error)) => result.failed(cycle, "destroy", error),
            Err(_) => result.failed(cycle, "destroy", SpikeError::Deadline),
        }
    }
    if (result.destroyed || collision)
        && let Err(error) = reaper.release(registration)
    {
        result.failed(cycle, "release", error);
    }
    result
}
