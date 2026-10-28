//! Bounded frame transcripts persisted through the kernel's actual CAS and durable event bus.
//!
//! Preparation is bounded CPU/serialization work at this allocating inspection boundary. Store
//! I/O runs on a blocking owner with the same global admission and shutdown guards as tools.
//! Once that owner is spawned, dropping the caller does not cancel an accepted audit record.
//! An event-publication failure after CAS persistence can leave an orphan; no rollback is claimed.
use crate::{KernelError, KernelHandle};
use polaris_runtime::frames::{
    FrameJournal, FrameJournalError, FrameJournalLimits, FrameStepRecord, FrameToolBinding,
    PreparedFrameStep,
};
use polaris_sdk::{
    contracts::Event,
    error::{ErrorKind, HasKind},
    ids::Cid,
    registry::ToolError,
};
use std::sync::Arc;

/// Actual frame journal bound to one kernel session and sealed active-frame identity.
///
/// This is an audit sink, not proof that a provider or independent verifier was correct. Each
/// successful call publishes one event; this API does not provide idempotent retry semantics.
/// The frame driver owns step reservations and their order. This sink records the supplied
/// attempt metadata; it does not infer step order or independently establish verification.
pub struct KernelFrameJournal {
    kernel: KernelHandle,
    binding: FrameToolBinding,
    limits: FrameJournalLimits,
    private_checkpoint: Option<(
        Arc<crate::PrivateFrameCheckpointStore>,
        Arc<polaris_security::pii::PiiVault>,
    )>,
}

impl std::fmt::Debug for KernelFrameJournal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("KernelFrameJournal(<redacted>)")
    }
}

impl KernelFrameJournal {
    /// Bind a frame's audit sink to the actual kernel root and explicit transcript limits.
    ///
    /// # Errors
    /// Rejects foreign root identity or invalid/higher-than-hard-ceiling journal limits.
    pub fn new(
        kernel: KernelHandle,
        binding: FrameToolBinding,
        limits: FrameJournalLimits,
    ) -> Result<Self, KernelFrameJournalError> {
        if binding.root_frame_id() != kernel.frame() {
            return Err(KernelFrameJournalError::FrameMismatch);
        }
        limits.validate()?;
        Ok(Self {
            kernel,
            binding,
            limits,
            private_checkpoint: None,
        })
    }

    /// Require encrypted marker-map recovery persistence before each transcript event.
    /// Accepted blocking owners retain both key and vault through checkpoint and publication.
    /// # Errors
    /// Rejects a store or marker vault belonging to another actual session/frame.
    pub fn with_private_checkpoints(
        mut self,
        store: Arc<crate::PrivateFrameCheckpointStore>,
        vault: Arc<polaris_security::pii::PiiVault>,
    ) -> Result<Self, KernelFrameJournalError> {
        if !store.matches(&self.kernel, self.binding.frame_id(), &vault) {
            return Err(KernelFrameJournalError::FrameMismatch);
        }
        self.private_checkpoint = Some((store, vault));
        Ok(self)
    }
}

impl FrameJournal for KernelFrameJournal {
    type Error = KernelFrameJournalError;

    async fn record(&self, record: &FrameStepRecord) -> Result<Cid, Self::Error> {
        if record.frame != self.binding.frame_id() {
            return Err(KernelFrameJournalError::FrameMismatch);
        }
        let owner = self.kernel.inspection_owner().await?;
        // Acquire the global owner slot before allocating encoded/snapshot buffers, so queued
        // calls cannot bypass admission's bound on additional prepared-transcript memory.
        let prepared = PreparedFrameStep::new(record, self.limits)?;
        let context = Arc::clone(self.kernel.context());
        let private_checkpoint = self.private_checkpoint.clone();
        tokio::task::spawn_blocking(move || {
            // Successful admission owns a read guard. Shutdown closes new admission but cannot
            // close the stores until this owner returns. Finish already-accepted audit records
            // even when shared command admission has since closed.
            context.ensure_open()?;
            let transcript = prepared.persist(owner.cas())?;
            if let Some((store, vault)) = private_checkpoint {
                store.persist(owner.cas(), &vault, prepared.step(), transcript)?;
            }
            let _writer = context.lock_writer()?;
            context.ensure_open()?;
            let _published = context.publish(
                owner.session(),
                Event::FrameStepRecorded {
                    frame: prepared.frame(),
                    step: prepared.step(),
                    transcript,
                    outcome: prepared.outcome(),
                },
            )?;
            Ok(transcript)
        })
        .await
        .map_err(|_| KernelFrameJournalError::OwnerFailed)?
    }
}

