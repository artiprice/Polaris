// Unit tests of crates/polaris-system-one/crates/client/src/singleflight.rs (`tests`); compiled into that module via #[path].
use std::time::Duration;
use tokio::time::Instant;

use polaris_sdk::registry::DecisionError;

use super::{MAX_FLIGHTS, MAX_WAITERS, Role, SingleFlight};
use crate::cache::ModelIdentity;

fn identity(incarnation: u8) -> Result<ModelIdentity, DecisionError> {
    ModelIdentity::from_health(&polaris_proto::pb::HealthResponse {
        protocol_version: Some(polaris_proto::PROTOCOL_VERSION.into()),
        build_version: "test".to_owned(),
        ready: true,
        loaded_model_identity_blake3: vec![1; 32],
        loaded_model_generation: 7,
        worker_incarnation: vec![incarnation; 16],
        loaded_pii_model_identity_blake3: Vec::new(),
        loaded_pii_model_generation: 0,
    })
    .map_err(|()| DecisionError::Model("test model identity invalid".to_owned()))?
    .ok_or_else(|| DecisionError::Model("test model identity missing".to_owned()))
}

#[tokio::test]
async fn owner_drop_wakes_waiter_to_recompute() -> Result<(), DecisionError> {
    let flights = SingleFlight::default();
    let owner = match flights.join(identity(2)?, b"exact".to_vec()) {
        Role::Owner(owner) => owner,
        Role::Waiter(_) | Role::Bypass => {
            return Err(DecisionError::Model("owner election failed".to_owned()));
        }
    };
    let mut waiter = match flights.join(identity(2)?, b"exact".to_vec()) {
        Role::Waiter(waiter) => waiter,
        Role::Owner(_) | Role::Bypass => {
            return Err(DecisionError::Model("waiter join failed".to_owned()));
        }
    };
    drop(owner);
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(1))
        .ok_or_else(|| DecisionError::Model("test deadline overflow".to_owned()))?;
    assert!(waiter.wait_until(deadline).await.is_none());
    assert!(matches!(
        flights.join(identity(2)?, b"exact".to_vec()),
        Role::Owner(_)
    ));
    Ok(())
}

#[tokio::test]
async fn waiter_deadline_does_not_remove_owner() -> Result<(), DecisionError> {
    let flights = SingleFlight::default();
    let owner = match flights.join(identity(2)?, b"exact".to_vec()) {
        Role::Owner(owner) => owner,
        Role::Waiter(_) | Role::Bypass => {
            return Err(DecisionError::Model("owner election failed".to_owned()));
        }
    };
    let mut waiter = match flights.join(identity(2)?, b"exact".to_vec()) {
        Role::Waiter(waiter) => waiter,
        Role::Owner(_) | Role::Bypass => {
            return Err(DecisionError::Model("waiter join failed".to_owned()));
        }
    };
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(1))
        .ok_or_else(|| DecisionError::Model("test deadline overflow".to_owned()))?;
    assert!(waiter.wait_until(deadline).await.is_none());
    drop(waiter);
    assert!(matches!(
        flights.join(identity(2)?, b"exact".to_vec()),
        Role::Waiter(_)
    ));
    drop(owner);
    Ok(())
}

#[test]
fn identity_and_request_must_both_match() -> Result<(), DecisionError> {
    let flights = SingleFlight::default();
    let _owner = match flights.join(identity(2)?, b"request-a".to_vec()) {
        Role::Owner(owner) => owner,
        Role::Waiter(_) | Role::Bypass => {
            return Err(DecisionError::Model("first election failed".to_owned()));
        }
    };
    assert!(matches!(
        flights.join(identity(3)?, b"request-a".to_vec()),
        Role::Owner(_)
    ));
    assert!(matches!(
        flights.join(identity(2)?, b"request-b".to_vec()),
        Role::Owner(_)
    ));
    Ok(())
}

#[test]
fn flight_entries_are_bounded() -> Result<(), DecisionError> {
    let flights = SingleFlight::default();
    let mut owners = Vec::new();
    for index in 0..MAX_FLIGHTS {
        let bytes = u32::try_from(index)
            .map_err(|_| DecisionError::Model("test index overflow".to_owned()))?
            .to_le_bytes()
            .to_vec();
        owners.push(flights.join(identity(2)?, bytes));
    }
    assert!(matches!(
        flights.join(identity(2)?, b"overflow".to_vec()),
        Role::Bypass
    ));
    Ok(())
}

#[test]
fn waiter_count_is_bounded() -> Result<(), DecisionError> {
    let flights = SingleFlight::default();
    let owner = match flights.join(identity(2)?, b"exact".to_vec()) {
        Role::Owner(owner) => owner,
        Role::Waiter(_) | Role::Bypass => {
            return Err(DecisionError::Model("owner election failed".to_owned()));
        }
    };
    let mut waiters = Vec::new();
    for _ in 0..MAX_WAITERS {
        waiters.push(flights.join(identity(2)?, b"exact".to_vec()));
    }
    assert!(matches!(
        flights.join(identity(2)?, b"exact".to_vec()),
        Role::Bypass
    ));
    drop(waiters);
    drop(owner);
    Ok(())
}
