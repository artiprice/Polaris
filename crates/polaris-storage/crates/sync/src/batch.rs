//! Bounded, deterministic tar encoding for sync batches (descriptor: `specs/descriptors/sync_batch.json`).
//!
//! This module only constructs an archive in memory. Applying it requires the guest's
//! separately reviewed compare-and-swap command.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use bytes::Bytes;
use polaris_sdk::ids::Cid;
use polaris_sdk::registry::{SandboxBatchAction, SandboxSyncBatch};

use crate::{RelPath, SyncError};

const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 4 * 1024 * 1024;
const MAX_MUTATIONS: usize = 4096;
const MAX_BATCH_FILE_BYTES: u64 = 27 * 1024 * 1024;

/// Encode one bounded batch as a deterministic, uncompressed tar archive.
///
/// Payload names are synthetic (`payload/000000` etc.); relative workspace paths appear only
/// in the typed manifest. The guest must treat the archive as untrusted input.
///
/// # Errors
/// Returns [`SyncError::Limit`] when the archive or its inputs exceed the batch budget, and
/// [`SyncError::Rule`] when paths, payload references, or hashes are inconsistent.
pub fn encode_batch_archive(
    batch: &SandboxSyncBatch,
    payloads: &BTreeMap<String, Bytes>,
) -> Result<Bytes, SyncError> {
    if batch.version != 1
        || !valid_remote_root(&batch.root)
        || batch.mutations.is_empty()
        || batch.mutations.len() > MAX_MUTATIONS
    {
        return Err(SyncError::Rule(
            "invalid sync batch header or mutation count".into(),
        ));
    }
    let mut paths = BTreeSet::new();
    let mut referenced = BTreeSet::new();
    for mutation in &batch.mutations {
        let path = RelPath::new(&mutation.relative_path)?;
        if path.as_str().len() > 4096
            || path.components().any(|part| {
                part.eq_ignore_ascii_case(".git") || part.eq_ignore_ascii_case(".polaris")
            })
            || !paths.insert(mutation.relative_path.as_str())
        {
            return Err(SyncError::Rule("invalid or duplicate batch path".into()));
        }
        match &mutation.action {
            SandboxBatchAction::Delete => {}
            SandboxBatchAction::File { payload, hash, .. } => {
                if !valid_payload_name(payload) || !referenced.insert(payload.as_str()) {
                    return Err(SyncError::Rule(
                        "invalid or reused batch payload name".into(),
                    ));
                }
                let bytes = payloads
                    .get(payload)
                    .ok_or_else(|| SyncError::Rule("batch payload is missing".into()))?;
                if bytes.len() as u64 > MAX_BATCH_FILE_BYTES
                    || Cid::from_bytes(*blake3::hash(bytes).as_bytes()) != *hash
                {
                    return Err(SyncError::Rule(
                        "batch payload size or digest mismatch".into(),
                    ));
                }
            }
            SandboxBatchAction::Link { target, hash } => {
                if !safe_link_target(&path, target)
                    || Cid::from_bytes(*blake3::hash(target.as_bytes()).as_bytes()) != *hash
                {
                    return Err(SyncError::Rule(
                        "invalid batch link target or digest".into(),
                    ));
                }
            }
        }
    }
    if referenced.len() != payloads.len()
        || payloads
            .keys()
            .any(|name| !referenced.contains(name.as_str()))
    {
        return Err(SyncError::Rule("batch contains an unused payload".into()));
    }

    let manifest = serde_json::to_vec(batch)
        .map_err(|error| SyncError::Rule(format!("cannot encode batch manifest: {error}")))?;
    if manifest.len() > MAX_MANIFEST_BYTES {
        return Err(SyncError::Limit {
            what: "sync batch manifest bytes",
            limit: MAX_MANIFEST_BYTES as u64,
        });
    }
    let payload_bytes = payloads.values().try_fold(0_u64, |total, bytes| {
        total
            .checked_add(bytes.len() as u64)
            .ok_or(SyncError::Limit {
                what: "sync batch bytes",
                limit: MAX_ARCHIVE_BYTES,
            })
    })?;
    let tar_overhead = u64::try_from(payloads.len().saturating_add(2))
        .unwrap_or(u64::MAX)
        .saturating_mul(1024);
    let estimated = payload_bytes
        .saturating_add(manifest.len() as u64)
        .saturating_add(tar_overhead);
    if estimated > MAX_ARCHIVE_BYTES {
        return Err(SyncError::Limit {
            what: "sync batch archive bytes",
            limit: MAX_ARCHIVE_BYTES,
        });
    }

    let mut builder = tar::Builder::new(Vec::new());
    append_member(&mut builder, "manifest.json", &manifest)?;
    for (name, bytes) in payloads {
        if !valid_payload_name(name) {
            return Err(SyncError::Rule("invalid batch payload name".into()));
        }
        append_member(&mut builder, name, bytes)?;
    }
    let archive = builder
        .into_inner()
        .map_err(|error| SyncError::Rule(format!("cannot finish sync batch archive: {error}")))?;
    if archive.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err(SyncError::Limit {
            what: "sync batch archive bytes",
            limit: MAX_ARCHIVE_BYTES,
        });
    }
    Ok(Bytes::from(archive))
}

