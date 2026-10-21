//! Local composition storage and ownership. Blocking APIs are explicit; async transitions move
//! filesystem/database work off Tokio workers using `spawn_blocking`.
use crate::{
    KernelError,
    bus::{Bus, BusConfig, CommandReceiver, EventStream},
    reaper::{OrphanAction, Reaper, ShutdownConfig, ShutdownReport},
    storage::{
        cas::{Cas, CasConfig},
        eventlog::{EventLog, EventLogConfig},
        registry::{Registry, SessionMeta},
    },
};
use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::ids::{Seq, SessionId};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// Shared local core state. One EventLog owner per root excludes competing process contexts.
#[derive(Debug)]
pub struct KernelCtx {
    pub(crate) cas: Cas,
    pub(crate) bus: Arc<Bus<Arc<EventLog>>>,
    pub(crate) registry: Registry,
    pub(crate) reaper: Reaper,
    pub(crate) writer: Mutex<()>,
    active: Mutex<BTreeSet<SessionId>>,
    closed: AtomicBool,
    // Retained for the orchestrator to own once its verified dispatcher is assembled.
    _commands: Mutex<CommandReceiver>,
    root: PathBuf,
}
pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, KernelError> + Send + 'static,
) -> Result<T, KernelError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| KernelError::Runtime(error.to_string()))?
}
impl KernelCtx {
    /// Open local durable stores, sweep safe orphan directories, and refuse unresolved resources
    /// before exposing any session API. Failed startup drops already-opened handles in reverse.
    ///
    /// # Errors
    /// Database/filesystem failures, journal errors, or `RecoveryRequired` for unsafe orphans.
    pub async fn open(root: impl AsRef<Path>) -> Result<Arc<Self>, KernelError> {
        let root = root.as_ref().to_owned();
        let (root, cas, log, registry, reaper) = blocking(move || {
            std::fs::create_dir_all(&root)?;
            let root = std::fs::canonicalize(root)?;
            let cas = Cas::open(CasConfig::new(root.join("cas")))?;
            let log = Arc::new(EventLog::open(EventLogConfig::new(
                root.join("events.redb"),
            ))?);
            let registry = Registry::open(&root)?;
            let reaper = Reaper::new(root.join("run"))?;
            Ok((root, cas, log, registry, reaper))
        })
        .await?;
        let sweep = reaper.sweep_orphans().await?;
        let unresolved = sweep
            .orphans
            .iter()
            .filter(|orphan| orphan.action != OrphanAction::Removed)
            .count();
        if unresolved != 0 {
            return Err(KernelError::RecoveryRequired { count: unresolved });
        }
        let (bus, commands) = Bus::new(BusConfig::default(), log)?;
        Ok(Arc::new(Self {
            cas,
            bus: Arc::new(bus),
            registry,
            reaper,
            writer: Mutex::new(()),
            active: Mutex::new(BTreeSet::new()),
            closed: AtomicBool::new(false),
            _commands: Mutex::new(commands),
            root,
        }))
    }
    /// Root holding `events.redb`, `registry.db`, `cas/`, and `run/reaper.journal`.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub(crate) fn ensure_open(&self) -> Result<(), KernelError> {
        if self.closed.load(Ordering::Acquire) {
            Err(KernelError::Closed)
        } else {
            Ok(())
        }
    }
    pub(crate) fn lock_writer(&self) -> Result<MutexGuard<'_, ()>, KernelError> {
        self.writer.lock().map_err(|_| KernelError::Poisoned)
    }
    pub(crate) fn claim(self: &Arc<Self>, id: SessionId) -> Result<SessionLease, KernelError> {
        let _writer = self.lock_writer()?;
        self.ensure_open()?;
        let mut active = self.active.lock().map_err(|_| KernelError::Poisoned)?;
        if !active.insert(id) {
            return Err(KernelError::SessionAlreadyOpen(id));
        }
        Ok(SessionLease {
            id,
            ctx: Arc::clone(self),
        })
    }
    pub(crate) fn publish(&self, id: SessionId, event: Event) -> Result<EventRecord, KernelError> {
        self.ensure_open()?;
        // SQLite's catalogue stores microsecond timestamps. Stamp the authoritative record at
        // that same explicit precision so projection rebuilds preserve identical timestamps.
        let at = Timestamp::from_microsecond(Timestamp::now().as_microsecond())
            .map_err(|_| KernelError::Integrity("current timestamp is out of range"))?;
        let record = EventRecord::new(Seq::ZERO, at, id, event);
        let seq = self.bus.publish(record)?;
        Ok(record.with_seq(seq))
    }
    /// Read an inclusive event range. Blocking database I/O; async consumers use an owner thread.
    ///
    /// # Errors
    /// Event-log read or codec failure.
    pub fn records(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, KernelError> {
        Ok(self.bus.store().read_range(from, to)?)
    }
    /// Read a rebuildable registry projection. This is blocking I/O.
    ///
    /// # Errors
    /// Registry read failure.
    pub fn session_meta(&self, id: SessionId) -> Result<Option<SessionMeta>, KernelError> {
        Ok(self.registry.get_session(id)?)
    }
    /// Subscribe to future committed records; older records are available through [`Self::records`].
    #[must_use]
    pub fn subscribe(&self) -> EventStream {
        self.bus.subscribe()
    }
    /// Stop accepting session mutations before cancelling and reaping owned resources.
    /// Immediate-durability log commits are already fsynced when publication returns.
    ///
    /// # Errors
    /// Repeated shutdown is rejected; journal cleanup failures remain in the returned report.
    pub async fn shutdown(self: &Arc<Self>) -> Result<ShutdownReport, KernelError> {
        let ctx = Arc::clone(self);
        blocking(move || {
            let _writer = ctx.lock_writer()?;
            if ctx.closed.swap(true, Ordering::AcqRel) {
                return Err(KernelError::Closed);
            }
            Ok(())
        })
        .await?;
        Ok(self.reaper.shutdown_all(&ShutdownConfig::default()).await?)
    }
}
#[derive(Debug)]
pub(crate) struct SessionLease {
    id: SessionId,
    ctx: Arc<KernelCtx>,
}
impl Drop for SessionLease {
    fn drop(&mut self) {
        let mut active = match self.ctx.active.lock() {
            Ok(active) => active,
            Err(poisoned) => poisoned.into_inner(),
        };
        let _removed = active.remove(&self.id);
    }
}