/// Sanitized errors from frame-bound audit admission, CAS verification or event publication.
#[derive(Debug, thiserror::Error)]
pub enum KernelFrameJournalError {
    /// Required encrypted recovery persistence failed; no transcript event was published.
    #[error(transparent)]
    PrivateCheckpoint(#[from] crate::PrivateCheckpointError),
    /// A transcript or sealed binding belongs to a different frame/kernel root.
    #[error("frame journal identity mismatch")]
    FrameMismatch,
    /// Schema, transcript/result bounds, or durable content validation failed.
    #[error(transparent)]
    Journal(#[from] FrameJournalError),
    /// The kernel stopped admitting new inspection owners.
    #[error(transparent)]
    Admission(#[from] ToolError),
    /// The actual kernel failed to publish the durable event; CAS may already contain its payload.
    #[error(transparent)]
    Publication(#[from] KernelError),
    /// The blocking owner did not return normally. Panic payloads are not exposed.
    #[error("frame journal owner failed")]
    OwnerFailed,
}

impl HasKind for KernelFrameJournalError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::PrivateCheckpoint(error) => error.kind(),
            Self::FrameMismatch => ErrorKind::Policy,
            Self::Journal(error) => error.kind(),
            Self::Admission(error) => error.kind(),
            Self::Publication(error) => error.kind(),
            Self::OwnerFailed => ErrorKind::Fatal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::KernelFrameJournal;
    use crate::{Kernel, LocalKernelConfig, storage::cas::content_id};
    use polaris_runtime::frames::{
        AgentContextFrame, AgentRole, FrameJournal, FrameJournalLimits, FrameScope,
        FrameStepRecord, ModelSelection, ResourceQuota, ScratchpadRef,
    };
    use polaris_sdk::{
        contracts::{Event, FrameStepOutcome},
        effort::Effort,
        ids::{Cid, FrameId, ModelId, Seq, SessionId},
    };
    use polaris_security::capability::{
        Authority, Clock, ResourceScope, Right, Rights, Scope, SystemClock,
    };
    use proptest::prelude::*;
    use std::{
        future::{Future, poll_fn},
        io,
        sync::{Arc, mpsc},
        task::Poll,
        time::Duration,
    };

    proptest! {
        #[test]
        fn journal_preparation_obeys_exact_escaped_json_bound(goal in ".{0,1024}", cap in 1_usize..32768) {
            let record = FrameStepRecord { schema: 1, frame: FrameId::from_u128(7), step: 1, goal, request: None, assistant_response: None, result: None, executed_tool_results: Vec::new(), watchdog_memory: None, outcome: FrameStepOutcome::Failed };
            let encoded = serde_json::to_vec(&record);
            let prepared = polaris_runtime::frames::PreparedFrameStep::new(&record, FrameJournalLimits {max_record_bytes: cap, max_tool_result_bytes: 1, max_tool_results: 1});
            match encoded {
                Ok(bytes) if bytes.len() <= cap => prop_assert!(prepared.is_ok()),
                Ok(_) => prop_assert!(matches!(prepared, Err(polaris_runtime::frames::FrameJournalError::RecordTooLarge { .. })), "oversized transcript must fail with a size error"),
                Err(_) => prop_assert!(prepared.is_err()),
            }
        }
    }

    #[tokio::test]
    async fn accepted_journal_finishes_after_future_drop_and_shutdown()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let frame_id = FrameId::new_v7();
        let session = SessionId::new_v7();
        let kernel = Kernel::start(LocalKernelConfig::new(
            root.path(),
            workspace.path(),
            session,
            frame_id,
        ))
        .await?;
        let authority = Authority::new_session()?;
        let scope = Scope {
            resource: ResourceScope::Workspace,
            max_churn_lines: 0,
            ttl: Duration::from_secs(300),
            allow_unsafe: false,
            diff_blake3: None,
            rights: Rights::new(&[Right::Read]),
        };
        let token = authority.issue_root(frame_id, &scope, &SystemClock)?;
        let expires_at_micros = SystemClock
            .now()
            .as_microsecond()
            .checked_add(300_000_000)
            .ok_or("expiry overflow")?;
        let frame = AgentContextFrame::new_root(
            frame_id,
            AgentRole::Orchestrator,
            token,
            FrameScope {
                capability: scope,
                expires_at_micros,
            },
            ResourceQuota {
                cpu_millis: 1000,
                memory_mb: 128,
                network: Default::default(),
                max_steps: 2,
                max_depth: 1,
                token_budget: 100,
            },
            ScratchpadRef::new(Cid::from_bytes([1; 32])),
        )
        .activate(
            ModelSelection {
                model: ModelId::new("test/synthetic-model")?,
                maximum_effort: Effort::Low,
            },
            Effort::Low,
        )?;
        let journal = KernelFrameJournal::new(
            kernel.clone(),
            frame.tool_binding(),
            FrameJournalLimits::default(),
        )?;
        let record = FrameStepRecord {
            schema: 1,
            frame: frame_id,
            step: 1,
            goal: "synthetic dropped journal".to_owned(),
            request: None,
            assistant_response: None,
            result: None,
            executed_tool_results: Vec::new(),
            watchdog_memory: None,
            outcome: FrameStepOutcome::Cancelled,
        };
        let expected = content_id(&serde_json::to_vec(&record)?);
        let context = Arc::clone(kernel.context());
        let blocker_context = Arc::clone(&context);
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _writer = blocker_context
                .lock_writer()
                .map_err(|_| io::Error::other("writer unavailable"))?;
            started_tx
                .send(())
                .map_err(|_| io::Error::other("start receiver dropped"))?;
            release_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| io::Error::other("journal blocker timed out"))?;
            Ok::<(), io::Error>(())
        });
        tokio::task::spawn_blocking(move || started_rx.recv_timeout(Duration::from_secs(5)))
            .await??;
        let mut pending = Box::pin(journal.record(&record));
        poll_fn(|cx| match pending.as_mut().poll(cx) {
            Poll::Pending => Poll::Ready(Ok::<(), io::Error>(())),
            Poll::Ready(_) => Poll::Ready(Err(io::Error::other("journal bypassed held writer"))),
        })
        .await?;
        // Actual CAS visibility proves the blocking owner progressed beyond persistence while
        // publication is fenced by the held writer; this is not inferred from a sleep alone.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let cas = context.cas.clone();
                if tokio::task::spawn_blocking(move || cas.contains(&expected)).await?? {
                    return Ok::<(), Box<dyn std::error::Error>>(());
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await??;
        drop(pending);
        let shutdown_kernel = kernel.clone();
        let mut shutdown = tokio::spawn(async move { shutdown_kernel.shutdown().await });
        let early = tokio::time::timeout(Duration::from_millis(25), &mut shutdown)
            .await
            .is_ok();
        release_tx.send(())?;
        blocker.await??;
        assert!(!early, "shutdown must wait for the accepted record");
        let _report = tokio::time::timeout(Duration::from_secs(5), shutdown).await???;
        let records = context.records(Seq::new(1), Seq::new(u64::MAX))?;
        assert!(records.iter().any(|entry| matches!(entry.event(), Event::FrameStepRecorded { transcript, .. } if *transcript == expected)), "dropped caller must not discard accepted audit publication");
        Ok(())
    }
}
