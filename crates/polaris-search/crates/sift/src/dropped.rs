//! Bounded CAS persistence for the original chunks discarded during context sifting.

use crate::{Distilled, Locator, RawChunk, SourceId, Trust};
use polaris_sdk::{
    content::{ContentStore, ContentStoreError},
    ids::Cid,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::HashSet;
use thiserror::Error;

const MANIFEST_KIND: &str = "polaris.search.dropped_manifest";
const CHUNK_KIND: &str = "polaris.search.dropped_chunk";
const RECORD_VERSION: u16 = 1;
/// Maximum encoded bytes accepted for one persisted dropped chunk.
pub const MAX_DROPPED_CHUNK_BYTES: usize = 1 << 20;
/// Maximum encoded bytes accepted for a manifest.
pub const MAX_DROPPED_MANIFEST_BYTES: usize = 1 << 20;
/// Maximum chunk occurrences recorded by one manifest.
pub const MAX_DROPPED_CHUNKS: usize = 4096;
/// Maximum aggregate encoded chunk bytes recovered from one manifest.
pub const MAX_DROPPED_RECOVERY_BYTES: usize = 16 << 20;

/// Durable references returned after a dropped-chunk manifest has been committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedManifest {
    /// CID of the manifest blob used by [`recover_dropped`].
    pub manifest_ref: Cid,
    /// Unique CIDs for the original chunk records, in first-seen order.
    pub dropped_refs: Vec<Cid>,
    /// Number of chunk occurrences represented by the manifest.
    pub dropped_count: usize,
}

