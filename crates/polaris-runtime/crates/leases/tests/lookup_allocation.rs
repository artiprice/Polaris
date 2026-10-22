//! Actual allocation count for the lock-free immutable snapshot read path.
use polaris_runtime_leases::{LeaseConfig, LeaseKey, LeaseManager, LeaseRequest, LeaseScope};
use polaris_sdk::ids::FrameId;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[test]
fn lookup_allocates_nothing_after_warmup() -> Result<(), Box<dyn std::error::Error>> {
    let _profiler = dhat::Profiler::builder().testing().build();
    let manager = LeaseManager::new(LeaseConfig::default());
    let mut guards = Vec::new();
    for index in 0..1024 {
        guards.push(manager.try_acquire(LeaseRequest {
            agent: FrameId::new_v7(),
            key: LeaseKey::from_resolved_relative(&format!("source-{index:04}.rs"))?,
            scope: LeaseScope::EntireFile,
            base: None,
        })?);
    }
    let found = LeaseKey::from_resolved_relative("source-1023.rs")?;
    let missing = LeaseKey::from_resolved_relative("source-1024.rs")?;
    for _ in 0..100 {
        std::hint::black_box(manager.holder_of(&found, &LeaseScope::EntireFile));
    }
    let before = dhat::HeapStats::get();
    for _ in 0..10_000 {
        assert!(manager.holder_of(&found, &LeaseScope::EntireFile).is_some());
        assert!(
            manager
                .holder_of(&missing, &LeaseScope::EntireFile)
                .is_none()
        );
    }
    let after = dhat::HeapStats::get();
    assert_eq!(
        after.total_blocks, before.total_blocks,
        "holder lookup allocated"
    );
    assert_eq!(guards.len(), 1024);
    Ok(())
}
