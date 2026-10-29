// Unit tests of crates/polaris-system-one/src/policy.rs (`recovery_tests`); compiled into that module via #[path].
use super::{OPEN, PolicyError, recover_admission};
use std::sync::atomic::{AtomicU64, Ordering};

#[test]
fn model_readiness_recovers_once_and_only_after_circuit_open() -> Result<(), PolicyError> {
    let state = AtomicU64::new((2_u64 << 32) | OPEN | 3);
    assert!(!recover_admission(&state, false)?);
    assert_eq!(state.load(Ordering::Acquire), (2_u64 << 32) | OPEN | 3);
    assert!(recover_admission(&state, true)?);
    assert_eq!(state.load(Ordering::Acquire), 3_u64 << 32);
    assert!(!recover_admission(&state, true)?);
    assert_eq!(state.load(Ordering::Acquire), 3_u64 << 32);
    Ok(())
}

#[test]
fn exhausted_recovery_epoch_stays_closed() {
    let state = AtomicU64::new((u64::from(u32::MAX) << 32) | OPEN | 1);
    assert_eq!(
        recover_admission(&state, true),
        Err(PolicyError::EpochExhausted)
    );
    assert_ne!(state.load(Ordering::Acquire) & OPEN, 0);
}
