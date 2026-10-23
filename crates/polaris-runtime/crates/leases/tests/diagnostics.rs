//! Total ownership diagnostics must include every key, without sampling.
use polaris_runtime_leases::{LeaseConfig, LeaseKey, LeaseManager, LeaseRequest, LeaseScope};
use polaris_sdk::ids::FrameId;

#[test]
fn total_count_tracks_every_guard_and_release() -> Result<(), Box<dyn std::error::Error>> {
    let manager = LeaseManager::new(LeaseConfig::default());
    assert_eq!(manager.active_count(), 0, "initial state");
    let mut guards = Vec::new();
    for index in 0..40 {
        guards.push(manager.try_acquire(LeaseRequest {
            agent: FrameId::new_v7(),
            key: LeaseKey::from_resolved_relative(&format!("file-{index}.rs"))?,
            scope: LeaseScope::EntireFile,
            base: None,
        })?);
        assert_eq!(manager.active_count(), guards.len(), "all keys counted");
    }
    drop(guards);
    assert_eq!(manager.active_count(), 0, "every guard released");
    Ok(())
}
