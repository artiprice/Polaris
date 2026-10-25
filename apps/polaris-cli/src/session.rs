//! Trusted asynchronous adapters over the real local kernel, registry and bounded CAS.
use crate::CliError;
use polaris_kernel::storage::registry::{Registry, SessionFilter, SessionMeta};
use polaris_kernel::{Kernel, KernelEvents, KernelHandle, LocalKernelConfig};
use polaris_sdk::{
    contracts::{Event, EventRecord},
    ids::{FrameId, Seq, SessionId},
    registry::{CommandReceipt, EventSource, KernelApi},
};
use polaris_storage_cas::{Cas, CasConfig, CasError};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug)]
struct PendingRecord {
    record: EventRecord,
    payload: tokio::task::JoinHandle<Result<Vec<u8>, CasError>>,
}

/// A committed domain record and a validated optional display payload.
#[derive(Debug)]
pub struct ResolvedRecord {
    /// Original fixed-size authoritative envelope; sequence is never synthesized.
    pub record: EventRecord,
    /// Bounded UTF-8 text for a directive, resolved by the adapter before projection.
    pub text: Option<String>,
}

/// One real local session and its ordered replay/live stream. It never generates model replies.
#[derive(Debug)]
pub struct OfflineSession {
    /// Stable session selected at creation or resume.
    pub id: SessionId,
    handle: KernelHandle,
    events: KernelEvents,
    cas: Arc<Cas>,
    root: PathBuf,
    workspace: PathBuf,
    pending_record: Option<PendingRecord>,
}
impl OfflineSession {
    /// Start or reopen local durable state and subscribe from the first global record.
    /// # Errors
    /// Real storage, recovery, session, replay or adapter failure.
    pub async fn open(root: &Path, workspace: &Path, id: SessionId) -> Result<Self, CliError> {
        let handle = Kernel::start(LocalKernelConfig::new(
            root,
            workspace,
            id,
            FrameId::new_v7(),
        ))
        .await?;
        let initialized = Self::initialize(handle.clone(), id).await;
        match initialized {
            Ok(session) => Ok(session),
            Err(primary) => match handle.shutdown().await {
                Ok(report) => match check_shutdown_report(&report) {
                    Ok(()) => Err(primary),
                    Err(cleanup) => Err(CliError::Ui(format!(
                        "session initialization failed: {primary}; cleanup also failed: {cleanup}"
                    ))),
                },
                Err(cleanup) => Err(CliError::Ui(format!(
                    "session initialization failed: {primary}; cleanup also failed: {cleanup}"
                ))),
            },
        }
    }
    async fn initialize(handle: KernelHandle, id: SessionId) -> Result<Self, CliError> {
        let root = handle.context().root().to_owned();
        let ctx = Arc::clone(handle.context());
        let metadata = tokio::task::spawn_blocking(move || ctx.session_meta(id))
            .await
            .map_err(|_| CliError::Unavailable("session metadata owner failed"))??;
        let workspace = metadata
            .ok_or(CliError::Unavailable("active session metadata is missing"))?
            .workspace;
        let cas_root = root.join("cas");
        let cas = tokio::task::spawn_blocking(move || Cas::open(CasConfig::new(cas_root)))
            .await
            .map_err(|_| CliError::Unavailable("CAS reader owner failed"))?
            .map_err(|e| CliError::Ui(format!("CAS reader failed: {e}")))?;
        let events = handle.subscribe(Seq::new(1)).await?;
        Ok(Self {
            id,
            handle,
            events,
            cas: Arc::new(cas),
            root,
            workspace,
            pending_record: None,
        })
    }
    /// Actual workspace stored with the session, including when resumed.
    #[must_use]
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
    /// Commit exactly one directive through the signed kernel admission/verifier path.
    /// # Errors
    /// Grammar, bounds, authorization, queue, persistence or projection rejection.
    pub async fn apply_directive(&self, text: &str) -> Result<CommandReceipt, CliError> {
        Ok(self.handle.apply_directive(text).await?)
    }
    /// Obtain the next global committed record, resolving only active-session directive text.
    /// Other sessions retain their sequence envelope without materializing private payloads.
    /// # Errors
    /// CAS bounds/hash/UTF-8 failure. Such a failure is surfaced; it is not skipped as success.
    pub async fn next_record(&mut self) -> Result<Option<ResolvedRecord>, CliError> {
        if self.pending_record.is_none() {
            let Some(record) = self.events.next_event().await else {
                return Ok(None);
            };
            if record.session() != self.id {
                return Ok(Some(ResolvedRecord { record, text: None }));
            }
            if let Event::DirectiveApplied { directive } = record.event() {
                let cas = Arc::clone(&self.cas);
                let cid = *directive;
                self.pending_record = Some(PendingRecord {
                    record,
                    payload: tokio::task::spawn_blocking(move || {
                        cas.get_bounded(&cid, 65_536).map(|bytes| bytes.to_vec())
                    }),
                });
            } else {
                return Ok(Some(ResolvedRecord { record, text: None }));
            }
        }

        let payload_result = {
            let pending = self
                .pending_record
                .as_mut()
                .ok_or(CliError::Unavailable("pending record disappeared"))?;
            (&mut pending.payload).await
        };
        let pending = self
            .pending_record
            .take()
            .ok_or(CliError::Unavailable("pending record disappeared"))?;
        let bytes = payload_result
            .map_err(|_| CliError::Unavailable("CAS payload owner failed"))?
            .map_err(|error| {
                CliError::Ui(format!("committed directive payload unavailable: {error}"))
            })?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| CliError::Unavailable("committed directive is not UTF-8"))?
            .to_owned();
        Ok(Some(ResolvedRecord {
            record: pending.record,
            text: Some(text),
        }))
    }
    /// Test one exact registry identity without applying the display-list limit.
    /// # Errors
    /// Registry/schema/query or worker failure.
    pub async fn session_exists(&self, id: SessionId) -> Result<bool, CliError> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let registry = Registry::open(&root)?;
            registry.get_session(id).map(|session| session.is_some())
        })
        .await
        .map_err(|_| CliError::Unavailable("session existence query owner failed"))?
        .map_err(|error| CliError::Ui(format!("session existence query failed: {error}")))
    }
    /// Inspect at most 100 genuine registry rows, newest first, outside the renderer.
    /// # Errors
    /// Registry/schema/query or worker failure.
    pub async fn list_sessions(&self) -> Result<Vec<SessionMeta>, CliError> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            let registry = Registry::open(&root)?;
            registry.list_sessions(&SessionFilter {
                limit: Some(100),
                ..SessionFilter::default()
            })
        })
        .await
        .map_err(|_| CliError::Unavailable("registry query owner failed"))?
        .map_err(|e| CliError::Ui(format!("session registry query failed: {e}")))
    }
    /// Stop admissions, persist pause and join/reap kernel-owned resources.
    /// # Errors
    /// Any owner/deadline/journal/cleanup failure remains an error.
    pub async fn close(&mut self) -> Result<(), CliError> {
        let shutdown_result = self.handle.shutdown().await;
        let payload_result = if let Some(pending) = self.pending_record.as_mut() {
            match (&mut pending.payload).await {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(error)) => Err(CliError::Ui(format!(
                    "CAS payload failed during close: {error}"
                ))),
                Err(_) => Err(CliError::Unavailable(
                    "CAS payload owner failed during close",
                )),
            }
        } else {
            Ok(())
        };
        self.pending_record = None;
        let kernel_result = shutdown_result
            .map_err(CliError::from)
            .and_then(|report| check_shutdown_report(&report));
        match (kernel_result, payload_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(kernel), Err(payload)) => Err(CliError::Ui(format!("{kernel}; {payload}"))),
        }
    }
}