/// Errors from bounded serialization, persistence, and recovery of dropped chunks.
#[derive(Debug, Error)]
pub enum DropManifestError {
    /// The CAS backend could not store or read a blob.
    #[error("dropped-chunk content store failed: {0}")]
    ContentStore(#[from] ContentStoreError),
    /// A source chunk is larger than the supported encoded-record bound.
    #[error("dropped chunk exceeds the encoded byte limit ({size} > {limit})")]
    ChunkTooLarge { limit: usize, size: usize },
    /// A manifest is larger than the supported encoded-record bound.
    #[error("dropped manifest exceeds the encoded byte limit ({size} > {limit})")]
    ManifestTooLarge { limit: usize, size: usize },
    /// The sum of chunk records requested by one manifest exceeds the recovery budget.
    #[error("dropped recovery exceeds the aggregate byte limit ({size} > {limit})")]
    RecoveryTooLarge { limit: usize, size: usize },
    /// The manifest or its occurrence list exceeds the supported count.
    #[error("dropped manifest has too many chunk occurrences ({count} > {limit})")]
    TooManyEntries { limit: usize, count: usize },
    /// The persisted drop counter disagrees with the actual dropped vector.
    #[error("drop stats count ({stats}) does not match dropped chunk count ({actual})")]
    DroppedCountMismatch { stats: u32, actual: usize },
    /// The blob does not contain a valid manifest or chunk record.
    #[error("dropped-chunk record encoding is invalid")]
    InvalidEncoding,
    /// The blob's record kind is not one of the supported dropped records.
    #[error("dropped-chunk record kind is invalid")]
    InvalidRecordKind,
    /// The blob's record version is unsupported.
    #[error("dropped-chunk record version {0} is unsupported")]
    UnsupportedVersion(u16),
    /// The fetched bytes do not hash to the requested CID.
    #[error("dropped-chunk content hash mismatch for {expected}")]
    ContentHashMismatch { expected: Cid },
    /// Persisted provenance contains an invalid source identifier.
    #[error("dropped-chunk source identifier is invalid")]
    InvalidSourceId,
    /// A chunk cannot be serialized within the bounded encoding budget.
    #[error("dropped-chunk record could not be encoded")]
    Encoding,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestRecord {
    kind: String,
    version: u16,
    entries: Vec<Cid>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkRecord {
    kind: String,
    version: u16,
    source: String,
    locator: WireLocator,
    text: String,
    trust: WireTrust,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum WireLocator {
    Url {
        url: String,
    },
    Document {
        id: String,
        page: Option<u32>,
        line_start: Option<u32>,
        line_end: Option<u32>,
    },
    Symbol {
        path: String,
        name: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireTrust {
    Untrusted,
    CallerValidated,
}

impl Distilled {
    /// Persist original dropped chunks and a manifest through the SDK's generic CAS contract.
    ///
    /// Identical chunk records share a content reference in `dropped_refs`; the manifest keeps
    /// each occurrence in input order so recovery restores the exact dropped vector.
    ///
    /// # Errors
    /// Returns an error for inconsistent drop statistics, configured size/count bounds, invalid
    /// provenance, a serialization failure, or a content-store failure.
    pub fn persist_dropped<S: ContentStore + ?Sized>(
        &self,
        store: &S,
    ) -> Result<DroppedManifest, DropManifestError> {
        if self.dropped.len() > MAX_DROPPED_CHUNKS {
            return Err(DropManifestError::TooManyEntries {
                limit: MAX_DROPPED_CHUNKS,
                count: self.dropped.len(),
            });
        }
        if usize::try_from(self.stats.dropped).map_err(|_| {
            DropManifestError::DroppedCountMismatch {
                stats: self.stats.dropped,
                actual: self.dropped.len(),
            }
        })? != self.dropped.len()
        {
            return Err(DropManifestError::DroppedCountMismatch {
                stats: self.stats.dropped,
                actual: self.dropped.len(),
            });
        }

        let mut entries = Vec::with_capacity(self.dropped.len());
        let mut unique_refs = Vec::with_capacity(self.dropped.len());
        let mut seen = HashSet::with_capacity(self.dropped.len());
        let mut recovered_bytes = 0_usize;
        for chunk in &self.dropped {
            let record = encode_chunk(chunk)?;
            recovered_bytes = recovered_bytes.saturating_add(record.len());
            if recovered_bytes > MAX_DROPPED_RECOVERY_BYTES {
                return Err(DropManifestError::RecoveryTooLarge {
                    limit: MAX_DROPPED_RECOVERY_BYTES,
                    size: recovered_bytes,
                });
            }
            let cid = put_verified(store, &record)?;
            entries.push(cid);
            if seen.insert(cid) {
                unique_refs.push(cid);
            }
        }

        let manifest = ManifestRecord {
            kind: MANIFEST_KIND.into(),
            version: RECORD_VERSION,
            entries,
        };
        let bytes = serde_json::to_vec(&manifest).map_err(|_| DropManifestError::Encoding)?;
        check_size(bytes.len(), MAX_DROPPED_MANIFEST_BYTES, false)?;
        let manifest_ref = put_verified(store, &bytes)?;

        Ok(DroppedManifest {
            manifest_ref,
            dropped_refs: unique_refs,
            dropped_count: self.dropped.len(),
        })
    }
}

/// Recover every original dropped chunk named by a durable manifest CID.
///
/// Both manifest and chunk bytes are bounded and independently re-hashed before decoding. Record
/// kind and version are checked before source metadata is exposed to the caller. The restored
/// trust value is the original adapter annotation; recovery does not revalidate that source.
///
/// # Errors
/// Returns an error for absent/corrupt CAS blobs, hash mismatches, malformed records, unsupported
/// kinds or versions, invalid source identifiers, or configured size/count bounds.
pub fn recover_dropped<S: ContentStore + ?Sized>(
    store: &S,
    manifest_ref: &Cid,
) -> Result<Vec<RawChunk>, DropManifestError> {
    let manifest_bytes = get_bounded(store, manifest_ref, MAX_DROPPED_MANIFEST_BYTES, false, None)?;
    let manifest: ManifestRecord = decode_record(&manifest_bytes)?;
    validate_record(&manifest.kind, manifest.version, MANIFEST_KIND)?;
    if manifest.entries.len() > MAX_DROPPED_CHUNKS {
        return Err(DropManifestError::TooManyEntries {
            limit: MAX_DROPPED_CHUNKS,
            count: manifest.entries.len(),
        });
    }

    let mut chunks = Vec::with_capacity(manifest.entries.len());
    let mut recovered_bytes = 0_usize;
    for cid in manifest.entries {
        let remaining = MAX_DROPPED_RECOVERY_BYTES.saturating_sub(recovered_bytes);
        let bytes = get_bounded(
            store,
            &cid,
            MAX_DROPPED_CHUNK_BYTES,
            true,
            Some((MAX_DROPPED_RECOVERY_BYTES, remaining)),
        )?;
        recovered_bytes = recovered_bytes.saturating_add(bytes.len());
        let record: ChunkRecord = decode_record(&bytes)?;
        validate_record(&record.kind, record.version, CHUNK_KIND)?;
        chunks.push(decode_chunk(record)?);
    }
    Ok(chunks)
}

fn encode_chunk(chunk: &RawChunk) -> Result<Vec<u8>, DropManifestError> {
    check_size(raw_chunk_size_hint(chunk), MAX_DROPPED_CHUNK_BYTES, true)?;
    let locator = match &chunk.locator {
        Locator::Url(url) => WireLocator::Url { url: url.clone() },
        Locator::Document {
            id,
            page,
            line_start,
            line_end,
        } => WireLocator::Document {
            id: id.clone(),
            page: *page,
            line_start: *line_start,
            line_end: *line_end,
        },
        Locator::Symbol { path, name } => WireLocator::Symbol {
            path: path.clone(),
            name: name.clone(),
        },
    };
    let record = ChunkRecord {
        kind: CHUNK_KIND.into(),
        version: RECORD_VERSION,
        source: chunk.source.as_str().into(),
        locator,
        text: chunk.text.clone(),
        trust: match chunk.trust {
            Trust::Untrusted => WireTrust::Untrusted,
            Trust::CallerValidated => WireTrust::CallerValidated,
        },
    };
    let bytes = serde_json::to_vec(&record).map_err(|_| DropManifestError::Encoding)?;
    check_size(bytes.len(), MAX_DROPPED_CHUNK_BYTES, true)?;
    Ok(bytes)
}

fn decode_chunk(record: ChunkRecord) -> Result<RawChunk, DropManifestError> {
    let source = SourceId::new(record.source).map_err(|_| DropManifestError::InvalidSourceId)?;
    let locator = match record.locator {
        WireLocator::Url { url } => Locator::Url(url),
        WireLocator::Document {
            id,
            page,
            line_start,
            line_end,
        } => Locator::Document {
            id,
            page,
            line_start,
            line_end,
        },
        WireLocator::Symbol { path, name } => Locator::Symbol { path, name },
    };
    let trust = match record.trust {
        WireTrust::Untrusted => Trust::Untrusted,
        WireTrust::CallerValidated => Trust::CallerValidated,
    };
    Ok(RawChunk {
        source,
        locator,
        text: record.text,
        trust,
    })
}

fn raw_chunk_size_hint(chunk: &RawChunk) -> usize {
    // JSON escaping can expand one UTF-8 byte to at most six ASCII bytes. This coarse estimate
    // rejects excessively large caller strings before cloning them into the private wire record.
    let locator_bytes = match &chunk.locator {
        Locator::Url(url) => url.len(),
        Locator::Document { id, .. } => id.len(),
        Locator::Symbol { path, name } => path.len().saturating_add(name.len()),
    };
    chunk
        .source
        .as_str()
        .len()
        .saturating_add(locator_bytes)
        .saturating_add(chunk.text.len())
        .saturating_mul(6)
        .saturating_add(256)
}

fn check_size(size: usize, limit: usize, chunk: bool) -> Result<(), DropManifestError> {
    if size <= limit {
        return Ok(());
    }
    if chunk {
        Err(DropManifestError::ChunkTooLarge { limit, size })
    } else {
        Err(DropManifestError::ManifestTooLarge { limit, size })
    }
}

fn put_verified<S: ContentStore + ?Sized>(
    store: &S,
    bytes: &[u8],
) -> Result<Cid, DropManifestError> {
    let cid = store.put(bytes)?;
    let actual = content_id(bytes);
    if actual != cid {
        return Err(DropManifestError::ContentHashMismatch { expected: cid });
    }
    Ok(cid)
}

fn get_bounded<S: ContentStore + ?Sized>(
    store: &S,
    cid: &Cid,
    max_bytes: usize,
    chunk: bool,
    aggregate_limit: Option<(usize, usize)>,
) -> Result<Vec<u8>, DropManifestError> {
    let max_bytes_u64 = u64::try_from(max_bytes).map_err(|_| DropManifestError::InvalidEncoding)?;
    let bytes = store.get(cid, max_bytes_u64)?;
    if bytes.len() > max_bytes {
        return Err(if chunk {
            DropManifestError::ChunkTooLarge {
                limit: max_bytes,
                size: bytes.len(),
            }
        } else {
            DropManifestError::ManifestTooLarge {
                limit: max_bytes,
                size: bytes.len(),
            }
        });
    }
    if let Some((limit, remaining)) = aggregate_limit
        && bytes.len() > remaining
    {
        return Err(DropManifestError::RecoveryTooLarge {
            limit,
            size: limit.saturating_sub(remaining).saturating_add(bytes.len()),
        });
    }
    let actual = content_id(&bytes);
    if actual != *cid {
        return Err(DropManifestError::ContentHashMismatch { expected: *cid });
    }
    Ok(bytes.to_vec())
}

fn decode_record<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, DropManifestError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    // serde_json's default recursion cap remains enabled; these blobs are byte-bounded above.
    let value =
        T::deserialize(&mut deserializer).map_err(|_| DropManifestError::InvalidEncoding)?;
    deserializer
        .end()
        .map_err(|_| DropManifestError::InvalidEncoding)?;
    Ok(value)
}

fn validate_record(kind: &str, version: u16, expected_kind: &str) -> Result<(), DropManifestError> {
    if kind != expected_kind {
        return Err(DropManifestError::InvalidRecordKind);
    }
    if version != RECORD_VERSION {
        return Err(DropManifestError::UnsupportedVersion(version));
    }
    Ok(())
}

fn content_id(bytes: &[u8]) -> Cid {
    Cid::from_bytes(*blake3::hash(bytes).as_bytes())
}
