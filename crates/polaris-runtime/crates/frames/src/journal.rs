//! Durable frame-step records: prepare bounded bytes, verify CAS references, then publish events.

use core::{fmt, future::Future};
use std::io::{self, Write};

use polaris_sdk::{
    contracts::{Event, EventRecord, FrameStepOutcome, Timestamp},
    error::{ErrorKind, HasKind},
    ids::{Cid, FrameId, Seq, SessionId},
    registry::{ContentStore, ContentStoreError, EventStore, EventStoreError},
};
use polaris_system_one_watchdog::{NegativeConstraint, StepRecord};
use serde::{Deserialize, Serialize};

use crate::{AssistantResponseCapture, ExecutedToolResult, StepOutcome};

/// Current JSON schema version for content-addressed frame-step transcripts.
pub const FRAME_STEP_RECORD_SCHEMA_VERSION: u16 = 1;

const DEFAULT_MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_TOOL_RESULT_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_MAX_TOOL_RESULTS: usize = 256;
const HARD_MAX_RECORD_BYTES: usize = 64 * 1024 * 1024;
const HARD_MAX_TOOL_RESULT_BYTES: usize = 16 * 1024 * 1024;
const HARD_MAX_TOOL_RESULTS: usize = 4096;

/// Configurable bounds for preparing a durable frame-step transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameJournalLimits {
    /// Maximum encoded transcript size.
    pub max_record_bytes: usize,
    /// Maximum model-visible content bytes for each tool result.
    pub max_tool_result_bytes: usize,
    /// Maximum number of tool results in the complete record.
    pub max_tool_results: usize,
}

impl Default for FrameJournalLimits {
    fn default() -> Self {
        Self {
            max_record_bytes: DEFAULT_MAX_RECORD_BYTES,
            max_tool_result_bytes: DEFAULT_MAX_TOOL_RESULT_BYTES,
            max_tool_results: DEFAULT_MAX_TOOL_RESULTS,
        }
    }
}

impl FrameJournalLimits {
    /// Reject zero bounds, values above hard ceilings, and inconsistent byte limits.
    ///
    /// # Errors
    /// Returns [`FrameJournalError::InvalidLimits`] when a value is zero, over its ceiling, or
    /// when the tool-result byte limit exceeds the whole-record limit.
    pub fn validate(self) -> Result<(), FrameJournalError> {
        if self.max_record_bytes == 0
            || self.max_tool_result_bytes == 0
            || self.max_tool_results == 0
            || self.max_record_bytes > HARD_MAX_RECORD_BYTES
            || self.max_tool_result_bytes > HARD_MAX_TOOL_RESULT_BYTES
            || self.max_tool_results > HARD_MAX_TOOL_RESULTS
            || self.max_tool_result_bytes > self.max_record_bytes
        {
            return Err(FrameJournalError::InvalidLimits);
        }
        Ok(())
    }
}

/// Durable contents and final disposition of one reserved frame step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub executed_tool_results: Vec<ExecutedToolResult>,
    /// Optional bounded hash-only watchdog state for restart recovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watchdog_memory: Option<WatchdogFrameMemory>,
    /// Final disposition, including cancellation and failure paths.
    pub outcome: FrameStepOutcome,
}

/// Hash-only deterministic watchdog state captured at a frame-step boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchdogFrameMemory {
    /// Snapshot format version.
    pub version: u16,
    /// Frame that owns this advisory memory.
    pub frame: FrameId,
    /// Latest reserved frame step represented by the snapshot.
    pub step: u32,
    /// Recent hash-only records in increasing step order.
    pub recent: Vec<StepRecord>,
    /// Bounded rejected-hypothesis fingerprints.
    pub negative_constraints: Vec<NegativeConstraint>,
}

impl WatchdogFrameMemory {
    /// Copy bounded hash-only memory from the secret-free runtime view.
    #[must_use]
    pub fn from_view(view: &crate::FrameView) -> Option<Self> {
        if view.watchdog_recent.is_empty() && view.negative_constraints.is_empty() {
            return None;
        }
        Some(Self {
            version: 1,
            frame: view.id,
            step: view.steps,
            recent: view.watchdog_recent.clone(),
            negative_constraints: view.negative_constraints.clone(),
        })
    }

    pub(crate) fn validate(&self, expected_frame: FrameId) -> Result<(), FrameJournalError> {
        if self.version != 1
            || self.frame != expected_frame
            || self.step == 0
            || self.recent.len() > 128
            || self.negative_constraints.len() > 256
            || self.recent.iter().any(|record| {
                record.index.get() == 0
                    || record.index.get() > self.step
                    || record.shingles.len() > 4096
            })
            || self
                .negative_constraints
                .iter()
                .any(|entry| entry.rejected_at.get() == 0 || entry.rejected_at.get() > self.step)
            || self
                .recent
                .windows(2)
                .any(|pair| pair[0].index >= pair[1].index)
        {
            return Err(FrameJournalError::InvalidWatchdogMemory);
        }
        Ok(())
    }
}