fn append_member<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    name: &str,
    bytes: &[u8],
) -> Result<(), SyncError> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o600);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    builder
        .append_data(&mut header, name, Cursor::new(bytes))
        .map_err(|error| SyncError::Rule(format!("cannot append sync batch member: {error}")))
}

fn valid_remote_root(root: &str) -> bool {
    root.starts_with('/')
        && !root.starts_with("//")
        && !root.contains(['\\', '\0'])
        && root
            .split('/')
            .skip(1)
            .all(|part| part != "." && part != "..")
}

fn valid_payload_name(name: &str) -> bool {
    let Some(index) = name.strip_prefix("payload/") else {
        return false;
    };
    index.len() == 6 && index.bytes().all(|byte| byte.is_ascii_digit())
}

fn safe_link_target(path: &RelPath, target: &str) -> bool {
    if target.is_empty() || target.starts_with('/') || target.contains(['\\', '\0']) {
        return false;
    }
    let mut depth = path
        .parent()
        .map_or(0usize, |parent| parent.split('/').count());
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." => match depth.checked_sub(1) {
                Some(value) => depth = value,
                None => return false,
            },
            _ => {
                depth = match depth.checked_add(1) {
                    Some(value) => value,
                    None => return false,
                }
            }
        }
    }
    depth > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use polaris_sdk::registry::{SandboxBatchAction, SandboxBatchMutation};
    use std::io::Read;

    fn file_mutation(path: &str, payload: &str, bytes: &[u8]) -> SandboxBatchMutation {
        SandboxBatchMutation {
            relative_path: path.into(),
            expected: None,
            action: SandboxBatchAction::File {
                payload: payload.into(),
                hash: Cid::from_bytes(*blake3::hash(bytes).as_bytes()),
                executable: false,
            },
        }
    }

    #[test]
    fn archive_roundtrips_manifest_and_exact_payload_bytes() {
        let bytes = b"first\r\nsecond\r\n\0";
        let batch = SandboxSyncBatch {
            version: 1,
            root: "/sandbox/workspace".into(),
            mutations: vec![file_mutation("nested/file.bin", "payload/000000", bytes)],
        };
        let payloads = BTreeMap::from([("payload/000000".into(), Bytes::from_static(bytes))]);
        let archive = encode_batch_archive(&batch, &payloads).expect("bounded archive");
        let mut tar = tar::Archive::new(Cursor::new(archive));
        let mut entries = tar.entries().expect("entries");
        let mut manifest = Vec::new();
        let mut payload = Vec::new();
        entries
            .next()
            .expect("manifest entry")
            .expect("manifest")
            .read_to_end(&mut manifest)
            .expect("manifest bytes");
        entries
            .next()
            .expect("payload entry")
            .expect("payload")
            .read_to_end(&mut payload)
            .expect("payload bytes");
        assert_eq!(
            serde_json::from_slice::<SandboxSyncBatch>(&manifest).expect("typed manifest"),
            batch
        );
        assert_eq!(payload, bytes);
        assert!(entries.next().is_none());
    }

    #[test]
    fn rejects_wrong_hash_deny_paths_and_unbounded_inputs() {
        let batch = SandboxSyncBatch {
            version: 1,
            root: "/sandbox/workspace".into(),
            mutations: vec![file_mutation("safe.txt", "payload/000000", b"expected")],
        };
        let payloads = BTreeMap::from([("payload/000000".into(), Bytes::from_static(b"changed"))]);
        assert!(encode_batch_archive(&batch, &payloads).is_err());
        let mut denied = batch.clone();
        denied.mutations[0].relative_path = ".polaris/secret".into();
        let payloads = BTreeMap::from([("payload/000000".into(), Bytes::from_static(b"expected"))]);
        assert!(encode_batch_archive(&denied, &payloads).is_err());
        let mut huge = batch;
        huge.mutations = (0..MAX_MUTATIONS + 1)
            .map(|index| file_mutation(&format!("f{index}"), &format!("payload/{index:06}"), b"x"))
            .collect();
        let payloads = (0..MAX_MUTATIONS + 1)
            .map(|index| (format!("payload/{index:06}"), Bytes::from_static(b"x")))
            .collect();
        assert!(encode_batch_archive(&huge, &payloads).is_err());
    }
}
