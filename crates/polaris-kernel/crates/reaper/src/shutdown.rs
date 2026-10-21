//! Resource registry and ordered teardown. Registry mutation is journal-first from the caller's
//! perspective: failed journal updates roll back registration/release instead of claiming success.
use crate::journal;
use crate::resources::{
    ProcessResource, TaskResource, TempResource, orphan_temp_path, remove_owned,
};
use crate::{
    Disposition, JOURNAL_FILE_NAME, LeakEntry, LeakReport, Locator, OrphanAction, OrphanReport,
    Outcome, ReapId, ReaperError, ResourceKind, ShutdownConfig, ShutdownEntry, ShutdownReport,
    SweepReport, TEMP_DIR_PREFIX, Teardown, TeardownContext, TeardownError,
};
use polaris_sdk::registry::{PlatformOps, ProcessTree};
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

// Heterogeneous resources require erasure on this cold path only. The public API stays generic;
// resource implementations and all business logic use the typed Teardown trait.
trait Erased: Send {
    fn graceful(
        &mut self,
        ctx: TeardownContext,
    ) -> Pin<Box<dyn Future<Output = Result<Disposition, TeardownError>> + Send + '_>>;
    fn force(
        &mut self,
        ctx: TeardownContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), TeardownError>> + Send + '_>>;
}
impl<R: Teardown> Erased for R {
    fn graceful(
        &mut self,
        ctx: TeardownContext,
    ) -> Pin<Box<dyn Future<Output = Result<Disposition, TeardownError>> + Send + '_>> {
        Box::pin(Teardown::graceful(self, ctx))
    }
    fn force(
        &mut self,
        ctx: TeardownContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), TeardownError>> + Send + '_>> {
        Box::pin(Teardown::force(self, ctx))
    }
}
type Resource = Arc<AsyncMutex<Box<dyn Erased>>>;
struct Entry {
    meta: LeakEntry,
    resource: Option<Resource>,
}
struct State {
    entries: BTreeMap<ReapId, Entry>,
    next: u64,
    shutdown: bool,
    ignored: usize,
}

