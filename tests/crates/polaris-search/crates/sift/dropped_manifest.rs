use polaris_sdk::{
    content::{ContentStore, ContentStoreError},
    ids::Cid,
};
use polaris_search_sift::{
    Distilled, DropManifestError, DropStats, Locator, MAX_DROPPED_CHUNK_BYTES, MAX_DROPPED_CHUNKS,
    MAX_DROPPED_MANIFEST_BYTES, MAX_DROPPED_RECOVERY_BYTES, RawChunk, SiftError, SourceId, Trust,
    recover_dropped,
};
use polaris_storage_cas::{Cas, CasConfig};
use std::{collections::HashMap, error::Error, sync::Mutex};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn chunk(text: &str) -> Result<RawChunk, SiftError> {
    Ok(RawChunk {
        source: SourceId::new("fixture-source")?,
        locator: Locator::Document {
            id: "doc://fixture/42".into(),
            page: Some(7),
            line_start: Some(11),
            line_end: Some(13),
        },
        text: text.into(),
        trust: Trust::CallerValidated,
    })
}

fn distilled(dropped: Vec<RawChunk>) -> Distilled {
    let dropped_count = count_or_max(dropped.len());
    Distilled {
        kept: Vec::new(),
        stats: DropStats {
            dropped: dropped_count,
            estimated_tokens_saved: 0,
        },
        dropped,
        fallback: false,
        budget_exceeded: false,
    }
}

fn count_or_max(value: usize) -> u32 {
    let Ok(count) = u32::try_from(value) else {
        return u32::MAX;
    };
    count
}

#[test]
fn dropped_chunks_recoverable_by_ref() -> TestResult {
    let dir = tempfile::tempdir()?;
    let cas = Cas::open(CasConfig::new(dir.path().join("cas")))?;
    let expected = vec![chunk("original source text, verbatim")?];
    let persisted = distilled(expected.clone()).persist_dropped(&cas)?;

    let actual = recover_dropped(&cas, &persisted.manifest_ref)?;
    assert_eq!(
        actual, expected,
        "recovery must preserve source metadata and text"
    );
    assert_eq!(
        persisted.dropped_refs.len(),
        1,
        "one chunk produces one content ref"
    );
    Ok(())
}

#[test]
fn dropped_refs_unique() -> TestResult {
    let dir = tempfile::tempdir()?;
    let cas = Cas::open(CasConfig::new(dir.path().join("cas")))?;
    let repeated = chunk("same original source text")?;
    let expected = vec![repeated.clone(), chunk("different text")?, repeated];
    let persisted = distilled(expected.clone()).persist_dropped(&cas)?;

    assert_eq!(
        persisted.dropped_refs.len(),
        2,
        "identical chunks share one CAS ref"
    );
    assert_eq!(recover_dropped(&cas, &persisted.manifest_ref)?, expected);
    Ok(())
}

#[test]
fn drop_stats_count_matches_dropped() -> TestResult {
    let dir = tempfile::tempdir()?;
    let cas = Cas::open(CasConfig::new(dir.path().join("cas")))?;
    let expected = vec![chunk("one")?, chunk("two")?];
    let persisted = distilled(expected.clone()).persist_dropped(&cas)?;

    assert_eq!(persisted.dropped_count, expected.len());
    assert_eq!(
        recover_dropped(&cas, &persisted.manifest_ref)?.len(),
        expected.len()
    );
    let mut inconsistent = distilled(expected);
    inconsistent.stats.dropped = 0;
    assert!(matches!(
        inconsistent.persist_dropped(&cas),
        Err(DropManifestError::DroppedCountMismatch { .. })
    ));
    Ok(())
}

#[test]
fn oversized_dropped_chunk_is_rejected() -> TestResult {
    let oversized = "x".repeat(MAX_DROPPED_CHUNK_BYTES);
    let result = distilled(vec![chunk(&oversized)?]).persist_dropped(&MemoryStore::default());

    assert!(matches!(
        result,
        Err(DropManifestError::ChunkTooLarge { .. })
    ));
    Ok(())
}

#[test]
fn recovery_rejects_content_hash_mismatch() -> TestResult {
    let store = MemoryStore::default();
    let persisted = distilled(vec![chunk("original")?]).persist_dropped(&store)?;
    let chunk_ref = persisted
        .dropped_refs
        .first()
        .copied()
        .ok_or("persisted manifest had no chunk ref")?;
    store.corrupt(chunk_ref)?;

    assert!(matches!(
        recover_dropped(&store, &persisted.manifest_ref),
        Err(DropManifestError::ContentHashMismatch { .. })
    ));
    Ok(())
}