/// Reconstruct the newest available hash-only watchdog snapshot from newest-first frame records.
/// Legacy records without the optional field are skipped. Every record and snapshot remains
/// frame-bound, schema-versioned, and within the configured transcript bound.
/// # Errors
/// Rejects malformed, foreign-frame, unsupported, or unbounded records.
pub fn reconstruct_watchdog_memory<'a>(
    records_newest_first: impl IntoIterator<Item = &'a [u8]>,
    expected_frame: FrameId,
    limits: FrameJournalLimits,
) -> Result<Option<WatchdogFrameMemory>, FrameJournalError> {
    limits.validate()?;
    for bytes in records_newest_first {
        if bytes.len() > limits.max_record_bytes {
            return Err(FrameJournalError::RecordTooLarge {
                limit: limits.max_record_bytes,
            });
        }
        let record: FrameStepRecord =
            serde_json::from_slice(bytes).map_err(|_| FrameJournalError::Encoding)?;
        if record.schema != FRAME_STEP_RECORD_SCHEMA_VERSION || record.frame != expected_frame {
            return Err(FrameJournalError::InvalidWatchdogMemory);
        }
        if let Some(memory) = record.watchdog_memory {
            memory.validate(expected_frame)?;
            if memory.step > record.step {
                return Err(FrameJournalError::InvalidWatchdogMemory);
            }
            return Ok(Some(memory));
        }
    }
    Ok(None)
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolResultSnapshot {
    receipt_cid: Cid,
    content_cid: Cid,
    content: String,
}

/// A validated bounded transcript ready for synchronous CAS persistence.
pub struct PreparedFrameStep {
    frame: FrameId,
    step: u32,
    outcome: FrameStepOutcome,
    bytes: Vec<u8>,
    limits: FrameJournalLimits,
    tool_results: Vec<ToolResultSnapshot>,
}

impl fmt::Debug for PreparedFrameStep {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedFrameStep")
            .field("payload", &"REDACTED")
            .finish_non_exhaustive()
    }
}

impl PreparedFrameStep {
    /// Validate and encode a frame-step record without allowing the encoded buffer past its bound.
    ///
    /// # Errors
    /// Returns a typed error when limits, schema, step number, result count, result bytes, or
    /// serialized record size violate the configured contract.
    pub fn new(
        record: &FrameStepRecord,
        limits: FrameJournalLimits,
    ) -> Result<Self, FrameJournalError> {
        limits.validate()?;
        if record.schema != FRAME_STEP_RECORD_SCHEMA_VERSION {
            return Err(FrameJournalError::UnsupportedSchema(record.schema));
        }
        if let Some(memory) = &record.watchdog_memory {
            memory.validate(record.frame)?;
            if memory.step > record.step {
                return Err(FrameJournalError::InvalidWatchdogMemory);
            }
        }
        if record.step == 0 {
            return Err(FrameJournalError::InvalidStep);
        }

        let mut result_count = 0_usize;
        if let Some(StepOutcome::ToolResults { results, .. }) = &record.result {
            result_count = checked_result_count(result_count, results.len(), limits)?;
            validate_tool_results(results, limits)?;
        }
        result_count =
            checked_result_count(result_count, record.executed_tool_results.len(), limits)?;
        validate_tool_results(&record.executed_tool_results, limits)?;

        let mut writer = BoundedVecWriter::new(limits.max_record_bytes);
        let encoded = serde_json::to_writer(&mut writer, record);
        if writer.exceeded {
            return Err(FrameJournalError::RecordTooLarge {
                limit: limits.max_record_bytes,
            });
        }
        if encoded.is_err() {
            return Err(FrameJournalError::Encoding);
        }

        let mut tool_results = Vec::with_capacity(result_count);
        if let Some(StepOutcome::ToolResults { results, .. }) = &record.result {
            snapshot_tool_results(results, &mut tool_results);
        }
        snapshot_tool_results(&record.executed_tool_results, &mut tool_results);

        Ok(Self {
            frame: record.frame,
            step: record.step,
            outcome: record.outcome,
            bytes: writer.bytes,
            limits,
            tool_results,
        })
    }

    /// Frame that produced this prepared transcript.
    #[must_use]
    pub const fn frame(&self) -> FrameId {
        self.frame
    }

