use bytes::Bytes;
use polaris_runtime_frames::ExecutedToolResult;
use polaris_runtime_frames::{
    FRAME_STEP_RECORD_SCHEMA_VERSION, FrameJournal, FrameJournalError, FrameJournalLimits,
    FrameStepRecord, JournalAdapter, PreparedFrameStep, ToolResultRef,
};
use polaris_sdk::{
    content::{ContentStore, ContentStoreError},
    contracts::{Event, EventRecord, FrameStepOutcome},
    error::{ErrorKind, HasKind},
    ids::{Cid, FrameId, Seq, SessionId},
    registry::{EventStore, EventStoreError},
};
use polaris_storage_cas::{Cas, CasConfig};
use std::{
    collections::BTreeMap,
    error::Error,
    sync::{Arc, Mutex, MutexGuard},
};

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoreMode {
    Honest,
    WrongPutCid,
    CorruptTranscriptReadback,
    MissingReceipt,
    CorruptReceiptReadback,
    CorruptVisibleReadback,
    OversizedReceiptIgnoringBound,
}

#[derive(Debug, Clone)]
struct HostileContentStore {
    blobs: Arc<Mutex<BTreeMap<Cid, Vec<u8>>>>,
    mode: StoreMode,
    transcript_puts: Arc<Mutex<usize>>,
}

impl HostileContentStore {
    fn new(mode: StoreMode) -> Self {
        Self {
            blobs: Arc::new(Mutex::new(BTreeMap::new())),
            mode,
            transcript_puts: Arc::new(Mutex::new(0)),
        }
    }

    fn insert(&self, bytes: &[u8]) -> Cid {
        let cid = cid_for(bytes);
        lock_recover(&self.blobs).insert(cid, bytes.to_vec());
        cid
    }

    fn transcript_put_count(&self) -> usize {
        *lock_recover(&self.transcript_puts)
    }
}

fn cid_for(bytes: &[u8]) -> Cid {
    Cid::from_bytes(*blake3::hash(bytes).as_bytes())
}

impl ContentStore for HostileContentStore {
    fn put(&self, content: &[u8]) -> Result<Cid, ContentStoreError> {
        let mut count = lock_recover(&self.transcript_puts);
        *count = count.saturating_add(1);
        let cid = cid_for(content);
        lock_recover(&self.blobs).insert(cid, content.to_vec());
        if self.mode == StoreMode::WrongPutCid {
            return Ok(cid_for(b"different bytes"));
        }
        Ok(cid)
    }

    fn contains(&self, cid: &Cid) -> Result<bool, ContentStoreError> {
        if self.mode == StoreMode::MissingReceipt {
            let blobs = lock_recover(&self.blobs);
            if blobs.get(cid).is_some_and(|value| value == b"raw receipt") {
                return Ok(false);
            }
            return Ok(blobs.contains_key(cid));
        }
        Ok(lock_recover(&self.blobs).contains_key(cid))
    }

    fn get(&self, cid: &Cid, max_bytes: u64) -> Result<Bytes, ContentStoreError> {
        let mut value = lock_recover(&self.blobs)
            .get(cid)
            .cloned()
            .ok_or(ContentStoreError::NotFound(*cid))?;
        let size = u64::try_from(value.len()).map_err(|_| ContentStoreError::TooLarge {
            limit: max_bytes,
            size: u64::MAX,
        })?;
        if size > max_bytes && self.mode != StoreMode::OversizedReceiptIgnoringBound {
            return Err(ContentStoreError::TooLarge {
                limit: max_bytes,
                size,
            });
        }
        match self.mode {
            StoreMode::CorruptTranscriptReadback if value.first() == Some(&b'{') => {
                value.push(b'!');
            }
            StoreMode::CorruptReceiptReadback if value == b"raw receipt" => value.push(b'!'),
            StoreMode::CorruptVisibleReadback if value == b"visible text" => value.push(b'!'),
            StoreMode::CorruptReceiptReadback | StoreMode::CorruptVisibleReadback => {}
            StoreMode::OversizedReceiptIgnoringBound => {}
            StoreMode::Honest
            | StoreMode::WrongPutCid
            | StoreMode::MissingReceipt
            | StoreMode::CorruptTranscriptReadback => {}
        }
        Ok(Bytes::from(value))
    }
}

#[derive(Debug, Clone)]
struct MemoryEventStore {
    content: HostileContentStore,
    records: Arc<Mutex<Vec<EventRecord>>>,
    fail: bool,
}