fn check_shutdown_report(report: &polaris_kernel::reaper::ShutdownReport) -> Result<(), CliError> {
    if report.deadline_exceeded
        || report.journal_error.is_some()
        || report.entries.iter().any(|entry| {
            !matches!(
                entry.outcome,
                polaris_kernel::reaper::Outcome::Released | polaris_kernel::reaper::Outcome::Forced
            )
        })
    {
        return Err(CliError::Unavailable(
            "UI session shutdown left unresolved resources",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{OfflineSession, PendingRecord};
    use polaris_sdk::{contracts::Event, ids::SessionId, registry::EventSource};
    use std::{sync::Arc, time::Duration};

    #[tokio::test]
    async fn cancelled_payload_poll_retains_record_and_joins_one_cas_job()
    -> Result<(), Box<dyn std::error::Error>> {
        let state = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let mut session =
            OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
        session.apply_directive("$websearch(off)").await?;
        let record = loop {
            let Some(record) = session.events.next_event().await else {
                return Err("event stream ended before directive".into());
            };
            if matches!(record.event(), Event::DirectiveApplied { .. }) {
                break record;
            }
        };
        let cid = match record.event() {
            Event::DirectiveApplied { directive } => directive.clone(),
            _ => return Err("expected directive event".into()),
        };
        let cas = Arc::clone(&session.cas);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        session.pending_record = Some(PendingRecord {
            record,
            payload: tokio::task::spawn_blocking(move || {
                let _ = started_tx.send(());
                let _ = release_rx.blocking_recv();
                cas.get_bounded(&cid, 65_536).map(|bytes| bytes.to_vec())
            }),
        });
        started_rx.await?;

        let cancelled = tokio::time::timeout(Duration::from_millis(5), session.next_record()).await;
        if cancelled.is_ok() {
            return Err("CAS read unexpectedly completed before cancellation".into());
        }
        release_tx
            .send(())
            .map_err(|_| "CAS read receiver disappeared")?;

        let resolved = session
            .next_record()
            .await?
            .ok_or("retained directive record was lost")?;
        if resolved.text.as_deref() != Some("$websearch(off)") {
            return Err("retained directive payload changed".into());
        }
        session.close().await?;
        Ok(())
    }
    #[tokio::test]
    async fn failed_payload_owner_is_consumed_once_before_close()
    -> Result<(), Box<dyn std::error::Error>> {
        let state = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let mut session =
            OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
        session.apply_directive("$websearch(off)").await?;
        let record = loop {
            let record = session.events.next_event().await.ok_or("stream ended")?;
            if matches!(record.event(), Event::DirectiveApplied { .. }) {
                break record;
            }
        };
        let (_release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let worker = tokio::spawn(async move {
            let _released = release_rx.await;
            Ok(Vec::<u8>::new())
        });
        worker.abort();
        session.pending_record = Some(PendingRecord {
            record,
            payload: worker,
        });
        let failed = session.next_record().await.is_err();
        let cleared = session.pending_record.is_none();
        let cleanup = session.close().await;
        assert!(failed, "failed worker must be reported");
        assert!(
            cleared,
            "completed JoinHandle cannot be awaited twice during close"
        );
        cleanup?;
        Ok(())
    }
    #[tokio::test]
    async fn unrelated_session_payload_is_not_materialized()
    -> Result<(), Box<dyn std::error::Error>> {
        let state = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let other_id = SessionId::new_v7();
        let mut other = OfflineSession::open(state.path(), workspace.path(), other_id).await?;
        other.apply_directive("$websearch(off)").await?;
        let cid = loop {
            let record = other.events.next_event().await.ok_or("stream ended")?;
            if let Event::DirectiveApplied { directive } = record.event() {
                break *directive;
            }
        };
        let blob_path = other.cas.blob_path(&cid);
        other.close().await?;
        drop(other);
        std::fs::remove_file(blob_path)?;
        let mut active =
            OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
        let observed = loop {
            match active.next_record().await {
                Ok(Some(record))
                    if record.record.session() == other_id
                        && matches!(record.record.event(), Event::DirectiveApplied { .. }) =>
                {
                    break Ok(record);
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    break Err(crate::CliError::Unavailable(
                        "stream ended before other session envelope",
                    ));
                }
                Err(error) => break Err(error),
            }
        };
        let cleanup = active.close().await;
        let observed = observed?;
        assert!(
            observed.text.is_none(),
            "other-session payload must stay unresolved"
        );
        cleanup?;
        Ok(())
    }
}
