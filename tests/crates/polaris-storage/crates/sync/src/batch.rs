// Unit tests of crates/polaris-storage/crates/sync/src/batch.rs (`tests`); compiled into that module via #[path].
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

#[test]
fn rejects_noncanonical_remote_roots() {
    for root in [
        "",
        "/",
        "//sandbox/workspace",
        "/sandbox/",
        "/sandbox//workspace",
        "/sandbox/../workspace",
    ] {
        let batch = SandboxSyncBatch {
            version: 1,
            root: root.into(),
            mutations: vec![file_mutation("safe.txt", "payload/000000", b"x")],
        };
        let payloads = BTreeMap::from([("payload/000000".into(), Bytes::from_static(b"x"))]);
        assert!(encode_batch_archive(&batch, &payloads).is_err(), "{root:?}");
    }
}
