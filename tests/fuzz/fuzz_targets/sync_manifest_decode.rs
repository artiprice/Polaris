#![no_main]

use libfuzzer_sys::fuzz_target;
use polaris_sdk::registry::SandboxSyncBatch;
use polaris_storage_sync::RelPath;

const MAX_MANIFEST_BYTES: usize = 1 << 20;

fuzz_target!(|data: &[u8]| {
    let input = data
        .get(..data.len().min(MAX_MANIFEST_BYTES))
        .unwrap_or(data);
    let Ok(batch) = serde_json::from_slice::<SandboxSyncBatch>(input) else {
        return;
    };
    for mutation in &batch.mutations {
        if let Ok(path) = RelPath::new(&mutation.relative_path) {
            assert!(path.as_str() == mutation.relative_path);
        }
    }
    if let Ok(encoded) = serde_json::to_vec(&batch) {
        assert!(encoded.len() <= MAX_MANIFEST_BYTES);
        if let Ok(decoded) = serde_json::from_slice::<SandboxSyncBatch>(&encoded) {
            assert_eq!(decoded, batch);
        }
    }
});
