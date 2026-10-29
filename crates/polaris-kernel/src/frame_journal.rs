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
    ids::{Cid, MicroUsd, TokenCount},
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
        // A dispatched request is also accounted as one model request with its reported usage
        // (zero when the provider sent none); cost is not reported by these providers.
        let dispatched = record.request.is_some();
        let usage = record
            .assistant_response
            .as_ref()
            .and_then(|response| response.usage);
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
            if dispatched {
                let zero = TokenCount::new(0);
                let accounted = context.publish(
                    owner.session(),
                    Event::ModelRequestCompleted {
                        frame: prepared.frame(),
                        request: transcript,
                        prompt_tokens: usage.map_or(zero, |usage| usage.prompt_tokens),
                        completion_tokens: usage.map_or(zero, |usage| usage.completion_tokens),
                        cached_tokens: usage.map_or(zero, |usage| usage.cached_tokens),
                        cost: MicroUsd::new(0),
                        latency_us: 0,
                    },
                )?;
                // The registry row is a projection; a failed update is repaired on reopen.
                if let Err(error) = context.registry.add_usage(
                    owner.session(),
                    usage.map_or(0, |usage| u64::from(usage.prompt_tokens.get())),
                    usage.map_or(0, |usage| u64::from(usage.completion_tokens.get())),
                    accounted.at(),
                ) {
                    tracing::warn!(%error, "session usage projection failed");
                }
            }
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
#[path = "../../../tests/crates/polaris-kernel/src/frame_journal.rs"]
mod tests;
