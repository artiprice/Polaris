//! Trusted asynchronous adapter over the real local kernel, registry and bounded CAS.
use crate::{SessionHostError, queries};
use polaris_kernel::storage::cas::{Cas, CasConfig, CasError};
use polaris_kernel::storage::registry::SessionMeta;
use polaris_kernel::{Kernel, KernelEvents, KernelHandle, LocalKernelConfig};
use polaris_sdk::{
    contracts::{Event, EventRecord},
    ids::{Cid, FrameId, Seq, SessionId},
    registry::{CommandReceipt, EventSource, KernelApi},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Upper bound of any display text read back from CAS (matches the kernel's control limit).
const MAX_TEXT_BYTES: u64 = 65_536;
/// Registry rows returned by [`OfflineSession::list_sessions`].
const SESSION_LIST_LIMIT: u32 = 100;

/// Which committed event a [`ResolvedRecord`]'s text belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordTextKind {
    /// Canonical text of a `DirectiveApplied` event.
    Directive,
    /// Verbatim text of a `UserInputSubmitted` event.
    UserInput,
    /// The model's reply of a `FrameStepRecorded` step, served by `polarisd` to attached
    /// frontends (a local session streams its replies through [`crate::ChatProgress`] instead).
    Assistant,
}

#[derive(Debug)]
struct PendingRecord {
    record: EventRecord,
    kind: RecordTextKind,
    payload: tokio::task::JoinHandle<Result<Vec<u8>, CasError>>,
}

/// A committed domain record and a validated optional display payload.
#[derive(Debug)]
pub struct ResolvedRecord {
    /// Original fixed-size authoritative envelope; sequence is never synthesized.
    pub record: EventRecord,
    /// Bounded UTF-8 text for a directive or typed user input of the active session, resolved
    /// by the adapter before projection. `None` for every other record.
    pub text: Option<String>,
    /// Which kind of text `text` holds; `Some` exactly when `text` is `Some`.
    pub text_kind: Option<RecordTextKind>,
}

/// The CAS reference and kind of the display text an event carries, if any.
fn text_target(event: &Event) -> Option<(RecordTextKind, Cid)> {
    if let Event::DirectiveApplied { directive } = event {
        Some((RecordTextKind::Directive, *directive))
    } else if let Event::UserInputSubmitted { text } = event {
        Some((RecordTextKind::UserInput, *text))
    } else {
        None
    }
}

/// `true` exactly when `text` is one session directive and nothing else: no prose, slash command
/// or mention. Text the directive grammar rejects is not a directive and counts as prose.
#[must_use]
pub fn is_single_directive(text: &str) -> bool {
    polaris_kernel::directives::parse(text, &polaris_kernel::directives::ParseLimits::default())
        .is_ok_and(|parsed| {
            parsed.directives.len() == 1
                && parsed.command.is_none()
                && parsed.mentions.is_empty()
                && parsed.prose.trim().is_empty()
        })
}

/// One real local session and its ordered replay/live stream. Model replies come from
/// [`crate::ChatAgent`] running on [`Self::kernel`].
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
    pub async fn open(
        root: &Path,
        workspace: &Path,
        id: SessionId,
    ) -> Result<Self, SessionHostError> {
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
                    Err(cleanup) => Err(SessionHostError::Ui(format!(
                        "session initialization failed: {primary}; cleanup also failed: {cleanup}"
                    ))),
                },
                Err(cleanup) => Err(SessionHostError::Ui(format!(
                    "session initialization failed: {primary}; cleanup also failed: {cleanup}"
                ))),
            },
        }
    }
    async fn initialize(handle: KernelHandle, id: SessionId) -> Result<Self, SessionHostError> {
        let root = handle.context().root().to_owned();
        let ctx = Arc::clone(handle.context());
        let metadata = tokio::task::spawn_blocking(move || ctx.session_meta(id))
            .await
            .map_err(|_| SessionHostError::Unavailable("session metadata owner failed"))??;
        let workspace = metadata
            .ok_or(SessionHostError::Unavailable(
                "active session metadata is missing",
            ))?
            .workspace;
        let cas_root = root.join("cas");
        let cas = tokio::task::spawn_blocking(move || Cas::open(CasConfig::new(cas_root)))
            .await
            .map_err(|_| SessionHostError::Unavailable("CAS reader owner failed"))?
            .map_err(|e| SessionHostError::Ui(format!("CAS reader failed: {e}")))?;
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
    /// The session's kernel, for agent frames that must commit into this same log.
    #[must_use]
    pub const fn kernel(&self) -> &KernelHandle {
        &self.handle
    }
    /// Commit exactly one directive through the signed kernel admission/verifier path.
    /// # Errors
    /// Grammar, bounds, authorization, queue, persistence or projection rejection.
    pub async fn apply_directive(&self, text: &str) -> Result<CommandReceipt, SessionHostError> {
        Ok(self.handle.apply_directive(text).await?)
    }
    /// Record one typed user message verbatim through the signed kernel admission/verifier path.
    /// Nothing is parsed or executed; no model reply is implied.
    /// # Errors
    /// Blank or oversize text, authorization, queue, persistence or projection rejection.
    pub async fn submit_user_input(&self, text: &str) -> Result<CommandReceipt, SessionHostError> {
        Ok(self.handle.submit_user_input(text).await?)
    }
    /// Route one submitted line: exactly one directive with no prose, slash command or mention
    /// goes through [`Self::apply_directive`]; everything else (including text the directive
    /// grammar rejects) is recorded with [`Self::submit_user_input`].
    /// # Errors
    /// As the chosen path.
    pub async fn submit(&self, text: &str) -> Result<CommandReceipt, SessionHostError> {
        if is_single_directive(text) {
            self.apply_directive(text).await
        } else {
            self.submit_user_input(text).await
        }
    }
    /// Obtain the next global committed record, resolving only active-session directive and
    /// user-input text. Other sessions retain their sequence envelope without materializing
    /// private payloads.
    /// # Errors
    /// CAS bounds/hash/UTF-8 failure. Such a failure is surfaced; it is not skipped as success.
    pub async fn next_record(&mut self) -> Result<Option<ResolvedRecord>, SessionHostError> {
        if self.pending_record.is_none() {
            let Some(record) = self.events.next_event().await else {
                return Ok(None);
            };
            if record.session() != self.id {
                return Ok(Some(ResolvedRecord {
                    record,
                    text: None,
                    text_kind: None,
                }));
            }
            if let Some((kind, cid)) = text_target(record.event()) {
                let cas = Arc::clone(&self.cas);
                self.pending_record = Some(PendingRecord {
                    record,
                    kind,
                    payload: tokio::task::spawn_blocking(move || {
                        cas.get_bounded(&cid, MAX_TEXT_BYTES)
                            .map(|bytes| bytes.to_vec())
                    }),
                });
            } else {
                return Ok(Some(ResolvedRecord {
                    record,
                    text: None,
                    text_kind: None,
                }));
            }
        }

        let payload_result = {
            let pending = self
                .pending_record
                .as_mut()
                .ok_or(SessionHostError::Unavailable("pending record disappeared"))?;
            (&mut pending.payload).await
        };
        let pending = self
            .pending_record
            .take()
            .ok_or(SessionHostError::Unavailable("pending record disappeared"))?;
        let bytes = payload_result
            .map_err(|_| SessionHostError::Unavailable("CAS payload owner failed"))?
            .map_err(|error| {
                SessionHostError::Ui(match pending.kind {
                    RecordTextKind::Directive => {
                        format!("committed directive payload unavailable: {error}")
                    }
                    RecordTextKind::UserInput | RecordTextKind::Assistant => {
                        format!("committed user input payload unavailable: {error}")
                    }
                })
            })?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| {
                SessionHostError::Unavailable(match pending.kind {
                    RecordTextKind::Directive => "committed directive is not UTF-8",
                    RecordTextKind::UserInput | RecordTextKind::Assistant => {
                        "committed user input is not UTF-8"
                    }
                })
            })?
            .to_owned();
        Ok(Some(ResolvedRecord {
            record: pending.record,
            text: Some(text),
            text_kind: Some(pending.kind),
        }))
    }
    /// Test one exact registry identity without applying the display-list limit.
    /// # Errors
    /// Registry/schema/query or worker failure.
    pub async fn session_exists(&self, id: SessionId) -> Result<bool, SessionHostError> {
        queries::session_exists(&self.root, id).await
    }
    /// Inspect at most 100 genuine registry rows, newest first, outside the renderer.
    /// # Errors
    /// Registry/schema/query or worker failure.
    pub async fn list_sessions(&self) -> Result<Vec<SessionMeta>, SessionHostError> {
        queries::list_sessions(&self.root, SESSION_LIST_LIMIT).await
    }
    /// Stop admissions, persist pause and join/reap kernel-owned resources.
    /// # Errors
    /// Any owner/deadline/journal/cleanup failure remains an error.
    pub async fn close(&mut self) -> Result<(), SessionHostError> {
        let shutdown_result = self.handle.shutdown().await;
        let payload_result = if let Some(pending) = self.pending_record.as_mut() {
            match (&mut pending.payload).await {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(error)) => Err(SessionHostError::Ui(format!(
                    "CAS payload failed during close: {error}"
                ))),
                Err(_) => Err(SessionHostError::Unavailable(
                    "CAS payload owner failed during close",
                )),
            }
        } else {
            Ok(())
        };
        self.pending_record = None;
        let kernel_result = shutdown_result
            .map_err(SessionHostError::from)
            .and_then(|report| check_shutdown_report(&report));
        match (kernel_result, payload_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(kernel), Err(payload)) => {
                Err(SessionHostError::Ui(format!("{kernel}; {payload}")))
            }
        }
    }
}

fn check_shutdown_report(
    report: &polaris_kernel::reaper::ShutdownReport,
) -> Result<(), SessionHostError> {
    if report.deadline_exceeded
        || report.journal_error.is_some()
        || report.entries.iter().any(|entry| {
            !matches!(
                entry.outcome,
                polaris_kernel::reaper::Outcome::Released | polaris_kernel::reaper::Outcome::Forced
            )
        })
    {
        return Err(SessionHostError::Unavailable(
            "UI session shutdown left unresolved resources",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-ui/crates/session/src/offline.rs"]
mod tests;
