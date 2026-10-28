use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use polaris_sdk::registry::DecisionError;
use polaris_sdk::system_one::Decision;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::cache::ModelIdentity;

const MAX_FLIGHTS: usize = 32;
const MAX_WAITERS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FlightKey {
    identity: ModelIdentity,
    request: Vec<u8>,
}

struct Flight {
    result: watch::Sender<Option<Result<Decision, DecisionError>>>,
}

#[derive(Default)]
struct State {
    entries: HashMap<FlightKey, Arc<Flight>>,
    waiters: usize,
}

/// Bounded per-client registry for concurrent exact-request decision calls.
#[derive(Default)]
pub(crate) struct SingleFlight {
    state: Mutex<State>,
}

pub(crate) enum Role<'a> {
    Owner(FlightOwner<'a>),
    Waiter(FlightWaiter<'a>),
    Bypass,
}

impl SingleFlight {
    pub(crate) fn join(&self, identity: ModelIdentity, request: Vec<u8>) -> Role<'_> {
        let key = FlightKey { identity, request };
        let Ok(mut state) = self.state.lock() else {
            return Role::Bypass;
        };
        if let Some(flight) = state.entries.get(&key).cloned() {
            if state.waiters >= MAX_WAITERS {
                return Role::Bypass;
            }
            state.waiters = match state.waiters.checked_add(1) {
                Some(count) => count,
                None => return Role::Bypass,
            };
            let receiver = flight.result.subscribe();
            return Role::Waiter(FlightWaiter {
                registry: self,
                receiver,
                active: true,
            });
        }
        if state.entries.len() >= MAX_FLIGHTS {
            return Role::Bypass;
        }
        let (sender, _receiver) = watch::channel(None);
        let flight = Arc::new(Flight { result: sender });
        state.entries.insert(key.clone(), Arc::clone(&flight));
        Role::Owner(FlightOwner {
            registry: self,
            key,
            flight,
            active: true,
        })
    }

    fn remove(&self, key: &FlightKey, flight: &Arc<Flight>) {
        if let Ok(mut state) = self.state.lock()
            && state
                .entries
                .get(key)
                .is_some_and(|existing| Arc::ptr_eq(existing, flight))
        {
            state.entries.remove(key);
        }
    }

    fn release_waiter(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.waiters = state.waiters.saturating_sub(1);
        }
    }
}

pub(crate) struct FlightOwner<'a> {
    registry: &'a SingleFlight,
    key: FlightKey,
    flight: Arc<Flight>,
    active: bool,
}

impl FlightOwner<'_> {
    pub(crate) fn complete(mut self, decision: Decision) {
        self.registry.remove(&self.key, &self.flight);
        self.active = false;
        self.flight.result.send_replace(Some(Ok(decision)));
    }
}

impl Drop for FlightOwner<'_> {
    fn drop(&mut self) {
        if self.active {
            self.registry.remove(&self.key, &self.flight);
            self.flight
                .result
                .send_replace(Some(Err(DecisionError::Unavailable(
                    "S1 single-flight owner ended".to_owned(),
                ))));
        }
    }
}

pub(crate) struct FlightWaiter<'a> {
    registry: &'a SingleFlight,
    receiver: watch::Receiver<Option<Result<Decision, DecisionError>>>,
    active: bool,
}

impl FlightWaiter<'_> {
    pub(crate) async fn wait_until(&mut self, deadline: Instant) -> Option<Decision> {
        loop {
            if let Some(result) = self.receiver.borrow().clone() {
                return result.ok();
            }
            if tokio::time::timeout_at(deadline, self.receiver.changed())
                .await
                .is_err()
            {
                return None;
            }
        }
    }
}

impl Drop for FlightWaiter<'_> {
    fn drop(&mut self) {
        if self.active {
            self.registry.release_waiter();
        }
    }
}

#[cfg(test)]
mod tests {
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
}
