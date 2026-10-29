//! Zero Alloc Hot Path (descriptor: specs/descriptors/cas_zero_alloc_hot_path.json; PLAN ADR-003).
//! One integration-test binary per zero-alloc check: dhat allows one profiler per process.

use polaris_sdk::ids::ShortRef;
use polaris_storage_cas::{FOLD_REF_LEN, content_id, fast_fingerprint, fold_ref};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[test]
fn hot_path_functions_do_not_allocate() {
    let _profiler = dhat::Profiler::builder().testing().build();
    let data = [7u8; 4096];
    let mut out = [0u8; FOLD_REF_LEN];

    let before = dhat::HeapStats::get();
    let cid = content_id(&data);
    let fp = fast_fingerprint(&data);
    let written = fold_ref(&cid, &mut out);
    let short = ShortRef::of(&cid);
    let after = dhat::HeapStats::get();

    assert_eq!(
        after.total_blocks, before.total_blocks,
        "hot path allocated"
    );
    assert_eq!(written, FOLD_REF_LEN);
    assert_ne!(fp.get(), 0);
    assert_eq!(short.as_bytes().first(), cid.as_bytes().first());
}
