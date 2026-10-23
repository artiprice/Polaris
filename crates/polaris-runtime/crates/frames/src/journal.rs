//! Durable frame-step records: write the complete transcript to CAS before publishing its event.

use core::future::Future;

use polaris_sdk::{
    contracts::{Event, EventRecord, FrameStepOutcome, Timestamp},
    error::{ErrorKind, HasKind},
    ids::{Cid, FrameId, Seq, SessionId},
    registry::{ContentStore, ContentStoreError, EventStore, EventStoreError},
};
use serde::Serialize;

use crate::{AssistantResponseCapture, StepOutcome};

/// Current JSON schema version for content-addressed frame-step transcripts.
pub const FRAME_STEP_RECORD_SCHEMA_VERSION: u16 = 1;

/// Durable contents and final disposition of one reserved frame step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FrameStepRecord {
    /// Durable transcript schema version.
    pub schema: u16,
    /// Frame that made the attempt.
    pub frame: FrameId,
    /// One-based reserved step number.
    pub step: u32,
    /// Goal text supplied for this attempt.
    pub goal: String,
    /// Actual provider request when assembly and dispatch completed.
    pub request: Option<polaris_sdk::registry::ChatRequest>,
    /// Bounded streamed assistant response, marked partial when no finish chunk arrived.
    pub assistant_response: Option<AssistantResponseCapture>,
    /// Verified or rejected step result; absent when cancellation/error stopped execution.
    pub result: Option<StepOutcome>,
    /// Durable tool results from interrupted or rejected attempts that are not carried by a
    /// replayable `ToolResults` outcome.
    pub executed_tool_results: Vec<crate::ExecutedToolResult>,
    /// Final disposition, including cancellation and failure paths.
    pub outcome: FrameStepOutcome,
}

/// Journaling boundary. Implementations must persist the payload before exposing its identifier.
pub trait FrameJournal: Send + Sync {
    /// Journal backend error.
    type Error: HasKind + Send;

    /// Persist a complete transcript and return its immutable content id.
    fn record(
        &self,
        record: &FrameStepRecord,
    ) -> impl Future<Output = Result<Cid, Self::Error>> + Send;
}

/// A concrete journal over the SDK CAS and append-only event-store contracts.
#[derive(Debug)]
pub struct JournalAdapter<C, E> {
    content: C,
    events: E,
    session: SessionId,
}

impl<C, E> JournalAdapter<C, E>
where
    C: ContentStore,
    E: EventStore,
{
    /// Bind the durable storage adapters to the owning session.
    #[must_use]
    pub const fn new(content: C, events: E, session: SessionId) -> Self {
        Self {
            content,
            events,
            session,
        }
    }
}

impl<C, E> FrameJournal for JournalAdapter<C, E>
where
    C: ContentStore,
    E: EventStore,
{
    type Error = FrameJournalError;

    async fn record(&self, record: &FrameStepRecord) -> Result<Cid, Self::Error> {
        if record.schema != FRAME_STEP_RECORD_SCHEMA_VERSION {
            return Err(FrameJournalError::UnsupportedSchema(record.schema));
        }
        if let Some(crate::StepOutcome::ToolResults { results, .. }) = &record.result {
            verify_tool_result_refs(&self.content, results)?;
        }
        verify_tool_result_refs(&self.content, &record.executed_tool_results)?;
        let bytes = serde_json::to_vec(record).map_err(|_| FrameJournalError::Encoding)?;
        let transcript = self.content.put(&bytes)?;
        let event = Event::FrameStepRecorded {
            frame: record.frame,
            step: record.step,
            transcript,
            outcome: record.outcome,
        };
        let envelope = EventRecord::new(Seq::ZERO, Timestamp::now(), self.session, event);
        let _sequence = self.events.append(&envelope)?;
        Ok(transcript)
    }
}

fn verify_tool_result_refs<C: ContentStore>(
    content: &C,
    results: &[crate::ExecutedToolResult],
) -> Result<(), FrameJournalError> {
    for entry in results {
        if !content.contains(&entry.result.cid)? || !content.contains(&entry.result.content_cid)? {
            return Err(FrameJournalError::ReferencedContentMissing);
        }
        let max_bytes =
            u64::try_from(entry.result.content.len()).map_err(|_| FrameJournalError::Encoding)?;
        let stored = content.get(&entry.result.content_cid, max_bytes)?;
        if stored.as_ref() != entry.result.content.as_bytes() {
            return Err(FrameJournalError::ReferencedContentMismatch);
        }
    }
    Ok(())
}

/// Durable journal failures.
#[derive(Debug, thiserror::Error)]
pub enum FrameJournalError {
    /// CAS write failed; no event was appended.
    #[error(transparent)]
    Content(#[from] ContentStoreError),
    /// Event append failed after the transcript was stored in CAS.
    #[error(transparent)]
    Event(#[from] EventStoreError),
    /// A returned tool result cid is not present in the content store.
    #[error("tool result content reference is missing")]
    ReferencedContentMissing,
    /// The durable model-visible tool content differs from the returned text.
    #[error("tool result content reference does not match its bytes")]
    ReferencedContentMismatch,
    /// The transcript did not satisfy the JSON encoding contract.
    #[error("frame transcript encoding failed")]
    Encoding,
    /// A transcript version this implementation cannot safely persist.
    #[error("unsupported frame transcript schema: {0}")]
    UnsupportedSchema(u16),
}

impl HasKind for FrameJournalError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Content(error) => error.kind(),
            Self::Event(error) => error.kind(),
            Self::ReferencedContentMissing | Self::ReferencedContentMismatch => ErrorKind::Fatal,
            Self::Encoding | Self::UnsupportedSchema(_) => ErrorKind::Fatal,
        }
    }
}