    /// One-based reserved step number.
    #[must_use]
    pub const fn step(&self) -> u32 {
        self.step
    }

    /// Final disposition persisted with the transcript and event.
    #[must_use]
    pub const fn outcome(&self) -> FrameStepOutcome {
        self.outcome
    }

    /// Synchronously validate references, store the transcript, and verify exact bounded readback.
    ///
    /// This method performs blocking content-store I/O. Callers on latency-sensitive or async
    /// executor threads must offload it to a blocking worker.
    ///
    /// # Errors
    /// Returns an error for a missing or corrupt tool reference, a failed CAS operation, a CID
    /// mismatch, or a bounded readback that differs from the prepared bytes.
    pub fn persist<C: ContentStore>(&self, content: &C) -> Result<Cid, FrameJournalError> {
        verify_tool_result_snapshots(content, &self.tool_results, self.limits)?;
        let expected_cid = hash_cid(&self.bytes);
        let stored_cid = content.put(&self.bytes)?;
        if stored_cid != expected_cid {
            return Err(FrameJournalError::TranscriptCidMismatch);
        }
        let max_bytes = u64::try_from(self.limits.max_record_bytes)
            .map_err(|_| FrameJournalError::InvalidLimits)?;
        let readback = content.get(&stored_cid, max_bytes)?;
        if readback.as_ref() != self.bytes.as_slice() || hash_cid(&readback) != expected_cid {
            return Err(FrameJournalError::TranscriptReadbackMismatch);
        }
        Ok(stored_cid)
    }
}

fn checked_result_count(
    current: usize,
    added: usize,
    limits: FrameJournalLimits,
) -> Result<usize, FrameJournalError> {
    let count = current
        .checked_add(added)
        .ok_or(FrameJournalError::TooManyToolResults {
            limit: limits.max_tool_results,
        })?;
    if count > limits.max_tool_results {
        return Err(FrameJournalError::TooManyToolResults {
            limit: limits.max_tool_results,
        });
    }
    Ok(count)
}

fn validate_tool_results(
    results: &[ExecutedToolResult],
    limits: FrameJournalLimits,
) -> Result<(), FrameJournalError> {
    for entry in results {
        let size = entry.result.content.len();
        if size > limits.max_tool_result_bytes {
            return Err(FrameJournalError::ToolResultTooLarge {
                limit: limits.max_tool_result_bytes,
            });
        }
    }
    Ok(())
}

fn snapshot_tool_results(results: &[ExecutedToolResult], snapshots: &mut Vec<ToolResultSnapshot>) {
    snapshots.extend(results.iter().map(|entry| ToolResultSnapshot {
        receipt_cid: entry.result.cid,
        content_cid: entry.result.content_cid,
        content: entry.result.content.clone(),
    }));
}

fn verify_tool_result_snapshots<C: ContentStore>(
    content: &C,
    snapshots: &[ToolResultSnapshot],
    limits: FrameJournalLimits,
) -> Result<(), FrameJournalError> {
    let max_bytes = u64::try_from(limits.max_tool_result_bytes)
        .map_err(|_| FrameJournalError::InvalidLimits)?;
    for snapshot in snapshots {
        if !content.contains(&snapshot.receipt_cid)? || !content.contains(&snapshot.content_cid)? {
            return Err(FrameJournalError::ReferencedContentMissing);
        }
        let receipt = content.get(&snapshot.receipt_cid, max_bytes)?;
        if receipt.len() > limits.max_tool_result_bytes {
            return Err(FrameJournalError::ReferencedContentTooLarge {
                limit: limits.max_tool_result_bytes,
            });
        }
        if hash_cid(&receipt) != snapshot.receipt_cid {
            return Err(FrameJournalError::ReferencedContentMismatch);
        }
        let visible = content.get(&snapshot.content_cid, max_bytes)?;
        if visible.len() > limits.max_tool_result_bytes {
            return Err(FrameJournalError::ReferencedContentTooLarge {
                limit: limits.max_tool_result_bytes,
            });
        }
        if hash_cid(&visible) != snapshot.content_cid
            || visible.as_ref() != snapshot.content.as_bytes()
        {
            return Err(FrameJournalError::ReferencedContentMismatch);
        }
    }
    Ok(())
}

fn hash_cid(bytes: &[u8]) -> Cid {
    Cid::from_bytes(*blake3::hash(bytes).as_bytes())
}

struct BoundedVecWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}

impl BoundedVecWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded: false,
        }
    }
}