impl EventStore for MemoryEventStore {
    fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError> {
        if self.fail {
            return Err(EventStoreError::Unavailable("event failure".to_owned()));
        }
        if let Event::FrameStepRecorded { transcript, .. } = record.event()
            && self.content.get(transcript, u64::MAX).is_err()
        {
            return Err(EventStoreError::Unavailable(
                "transcript missing".to_owned(),
            ));
        }
        let mut records = lock_recover(&self.records);
        records.push(*record);
        let sequence = u64::try_from(records.len())
            .map_err(|_| EventStoreError::Unavailable("sequence overflow".to_owned()))?;
        Ok(Seq::new(sequence))
    }

    fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
        Ok(lock_recover(&self.records)
            .iter()
            .filter(|record| record.seq() >= from && record.seq() <= to)
            .copied()
            .collect())
    }
}

fn sample_record() -> FrameStepRecord {
    FrameStepRecord {
        schema: FRAME_STEP_RECORD_SCHEMA_VERSION,
        frame: FrameId::from_u128(501),
        step: 1,
        goal: "inspect".to_owned(),
        request: None,
        assistant_response: None,
        result: None,
        executed_tool_results: Vec::new(),
        watchdog_memory: None,
        outcome: FrameStepOutcome::Cancelled,
    }
}

fn record_with_tool_result(store: &HostileContentStore) -> FrameStepRecord {
    record_with_receipt(store, b"raw receipt")
}

fn record_with_receipt(store: &HostileContentStore, receipt_bytes: &[u8]) -> FrameStepRecord {
    let receipt = store.insert(receipt_bytes);
    let visible = store.insert(b"visible text");
    let mut record = sample_record();
    record.executed_tool_results.push(ExecutedToolResult {
        tool_call_id: "call-1".to_owned(),
        result: ToolResultRef {
            cid: receipt,
            content_cid: visible,
            content: "visible text".to_owned(),
        },
    });
    record
}

#[test]
fn limits_default_and_reject_invalid_or_over_ceiling_values() -> Result<(), Box<dyn Error>> {
    let defaults = FrameJournalLimits::default();
    if defaults.max_record_bytes != 16 * 1024 * 1024
        || defaults.max_tool_result_bytes != 2 * 1024 * 1024
        || defaults.max_tool_results != 256
        || defaults.validate().is_err()
    {
        return Err("default limits differ from the documented bounded defaults".into());
    }
    for limits in [
        FrameJournalLimits {
            max_record_bytes: 0,
            ..defaults
        },
        FrameJournalLimits {
            max_tool_result_bytes: 0,
            ..defaults
        },
        FrameJournalLimits {
            max_tool_results: 0,
            ..defaults
        },
        FrameJournalLimits {
            max_record_bytes: 64 * 1024 * 1024 + 1,
            ..defaults
        },
        FrameJournalLimits {
            max_tool_result_bytes: 16 * 1024 * 1024 + 1,
            ..defaults
        },
        FrameJournalLimits {
            max_tool_results: 4097,
            ..defaults
        },
        FrameJournalLimits {
            max_record_bytes: 512,
            max_tool_result_bytes: 513,
            ..defaults
        },
    ] {
        if limits.validate().is_ok() {
            return Err("invalid journal limits were accepted".into());
        }
    }
    Ok(())
}

#[test]
fn prepare_rejects_invalid_schema_zero_step_and_result_count() -> Result<(), Box<dyn Error>> {
    let limits = FrameJournalLimits {
        max_tool_results: 1,
        ..FrameJournalLimits::default()
    };
    let mut record = sample_record();
    record.schema = FRAME_STEP_RECORD_SCHEMA_VERSION.saturating_add(1);
    if !matches!(
        PreparedFrameStep::new(&record, limits),
        Err(FrameJournalError::UnsupportedSchema(_))
    ) {
        return Err("unsupported schema was not rejected".into());
    }
    record = sample_record();
    record.step = 0;
    if !matches!(
        PreparedFrameStep::new(&record, limits),
        Err(FrameJournalError::InvalidStep)
    ) {
        return Err("step zero was not rejected".into());
    }
    let store = HostileContentStore::new(StoreMode::Honest);
    record = record_with_tool_result(&store);
    record.executed_tool_results.push(
        record
            .executed_tool_results
            .first()
            .ok_or("missing fixture result")?
            .clone(),
    );
    if !matches!(
        PreparedFrameStep::new(&record, limits),
        Err(FrameJournalError::TooManyToolResults { .. })
    ) {
        return Err("too many tool results were accepted".into());
    }
    Ok(())
}

