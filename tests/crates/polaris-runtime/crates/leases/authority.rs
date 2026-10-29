// Test doubles and fixtures fail loudly: a panic is this test's failure signal.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
use polaris_runtime_leases::{
    ChurnVerdict, LeaseConfig, LeaseError, LeaseKey, LeaseManager, LeaseRequest, LeaseScope,
    RevokeReason, SymbolPath,
};
use polaris_sdk::ids::{Cid, FrameId};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

fn key(n: usize) -> LeaseKey {
    LeaseKey::from_resolved_relative(&format!("src/file{n}.rs")).expect("relative key")
}
fn request(agent: u128, file: usize) -> LeaseRequest {
    LeaseRequest {
        agent: FrameId::from_u128(agent),
        key: key(file),
        scope: LeaseScope::EntireFile,
        base: Some(Cid::from_bytes([1; 32])),
    }
}

#[test]
fn immediate_acquisition_never_overlaps() {
    let manager = LeaseManager::new(LeaseConfig::default());
    let guard = manager.try_acquire(request(1, 0)).expect("first holder");
    assert!(matches!(
        manager.try_acquire(request(2, 0)),
        Err(LeaseError::Busy { .. })
    ));
    assert!(manager.try_acquire(request(2, 1)).is_ok());
    drop(guard);
    assert!(manager.try_acquire(request(2, 0)).is_ok());
}
#[test]
fn ast_siblings_can_coexist_but_ancestors_cannot() {
    let manager = LeaseManager::new(LeaseConfig::default());
    let mut a = request(1, 0);
    a.scope = LeaseScope::AstNode(SymbolPath::new(["Module", "a"]).expect("symbol"));
    let mut b = request(2, 0);
    b.scope = LeaseScope::AstNode(SymbolPath::new(["Module", "b"]).expect("symbol"));
    let _a = manager.try_acquire(a).expect("a");
    let _b = manager.try_acquire(b).expect("b");
    let mut parent = request(3, 0);
    parent.scope = LeaseScope::AstNode(SymbolPath::new(["Module"]).expect("symbol"));
    assert!(matches!(
        manager.try_acquire(parent),
        Err(LeaseError::Busy { .. })
    ));
    assert!(SymbolPath::new(["bad\0symbol"]).is_err());
}
#[tokio::test]
async fn queued_acquisition_wakes_at_expiry() {
    let manager = LeaseManager::new(LeaseConfig {
        lease_ttl: Duration::from_millis(30),
        acquire_timeout: Duration::from_secs(1),
        ..LeaseConfig::default()
    });
    let _old = manager.try_acquire(request(1, 0)).expect("first holder");
    let new = tokio::time::timeout(Duration::from_millis(500), manager.acquire(request(2, 0)))
        .await
        .expect("expiry wakes waiter")
        .expect("new grant");
    assert_eq!(new.owner(), Some(FrameId::from_u128(2)));
}
#[tokio::test]
async fn cancelled_fifo_head_does_not_block_successor() {
    let manager = LeaseManager::new(LeaseConfig::default());
    let old = manager.try_acquire(request(1, 0)).expect("first holder");
    let m = manager.clone();
    let head =
        tokio::spawn(async move { m.acquire_many(vec![request(2, 0), request(2, 1)]).await });
    tokio::task::yield_now().await;
    let m = manager.clone();
    let successor = tokio::spawn(async move { m.acquire(request(3, 1)).await });
    tokio::task::yield_now().await;
    assert!(!successor.is_finished(), "FIFO head keeps priority");
    head.abort();
    assert!(matches!(head.await, Err(error) if error.is_cancelled()));
    let granted = tokio::time::timeout(Duration::from_secs(1), successor)
        .await
        .expect("unblocked")
        .expect("joined")
        .expect("grant");
    drop(old);
    drop(granted);
    assert!(
        manager
            .holder_of(&key(0), &LeaseScope::EntireFile)
            .is_none()
    );
    assert!(
        manager
            .holder_of(&key(1), &LeaseScope::EntireFile)
            .is_none()
    );
}
#[tokio::test]
async fn occ_tickets_bind_manager_lifetime_base_and_churn() {
    let manager = LeaseManager::new(LeaseConfig::default());
    let other = LeaseManager::new(LeaseConfig::default());
    let mut notices = manager.subscribe_invalidations();
    let guard = manager.try_acquire(request(1, 0)).expect("grant");
    let base = Cid::from_bytes([1; 32]);
    let changed = Cid::from_bytes([2; 32]);
    assert!(matches!(
        manager.check_write(&guard, changed),
        Err(LeaseError::StaleBase { .. })
    ));
    assert_eq!(
        notices.recv().await.expect("invalidation").current,
        Some(changed)
    );
    let ticket = manager.check_write(&guard, base).expect("ticket");
    assert_eq!(
        other.validate_write_ticket(ticket.clone(), base),
        Err(LeaseError::InvalidGuard)
    );
    for _ in 0..2 {
        assert_eq!(
            manager.note_edit(FrameId::from_u128(1), &key(0), "task"),
            Ok(ChurnVerdict::Ok)
        );
    }
    assert_eq!(
        manager.note_edit(FrameId::from_u128(1), &key(0), "task"),
        Ok(ChurnVerdict::Warn { edits: 3 })
    );
    assert!(matches!(
        manager.note_edit(FrameId::from_u128(1), &key(0), "task"),
        Ok(ChurnVerdict::Revoke { .. })
    ));
    assert!(matches!(
        manager.validate_write_ticket(ticket, base),
        Err(LeaseError::Revoked(RevokeReason::ChurnLimit))
    ));
    manager
        .revoke_all(FrameId::from_u128(1), RevokeReason::FrameRevoked)
        .await;
    assert!(
        manager
            .holder_of(&key(0), &LeaseScope::EntireFile)
            .is_none()
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_64_frames_20_files() {
    let manager = LeaseManager::new(LeaseConfig::default());
    let active = Arc::new(Mutex::new([false; 20]));
    let mut tasks = Vec::new();
    for frame in 0..64_u128 {
        let m = manager.clone();
        let a = Arc::clone(&active);
        tasks.push(tokio::spawn(async move {
            for cycle in 0..20_usize {
                let file = (usize::try_from(frame).expect("bounded") + cycle) % 20;
                let grant = m.acquire(request(frame, file)).await.expect("FIFO grant");
                {
                    let mut state = a.lock().expect("mutex");
                    assert!(!state[file], "overlapping writer");
                    state[file] = true;
                }
                tokio::task::yield_now().await;
                {
                    let mut state = a.lock().expect("mutex");
                    assert!(state[file]);
                    state[file] = false;
                }
                drop(grant);
            }
        }));
    }
    for task in tasks {
        task.await.expect("writer task");
    }
    for file in 0..20 {
        assert!(
            manager
                .holder_of(&key(file), &LeaseScope::EntireFile)
                .is_none()
        );
    }
}