impl Write for BoundedVecWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(next_len) = self.bytes.len().checked_add(bytes.len()) else {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "bounded transcript",
            ));
        };
        if next_len > self.limit {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "bounded transcript",
            ));
        }
        let capacity = self.bytes.capacity().min(self.limit);
        if next_len > capacity {
            let mut target_capacity = capacity.max(1).min(self.limit);
            while target_capacity < next_len {
                target_capacity = match target_capacity.checked_mul(2) {
                    Some(doubled) => doubled.min(self.limit),
                    None => self.limit,
                };
            }
            let additional = target_capacity
                .checked_sub(self.bytes.len())
                .ok_or_else(|| io::Error::other("transcript capacity underflow"))?;
            self.bytes
                .try_reserve_exact(additional)
                .map_err(|_| io::Error::other("transcript allocation failed"))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A concrete journal over the SDK CAS and append-only event-store contracts.
///
/// `record` performs synchronous CAS reads and writes while polled; callers needing executor
/// responsiveness must offload this adapter to a blocking worker.
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
        let prepared = PreparedFrameStep::new(record, FrameJournalLimits::default())?;
        let transcript = prepared.persist(&self.content)?;
        let event = Event::FrameStepRecorded {
            frame: prepared.frame(),
            step: prepared.step(),
            transcript,
            outcome: prepared.outcome(),
        };
        let envelope = EventRecord::new(Seq::ZERO, Timestamp::now(), self.session, event);
        let _sequence = self.events.append(&envelope)?;
        Ok(transcript)
    }
}

/// Durable journal failures.
#[derive(Debug, thiserror::Error)]
pub enum FrameJournalError {
    /// CAS read or write failed; no event was appended.
    #[error(transparent)]
    Content(#[from] ContentStoreError),
    /// Event append failed after the transcript was stored in CAS.
    #[error(transparent)]
    Event(#[from] EventStoreError),
    /// The supplied limits are zero, inconsistent, or exceed a hard ceiling.
    #[error("invalid frame journal limits")]
    InvalidLimits,
    /// A reserved frame step number must be one-based.
    #[error("frame step number must be nonzero")]
    InvalidStep,
    /// The record exceeds its configured encoded-byte limit.
    #[error("frame transcript exceeds configured byte limit ({limit})")]
    RecordTooLarge {
        /// Maximum accepted encoded transcript bytes.
        limit: usize,
    },
    /// The tool result count exceeds the configured maximum.
    #[error("frame transcript exceeds configured tool result count ({limit})")]
    TooManyToolResults {
        /// Maximum accepted tool result count.
        limit: usize,
    },
    /// A model-visible tool result exceeds its configured byte limit.
    #[error("tool result exceeds configured byte limit ({limit})")]
    ToolResultTooLarge {
        /// Maximum accepted result bytes.
        limit: usize,
    },
    /// A referenced tool result blob is not present in CAS.
    #[error("tool result content reference is missing")]
    ReferencedContentMissing,
    /// A referenced tool result blob does not match its CID or expected visible text.
    #[error("tool result content reference does not match its bytes")]
    ReferencedContentMismatch,
    /// A content store returned a receipt larger than the requested bounded read.
    #[error("tool result reference exceeds configured byte limit ({limit})")]
    ReferencedContentTooLarge {
        /// Maximum accepted referenced blob bytes.
        limit: usize,
    },
    /// The transcript did not satisfy the JSON encoding contract.
    #[error("frame transcript encoding failed")]
    Encoding,
    /// A transcript version this implementation cannot safely persist.
    #[error("unsupported frame transcript schema: {0}")]
    UnsupportedSchema(u16),
    /// Hash-only watchdog recovery state is malformed, foreign, or outside hard bounds.
    #[error("invalid watchdog frame memory")]
    InvalidWatchdogMemory,
    /// CAS returned an identifier that does not hash the encoded transcript.
    #[error("content store returned a mismatched transcript identifier")]
    TranscriptCidMismatch,
    /// Bounded transcript readback differed from the bytes persisted or their hash.
    #[error("persisted transcript readback did not match the encoded bytes")]
    TranscriptReadbackMismatch,
}

impl HasKind for FrameJournalError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Content(error) => error.kind(),
            Self::Event(error) => error.kind(),
            Self::ReferencedContentMissing
            | Self::ReferencedContentMismatch
            | Self::ReferencedContentTooLarge { .. } => ErrorKind::Fatal,
            Self::InvalidLimits
            | Self::InvalidStep
            | Self::RecordTooLarge { .. }
            | Self::TooManyToolResults { .. }
            | Self::ToolResultTooLarge { .. }
            | Self::Encoding
            | Self::UnsupportedSchema(_)
            | Self::InvalidWatchdogMemory
            | Self::TranscriptCidMismatch
            | Self::TranscriptReadbackMismatch => ErrorKind::Fatal,
        }
    }
}