#[test]
fn prepare_rejects_tool_text_and_serialized_record_over_bounds() -> Result<(), Box<dyn Error>> {
    let store = HostileContentStore::new(StoreMode::Honest);
    let mut record = record_with_tool_result(&store);
    let small = FrameJournalLimits {
        max_record_bytes: 1024,
        max_tool_result_bytes: 4,
        max_tool_results: 1,
    };
    if !matches!(
        PreparedFrameStep::new(&record, small),
        Err(FrameJournalError::ToolResultTooLarge { .. })
    ) {
        return Err("oversized model-visible text was accepted".into());
    }
    record.executed_tool_results.clear();
    record.goal = "\u{1}".repeat(40);
    let bounded = FrameJournalLimits {
        max_record_bytes: 64,
        max_tool_result_bytes: 16,
        max_tool_results: 1,
    };
    if !matches!(
        PreparedFrameStep::new(&record, bounded),
        Err(FrameJournalError::RecordTooLarge { .. })
    ) {
        return Err("escaped serialized record exceeded its bound without rejection".into());
    }
    Ok(())
}

#[test]
fn bounded_writer_accepts_exact_size_and_rejects_one_byte_less() -> Result<(), Box<dyn Error>> {
    let record = sample_record();
    let encoded = serde_json::to_vec(&record)?;
    let exact = FrameJournalLimits {
        max_record_bytes: encoded.len(),
        max_tool_result_bytes: 1,
        max_tool_results: 1,
    };
    if PreparedFrameStep::new(&record, exact).is_err() {
        return Err("record was rejected at its exact encoded size".into());
    }
    let one_short = FrameJournalLimits {
        max_record_bytes: encoded
            .len()
            .checked_sub(1)
            .ok_or("encoded fixture unexpectedly empty")?,
        ..exact
    };
    if !matches!(
        PreparedFrameStep::new(&record, one_short),
        Err(FrameJournalError::RecordTooLarge { .. })
    ) {
        return Err("record was accepted with a bound one byte below its encoded size".into());
    }
    Ok(())
}

#[test]
fn bounded_writer_matches_json_size_for_escaped_input_family() -> Result<(), Box<dyn Error>> {
    for length in 0..=64 {
        let mut record = sample_record();
        record.goal = ['x', '\u{1}', '\\'].iter().cycle().take(length).collect();
        let encoded = serde_json::to_vec(&record)?;
        let exact = FrameJournalLimits {
            max_record_bytes: encoded.len(),
            max_tool_result_bytes: 1,
            max_tool_results: 1,
        };
        if PreparedFrameStep::new(&record, exact).is_err() {
            return Err("escaped record did not fit its exact encoded-size limit".into());
        }
        let one_short = FrameJournalLimits {
            max_record_bytes: encoded
                .len()
                .checked_sub(1)
                .ok_or("encoded fixture unexpectedly empty")?,
            ..exact
        };
        if !matches!(
            PreparedFrameStep::new(&record, one_short),
            Err(FrameJournalError::RecordTooLarge { .. })
        ) {
            return Err("escaped record crossed its exact byte boundary".into());
        }
    }
    Ok(())
}

#[test]
fn prepared_debug_redacts_payload_and_persist_verifies_exact_cas_bytes()
-> Result<(), Box<dyn Error>> {
    let record = sample_record();
    let prepared = PreparedFrameStep::new(&record, FrameJournalLimits::default())?;
    let debug = format!("{prepared:?}");
    if debug.contains("inspect") || debug.contains("Cancelled") || !debug.contains("REDACTED") {
        return Err("prepared debug output exposed transcript data".into());
    }
    if prepared.frame() != record.frame
        || prepared.step() != record.step
        || prepared.outcome() != record.outcome
    {
        return Err("safe prepared metadata getters returned incorrect values".into());
    }
    let content = HostileContentStore::new(StoreMode::Honest);
    let cid = prepared.persist(&content)?;
    if cid != cid_for(&content.get(&cid, 16 * 1024 * 1024)?) {
        return Err("persist did not return the hash of the exact stored bytes".into());
    }
    Ok(())
}

#[test]
fn prepared_persist_roundtrips_through_real_cas() -> Result<(), Box<dyn Error>> {
    let path = std::env::temp_dir().join(format!("polaris-bounded-journal-{}", FrameId::new_v7()));
    std::fs::create_dir(&path)?;
    let result = (|| -> Result<(), Box<dyn Error>> {
        let cas = Cas::open(CasConfig::new(&path))?;
        let prepared = PreparedFrameStep::new(&sample_record(), FrameJournalLimits::default())?;
        let cid = prepared.persist(&cas)?;
        let bytes = cas.get(&cid)?;
        if cid_for(&bytes) != cid || bytes.is_empty() {
            return Err("real CAS did not retain the prepared transcript bytes".into());
        }
        Ok(())
    })();
    let _removed = std::fs::remove_dir_all(&path);
    result
}

