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
#[path = "../../../../../tests/crates/polaris-system-one/crates/client/src/singleflight.rs"]
mod tests;
