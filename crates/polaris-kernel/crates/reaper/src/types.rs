//! Teardown contracts and inspectable resource outcomes.
use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

/// Journal file written inside the supplied run directory.
pub const JOURNAL_FILE_NAME: &str = "reaper.journal";
/// Prefix required before an orphan temp directory can be removed automatically.
pub const TEMP_DIR_PREFIX: &str = "polaris-reaper-";

/// Resource id, monotonically increasing within a run journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReapId(pub(crate) u64);
impl ReapId {
    /// Construct a persisted registration identity.
    ///
    /// # Errors
    /// Returns [`ReaperError::JournalFormat`] when the value is zero, which is never allocated.
    pub fn from_raw(value: u64) -> Result<Self, ReaperError> {
        if value == 0 {
            return Err(ReaperError::JournalFormat(
                "resource id must be nonzero".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Return the stable numeric identity written in the reaper journal.
    #[must_use]
    pub const fn as_raw(self) -> u64 {
        self.0
    }
}
/// Teardown priority, lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceKind {
    /// Cooperative task, then abort if necessary.
    Task,
    /// Owned process tree.
    Process,
    /// Worker control channel.
    WorkerPipe,
    /// Remote or local sandbox.
    Sandbox,
    /// Owned temporary directory.
    TempDir,
    /// Shared-memory segment.
    IpcSegment,
}
impl ResourceKind {
    /// Every resource kind, in shutdown priority order.
    pub const ALL: [Self; 6] = [
        Self::Task,
        Self::Process,
        Self::WorkerPipe,
        Self::Sandbox,
        Self::TempDir,
        Self::IpcSegment,
    ];
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Process => "process",
            Self::WorkerPipe => "worker_pipe",
            Self::Sandbox => "sandbox",
            Self::TempDir => "temp_dir",
            Self::IpcSegment => "ipc_segment",
        }
    }
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}
/// Minimal recovery identity; never store credentials, commands or resource payloads here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Locator {
    /// Resource exists only in the current process.
    None,
    /// Owned resource path.
    Path(PathBuf),
    /// PID only; insufficient identity for automated crash recovery.
    Pid(u32),
    /// Sandbox or other backend-owned identifier.
    Label(String),
}
/// Why teardown was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeardownReason {
    /// Normal explicit shutdown.
    Shutdown,
    /// User interrupt.
    CtrlC,
    /// Panic escalation.
    Panic,
    /// Parent process disappeared.
    ParentDied,
}
/// Per-step context, injected by the reaper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeardownContext {
    /// Shutdown trigger.
    pub reason: TeardownReason,
    /// Maximum remaining time for this step.
    pub budget: Duration,
}
/// What a successful graceful operation actually did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Resource cooperated.
    Clean,
    /// Backend had to force release.
    Forced,
}
/// Teardown implementation failure. Text is retained for the caller, never logged automatically.
#[derive(Debug, thiserror::Error)]
pub enum TeardownError {
    /// Backend failure.
    #[error("{0}")]
    Other(String),
    /// Local I/O failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
/// Generic resource cleanup. The reaper erases this only at its heterogeneous cold-path boundary.
pub trait Teardown: Send {
    /// Cooperatively release within the supplied step budget.
    fn graceful(
        &mut self,
        ctx: TeardownContext,
    ) -> impl Future<Output = Result<Disposition, TeardownError>> + Send;
    /// Force release after graceful timeout; the default reports lack of a force mechanism.
    fn force(
        &mut self,
        _ctx: TeardownContext,
    ) -> impl Future<Output = Result<(), TeardownError>> + Send {
        async {
            Err(TeardownError::Other(
                "resource has no forced teardown".to_owned(),
            ))
        }
    }
}
/// Time budgets for one ordered teardown run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownConfig {
    /// Trigger passed to every cleanup implementation.
    pub reason: TeardownReason,
    /// Maximum cooperative time per resource.
    pub graceful: Duration,
    /// Maximum forced time per resource.
    pub forced: Duration,
    /// Shared total budget, including all resources.
    pub overall: Duration,
}
impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            reason: TeardownReason::Shutdown,
            graceful: Duration::from_secs(4),
            forced: Duration::from_secs(1),
            overall: Duration::from_secs(10),
        }
    }
}
/// Final status of one cleanup attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Confirmed released cooperatively.
    Released,
    /// Confirmed released forcibly.
    Forced,
    /// Explicit backend failure.
    Failed(String),
    /// Timed out without confirmation.
    Abandoned,
    /// Overall budget expired before this resource started.
    Skipped,
}
impl Outcome {
    pub(crate) const fn resolved(&self) -> bool {
        matches!(self, Self::Released | Self::Forced)
    }
}
/// One ordered shutdown result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownEntry {
    /// Registered identity.
    pub id: ReapId,
    /// Cleanup priority category.
    pub kind: ResourceKind,
    /// Observed outcome.
    pub outcome: Outcome,
}
/// Every resource's outcome, including those not successfully cleaned up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownReport {
    /// In execution order.
    pub entries: Vec<ShutdownEntry>,
    /// Observed monotonic time spent in resource teardown (journal commit is separate).
    pub elapsed: Duration,
    /// Whether the overall resource budget expired.
    pub deadline_exceeded: bool,
    /// Recovery-journal update failure, if any.
    pub journal_error: Option<String>,
}
impl ShutdownReport {
    /// Find a registered resource's outcome.
    #[must_use]
    pub fn outcome(&self, id: ReapId) -> Option<&Outcome> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| &entry.outcome)
    }
}
/// Live or unresolved resource identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeakEntry {
    /// Registration id.
    pub id: ReapId,
    /// Resource category.
    pub kind: ResourceKind,
    /// Recovery locator.
    pub locator: Locator,
}
/// Live/unresolved resources, in registration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeakReport {
    /// Entries still owned by the reaper or awaiting orphan review.
    pub entries: Vec<LeakEntry>,
}
/// Recovery action for one crashed-run record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrphanAction {
    /// Owned directory was removed or already absent.
    Removed,
    /// Automatic recovery cannot safely establish ownership or contact a backend.
    NeedsOperator,
    /// Local cleanup failed; locator remains journaled.
    Failed(String),
}
/// Inspectable action and identity of one orphan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanReport {
    /// Persisted registration id.
    pub id: ReapId,
    /// Persisted category.
    pub kind: ResourceKind,
    /// Persisted locator.
    pub locator: Locator,
    /// Actual recovery action.
    pub action: OrphanAction,
}
/// Startup recovery report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepReport {
    /// Exactly one entry per distinct journal identity.
    pub orphans: Vec<OrphanReport>,
    /// Incomplete trailing records discarded by journal parsing.
    pub ignored: usize,
}
/// Registration, recovery or shutdown API failure.
#[derive(Debug, thiserror::Error)]
pub enum ReaperError {
    /// Filesystem failure while maintaining recovery evidence.
    #[error("reaper journal: {0}")]
    Journal(#[from] std::io::Error),
    /// Journal syntax/version cannot safely be interpreted.
    #[error("invalid reaper journal: {0}")]
    JournalFormat(String),
    /// Registration or another shutdown attempted after shutdown began.
    #[error("reaper already shut down")]
    AlreadyShutDown,
    /// Resource not registered or orphan already acknowledged.
    #[error("unknown resource")]
    UnknownResource,
    /// The background shutdown coordinator failed unexpectedly.
    #[error("reaper shutdown coordinator failed: {0}")]
    ShutdownWorker(String),
}