#[test]
fn recovery_rejects_unknown_kind_and_version() -> TestResult {
    // Validly hashed but semantically unsupported records must fail closed.
    let store = MemoryStore::default();
    let wrong_kind = store.put(br#"{"kind":"other","version":1,"source":"fixture-source","locator":{"type":"url","url":"https://example.test/"},"text":"x","trust":"untrusted"}"#)?;
    let manifest = store.put(&serde_json::to_vec(&serde_json::json!({
        "kind": "polaris.search.dropped_manifest",
        "version": 1,
        "entries": [wrong_kind.to_hex()]
    }))?)?;
    assert!(matches!(
        recover_dropped(&store, &manifest),
        Err(DropManifestError::InvalidRecordKind)
    ));

    let store = MemoryStore::default();
    let wrong_version = store.put(br#"{"kind":"polaris.search.dropped_chunk","version":999,"source":"fixture-source","locator":{"type":"url","url":"https://example.test/"},"text":"x","trust":"untrusted"}"#)?;
    let manifest = store.put(&serde_json::to_vec(&serde_json::json!({
        "kind": "polaris.search.dropped_manifest",
        "version": 1,
        "entries": [wrong_version.to_hex()]
    }))?)?;
    assert!(matches!(
        recover_dropped(&store, &manifest),
        Err(DropManifestError::UnsupportedVersion(999))
    ));
    Ok(())
}

#[test]
fn recovery_rejects_unknown_wire_fields() -> TestResult {
    let store = MemoryStore::default();
    let manifest = store.put(&serde_json::to_vec(&serde_json::json!({
        "kind": "polaris.search.dropped_manifest",
        "version": 1,
        "entries": [],
        "unexpected": "must not be ignored"
    }))?)?;

    assert!(matches!(
        recover_dropped(&store, &manifest),
        Err(DropManifestError::InvalidEncoding)
    ));
    Ok(())
}

#[test]
fn recovery_enforces_manifest_bounds() -> TestResult {
    let store = MemoryStore::default();
    let too_many = vec![serde_json::json!("00".repeat(32)); 4097];
    let bytes = serde_json::to_vec(&serde_json::json!({
        "kind": "polaris.search.dropped_manifest",
        "version": 1,
        "entries": too_many
    }))?;
    let manifest = store.put(&bytes)?;

    assert!(matches!(
        recover_dropped(&store, &manifest),
        Err(DropManifestError::TooManyEntries { .. })
    ));

    let oversized = vec![b'x'; MAX_DROPPED_MANIFEST_BYTES + 1];
    let oversized_ref = store.put(&oversized)?;
    assert!(matches!(
        recover_dropped(&store, &oversized_ref),
        Err(DropManifestError::ContentStore(
            ContentStoreError::TooLarge { .. }
        ))
    ));
    Ok(())
}

#[test]
fn recovery_limits_aggregate_content_bytes() -> TestResult {
    let store = MemoryStore::default();
    let text = "z".repeat(MAX_DROPPED_RECOVERY_BYTES / 128);
    let persisted = distilled(vec![chunk(&text)?]).persist_dropped(&store)?;
    let chunk_ref = persisted
        .dropped_refs
        .first()
        .copied()
        .ok_or("persisted manifest had no chunk ref")?;
    let entries = vec![serde_json::json!(chunk_ref.to_hex()); MAX_DROPPED_CHUNKS];
    let bytes = serde_json::to_vec(&serde_json::json!({
        "kind": "polaris.search.dropped_manifest",
        "version": 1,
        "entries": entries
    }))?;
    let manifest = store.put(&bytes)?;

    assert!(matches!(
        recover_dropped(&store, &manifest),
        Err(DropManifestError::RecoveryTooLarge { .. })
    ));
    Ok(())
}

#[derive(Default)]
struct MemoryStore {
    blobs: Mutex<HashMap<Cid, Vec<u8>>>,
}

#[test]
fn persistence_never_commits_an_unrecoverable_aggregate() -> TestResult {
    let store = MemoryStore::default();
    let text = "z".repeat(MAX_DROPPED_RECOVERY_BYTES / 128);
    let dropped = vec![chunk(&text)?; 129];
    assert!(matches!(
        distilled(dropped).persist_dropped(&store),
        Err(DropManifestError::RecoveryTooLarge { .. })
    ));
    Ok(())
}

impl MemoryStore {
    fn corrupt(&self, cid: Cid) -> TestResult {
        let mut blobs = self.blobs.lock().map_err(|_| "test store lock poisoned")?;
        let blob = blobs.get_mut(&cid).ok_or("test blob missing")?;
        blob.push(b'!');
        Ok(())
    }
}

impl ContentStore for MemoryStore {
    fn put(&self, content: &[u8]) -> Result<Cid, ContentStoreError> {
        let cid = Cid::from_bytes(*blake3::hash(content).as_bytes());
        let mut blobs = self
            .blobs
            .lock()
            .map_err(|_| ContentStoreError::Unavailable)?;
        blobs.insert(cid, content.to_vec());
        Ok(cid)
    }

    fn contains(&self, cid: &Cid) -> Result<bool, ContentStoreError> {
        let blobs = self
            .blobs
            .lock()
            .map_err(|_| ContentStoreError::Unavailable)?;
        Ok(blobs.contains_key(cid))
    }

    fn get(&self, cid: &Cid, max_bytes: u64) -> Result<bytes::Bytes, ContentStoreError> {
        let blobs = self
            .blobs
            .lock()
            .map_err(|_| ContentStoreError::Unavailable)?;
        let blob = blobs.get(cid).ok_or(ContentStoreError::NotFound(*cid))?;
        let size = u64::try_from(blob.len()).map_err(|_| ContentStoreError::TooLarge {
            limit: max_bytes,
            size: u64::MAX,
        })?;
        if size > max_bytes {
            return Err(ContentStoreError::TooLarge {
                limit: max_bytes,
                size,
            });
        }
        Ok(bytes::Bytes::copy_from_slice(blob))
    }
}