#[test]
fn persist_rejects_missing_and_corrupt_receipt_blobs() -> Result<(), Box<dyn Error>> {
    let missing_store = HostileContentStore::new(StoreMode::MissingReceipt);
    let record = record_with_tool_result(&missing_store);
    let prepared = PreparedFrameStep::new(&record, FrameJournalLimits::default())?;
    if !matches!(
        prepared.persist(&missing_store),
        Err(FrameJournalError::ReferencedContentMissing)
    ) || missing_store.transcript_put_count() != 0
    {
        return Err("missing raw receipt reached transcript persistence".into());
    }
    let corrupt_store = HostileContentStore::new(StoreMode::CorruptReceiptReadback);
    let record = record_with_tool_result(&corrupt_store);
    let prepared = PreparedFrameStep::new(&record, FrameJournalLimits::default())?;
    if !matches!(
        prepared.persist(&corrupt_store),
        Err(FrameJournalError::ReferencedContentMismatch)
    ) || corrupt_store.transcript_put_count() != 0
    {
        return Err("corrupt raw receipt reached transcript persistence".into());
    }
    Ok(())
}

#[test]
fn persist_rejects_valid_hash_oversized_receipt_from_bound_violating_store()
-> Result<(), Box<dyn Error>> {
    let store = HostileContentStore::new(StoreMode::OversizedReceiptIgnoringBound);
    let record = record_with_receipt(&store, b"oversized raw receipt");
    let limits = FrameJournalLimits {
        max_record_bytes: 4096,
        max_tool_result_bytes: 12,
        max_tool_results: 1,
    };
    let prepared = PreparedFrameStep::new(&record, limits)?;
    if !matches!(
        prepared.persist(&store),
        Err(FrameJournalError::ReferencedContentTooLarge { limit: 12 })
    ) || store.transcript_put_count() != 0
    {
        return Err("valid-hash oversized receipt was allowed to reach transcript storage".into());
    }
    Ok(())
}

#[test]
fn persist_rejects_model_visible_content_mismatch() -> Result<(), Box<dyn Error>> {
    let store = HostileContentStore::new(StoreMode::CorruptVisibleReadback);
    let record = record_with_tool_result(&store);
    let prepared = PreparedFrameStep::new(&record, FrameJournalLimits::default())?;
    if !matches!(
        prepared.persist(&store),
        Err(FrameJournalError::ReferencedContentMismatch)
    ) || store.transcript_put_count() != 0
    {
        return Err("mismatched model-visible content was persisted".into());
    }
    Ok(())
}

#[test]
fn persist_rejects_wrong_put_cid_and_corrupt_readback() -> Result<(), Box<dyn Error>> {
    let prepared = PreparedFrameStep::new(&sample_record(), FrameJournalLimits::default())?;
    let wrong = HostileContentStore::new(StoreMode::WrongPutCid);
    if !matches!(
        prepared.persist(&wrong),
        Err(FrameJournalError::TranscriptCidMismatch)
    ) {
        return Err("wrong CID returned by put was accepted".into());
    }
    let corrupt = HostileContentStore::new(StoreMode::CorruptTranscriptReadback);
    if !matches!(
        prepared.persist(&corrupt),
        Err(FrameJournalError::TranscriptReadbackMismatch)
    ) {
        return Err("corrupt transcript readback was accepted".into());
    }
    Ok(())
}

#[tokio::test]
async fn journal_adapter_appends_only_after_verified_persistence() -> Result<(), Box<dyn Error>> {
    let failed_content = HostileContentStore::new(StoreMode::WrongPutCid);
    let failed_records = Arc::new(Mutex::new(Vec::new()));
    let failed_events = MemoryEventStore {
        content: failed_content.clone(),
        records: Arc::clone(&failed_records),
        fail: false,
    };
    let failed = JournalAdapter::new(failed_content, failed_events, SessionId::from_u128(600));
    if !matches!(
        failed.record(&sample_record()).await,
        Err(FrameJournalError::TranscriptCidMismatch)
    ) || !lock_recover(&failed_records).is_empty()
    {
        return Err("CAS verification failure was followed by an event".into());
    }

    let content = HostileContentStore::new(StoreMode::Honest);
    let records = Arc::new(Mutex::new(Vec::new()));
    let events = MemoryEventStore {
        content: content.clone(),
        records: Arc::clone(&records),
        fail: true,
    };
    let journal = JournalAdapter::new(content.clone(), events, SessionId::from_u128(601));
    if !matches!(
        journal.record(&sample_record()).await,
        Err(FrameJournalError::Event(_))
    ) || content.transcript_put_count() != 1
        || !lock_recover(&records).is_empty()
    {
        return Err("event failure did not leave only the verified transcript orphaned".into());
    }
    if FrameJournalError::InvalidStep.kind() != ErrorKind::Fatal {
        return Err("invalid journal input did not classify as fatal".into());
    }
    Ok(())
}