/// Owner of cancellation and cleanup registration. Use one owner per journal directory.
pub struct Reaper {
    root: CancellationToken,
    journal: PathBuf,
    state: Mutex<State>,
}
impl core::fmt::Debug for Reaper {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Reaper")
            .field("journal", &self.journal)
            .field("resources", &self.leak_report())
            .finish_non_exhaustive()
    }
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
impl Reaper {
    /// Open a journal directory, refusing unknown formats without modifying them. Loaded records
    /// are orphans until explicitly swept; no PID is killed merely because it appears in a file.
    ///
    /// # Errors
    /// Reports I/O errors and unsupported/corrupt complete journal records.
    pub fn new(dir: impl AsRef<Path>) -> Result<Self, ReaperError> {
        std::fs::create_dir_all(dir.as_ref())?;
        let journal = std::fs::canonicalize(dir.as_ref())?.join(JOURNAL_FILE_NAME);
        let (loaded, ignored) = journal::read(&journal)?;
        let next = match loaded.keys().next_back() {
            Some(id) => id.0,
            None => 0,
        };
        let entries = loaded
            .into_iter()
            .map(|(id, meta)| {
                (
                    id,
                    Entry {
                        meta,
                        resource: None,
                    },
                )
            })
            .collect();
        Ok(Self {
            root: CancellationToken::new(),
            journal,
            state: Mutex::new(State {
                entries,
                next,
                shutdown: false,
                ignored,
            }),
        })
    }
    /// New descendant token; already cancelled when root cancellation has begun.
    #[must_use]
    pub fn child_token(&self) -> CancellationToken {
        self.root.child_token()
    }
    /// Cancel the entire registered work tree.
    pub fn cancel(&self) {
        self.root.cancel();
    }
    /// Whether root cancellation has begun.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.root.is_cancelled()
    }
    fn persist(&self, state: &State) -> Result<(), ReaperError> {
        journal::write(
            &self.journal,
            state.entries.values().map(|entry| entry.meta.clone()),
        )
    }

    /// Register an in-process resource, without a persistent locator.
    ///
    /// # Errors
    /// Rejects shutdown, identity exhaustion or failed journal maintenance.
    pub fn register<R: Teardown + 'static>(
        &self,
        kind: ResourceKind,
        resource: R,
    ) -> Result<ReapId, ReaperError> {
        self.register_journaled(kind, Locator::None, resource)
    }
    /// Register owned cleanup with minimal persistent recovery metadata.
    ///
    /// # Errors
    /// Rejects shutdown, identity exhaustion or failed atomic journal updates.
    pub fn register_journaled<R: Teardown + 'static>(
        &self,
        kind: ResourceKind,
        locator: Locator,
        resource: R,
    ) -> Result<ReapId, ReaperError> {
        let mut state = lock(&self.state);
        if state.shutdown {
            return Err(ReaperError::AlreadyShutDown);
        }
        let next = state
            .next
            .checked_add(1)
            .ok_or_else(|| ReaperError::JournalFormat("resource id space exhausted".to_owned()))?;
        let id = ReapId(next);
        let entry = Entry {
            meta: LeakEntry { id, kind, locator },
            resource: Some(Arc::new(AsyncMutex::new(Box::new(resource)))),
        };
        let _previous = state.entries.insert(id, entry);
        if let Err(error) = self.persist(&state) {
            let _rolled_back = state.entries.remove(&id);
            return Err(error);
        }
        state.next = next;
        Ok(id)
    }
    /// Transfer ownership of a task handle to the reaper.
    ///
    /// # Errors
    /// As [`Self::register`].
    pub fn register_task(&self, task: JoinHandle<()>) -> Result<ReapId, ReaperError> {
        self.register(ResourceKind::Task, TaskResource(task))
    }
    /// Transfer a TempDir, retaining its RAII best-effort cleanup even before explicit shutdown.
    ///
    /// # Errors
    /// As [`Self::register_journaled`].
    pub fn register_temp_dir(&self, dir: tempfile::TempDir) -> Result<ReapId, ReaperError> {
        let path = dir.path().to_path_buf();
        self.register_journaled(
            ResourceKind::TempDir,
            Locator::Path(path),
            TempResource(dir),
        )
    }
    /// Create and register a prefixed directory eligible for automatic orphan recovery.
    ///
    /// # Errors
    /// Directory creation or registration failed.
    pub fn create_temp_dir(&self, parent: &Path) -> Result<(ReapId, PathBuf), ReaperError> {
        let dir = tempfile::Builder::new()
            .prefix(TEMP_DIR_PREFIX)
            .tempdir_in(parent)?;
        let path = dir.path().to_owned();
        let id = self.register_temp_dir(dir)?;
        Ok((id, path))
    }
    /// Register an already-owned process tree through the sdk platform interface.
    ///
    /// # Errors
    /// As [`Self::register_journaled`].
    pub fn register_process<P: PlatformOps + 'static>(
        &self,
        platform: Arc<P>,
        tree: ProcessTree,
    ) -> Result<ReapId, ReaperError> {
        self.register_journaled(
            ResourceKind::Process,
            Locator::Pid(tree.root_pid),
            ProcessResource { platform, tree },
        )
    }
    /// Remove a completed registration and its recovery locator.
    ///
    /// # Errors
    /// Unknown id or journal update failure; journal failure restores the registration.
    pub fn release(&self, id: ReapId) -> Result<(), ReaperError> {
        let mut state = lock(&self.state);
        let entry = state
            .entries
            .remove(&id)
            .ok_or(ReaperError::UnknownResource)?;
        if let Err(error) = self.persist(&state) {
            let _previous = state.entries.insert(id, entry);
            return Err(error);
        }
        Ok(())
    }
    /// List live resources and unresolved orphans in registration order.
    #[must_use]
    pub fn leak_report(&self) -> LeakReport {
        LeakReport {
            entries: lock(&self.state)
                .entries
                .values()
                .map(|entry| entry.meta.clone())
                .collect(),
        }
    }
    /// Remove an orphan only after an external operator/backend has resolved it.
    ///
    /// # Errors
    /// Rejects unknown or currently owned resources; journal errors leave the orphan retained.
    pub fn acknowledge_orphan(&self, id: ReapId) -> Result<(), ReaperError> {
        let mut state = lock(&self.state);
        if !state
            .entries
            .get(&id)
            .is_some_and(|entry| entry.resource.is_none())
        {
            return Err(ReaperError::UnknownResource);
        }
        let entry = state
            .entries
            .remove(&id)
            .ok_or(ReaperError::UnknownResource)?;
        if let Err(error) = self.persist(&state) {
            let _previous = state.entries.insert(id, entry);
            return Err(error);
        }
        Ok(())
    }
    /// Sweep only orphan directories with the reaper prefix. PID and backend locators require
    /// external confirmation and remain in the journal. No process is killed by a bare PID.
    ///
    /// # Errors
    /// Journal commit failed or a blocking filesystem worker could not be joined.
    pub async fn sweep_orphans(&self) -> Result<SweepReport, ReaperError> {
        let (orphans, ignored): (Vec<_>, _) = {
            let state = lock(&self.state);
            (
                state
                    .entries
                    .values()
                    .filter(|entry| entry.resource.is_none())
                    .map(|entry| entry.meta.clone())
                    .collect(),
                state.ignored,
            )
        };
        let mut reports = Vec::new();
        for orphan in orphans {
            let action = match (&orphan.kind, &orphan.locator) {
                (ResourceKind::TempDir, Locator::Path(path)) => {
                    if let Some(owned) = orphan_temp_path(path) {
                        match tokio::task::spawn_blocking(move || remove_owned(&owned)).await {
                            Ok(Ok(())) => OrphanAction::Removed,
                            Ok(Err(error)) => OrphanAction::Failed(error.to_string()),
                            Err(error) => OrphanAction::Failed(error.to_string()),
                        }
                    } else {
                        OrphanAction::NeedsOperator
                    }
                }
                _ => OrphanAction::NeedsOperator,
            };
            if action == OrphanAction::Removed {
                self.acknowledge_orphan(orphan.id)?;
            }
            reports.push(OrphanReport {
                id: orphan.id,
                kind: orphan.kind,
                locator: orphan.locator,
                action,
            });
        }
        let mut state = lock(&self.state);
        self.persist(&state)?;
        state.ignored = 0;
        Ok(SweepReport {
            orphans: reports,
            ignored,
        })
    }
    /// Cancel first, then clean each owned resource by category and reverse registration order.
    /// Panics in supplied cleanup futures are isolated in Tokio tasks and reported as failures.
    /// Unconfirmed resources retain both their locator and cleanup owner for inspection.
    ///
    /// # Errors
    /// A second/concurrent shutdown is rejected. Journal failures are retained in the report.
    pub async fn shutdown_all(
        &self,
        config: &ShutdownConfig,
    ) -> Result<ShutdownReport, ReaperError> {
        let mut resources: Vec<_> = {
            let mut state = lock(&self.state);
            if state.shutdown {
                return Err(ReaperError::AlreadyShutDown);
            }
            state.shutdown = true;
            state
                .entries
                .values()
                .filter_map(|entry| {
                    entry
                        .resource
                        .as_ref()
                        .map(|resource| (entry.meta.clone(), Arc::clone(resource)))
                })
                .collect()
        };
        self.root.cancel();
        resources.sort_by_key(|(meta, _)| (meta.kind, Reverse(meta.id)));
        let started = Instant::now();
        let mut entries = Vec::new();
        for (meta, resource) in resources {
            let remaining = config.overall.saturating_sub(started.elapsed());
            let outcome = if remaining.is_zero() {
                Outcome::Skipped
            } else {
                let config = *config;
                let result =
                    tokio::spawn(async move { teardown_one(resource, config, remaining).await })
                        .await;
                match result {
                    Ok(outcome) => outcome,
                    Err(error) => Outcome::Failed(format!("teardown task failed: {error}")),
                }
            };
            if outcome.resolved() {
                let _released = lock(&self.state).entries.remove(&meta.id);
            }
            entries.push(ShutdownEntry {
                id: meta.id,
                kind: meta.kind,
                outcome,
            });
        }
        let elapsed = started.elapsed();
        let journal_error = self
            .persist(&lock(&self.state))
            .err()
            .map(|error| error.to_string());
        Ok(ShutdownReport {
            entries,
            elapsed,
            deadline_exceeded: elapsed >= config.overall,
            journal_error,
        })
    }
}
impl Drop for Reaper {
    fn drop(&mut self) {
        self.root.cancel();
    }
}

async fn teardown_one(
    resource: Resource,
    config: ShutdownConfig,
    remaining: std::time::Duration,
) -> Outcome {
    let started = Instant::now();
    let mut resource = resource.lock().await;
    let graceful = config.graceful.min(remaining);
    match tokio::time::timeout(
        graceful,
        resource.graceful(TeardownContext {
            reason: config.reason,
            budget: graceful,
        }),
    )
    .await
    {
        Ok(Ok(Disposition::Clean)) => return Outcome::Released,
        Ok(Ok(Disposition::Forced)) => return Outcome::Forced,
        Ok(Err(error)) => return Outcome::Failed(error.to_string()),
        Err(_) => {}
    }
    let forced = config
        .forced
        .min(remaining.saturating_sub(started.elapsed()));
    if forced.is_zero() {
        return Outcome::Abandoned;
    }
    match tokio::time::timeout(
        forced,
        resource.force(TeardownContext {
            reason: config.reason,
            budget: forced,
        }),
    )
    .await
    {
        Ok(Ok(())) => Outcome::Forced,
        Ok(Err(error)) => Outcome::Failed(error.to_string()),
        Err(_) => Outcome::Abandoned,
    }
}
