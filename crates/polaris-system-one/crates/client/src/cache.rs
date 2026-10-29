use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use polaris_proto::pb;
use polaris_sdk::system_one::Decision;
use prost::Message;

const CACHE_CAPACITY: usize = 256;
const MAX_CACHE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ModelIdentity {
    digest: [u8; 32],
    generation: u64,
    incarnation: [u8; 16],
}

impl ModelIdentity {
    pub(crate) fn has_digest(self, expected: [u8; 32]) -> bool {
        self.digest == expected
    }
    pub(crate) fn from_health(response: &pb::HealthResponse) -> Result<Option<Self>, ()> {
        if response.loaded_model_identity_blake3.is_empty() && response.loaded_model_generation == 0
        {
            return Ok(None);
        }
        let digest = <[u8; 32]>::try_from(response.loaded_model_identity_blake3.as_slice())
            .map_err(|_| ())?;
        let incarnation =
            <[u8; 16]>::try_from(response.worker_incarnation.as_slice()).map_err(|_| ())?;
        if response.loaded_model_generation == 0 {
            return Err(());
        }
        Ok(Some(Self {
            digest,
            generation: response.loaded_model_generation,
            incarnation,
        }))
    }
}

struct CacheEntry {
    request: Vec<u8>,
    decision: Decision,
    weight: usize,
}

/// Bounded exact-request cache scoped to a freshly authenticated worker model identity.
pub(crate) struct DecisionCache {
    state: Mutex<CacheState>,
}

struct CacheState {
    entries: HashMap<[u8; 32], CacheEntry>,
    insertion_order: VecDeque<[u8; 32]>,
    total_bytes: usize,
    identity: Option<ModelIdentity>,
}

impl DecisionCache {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(CacheState {
                entries: HashMap::with_capacity(CACHE_CAPACITY),
                insertion_order: VecDeque::with_capacity(CACHE_CAPACITY),
                total_bytes: 0,
                identity: None,
            }),
        }
    }

    pub(crate) fn get(
        &self,
        identity: ModelIdentity,
        request: &pb::DecisionRequest,
    ) -> Option<Decision> {
        let bytes = request.encode_to_vec();
        let key = cache_key(identity, &bytes);
        let mut state = self.state.lock().ok()?;
        activate_identity(&mut state, identity);
        lookup_entry(&state, key, &bytes)
    }

    pub(crate) fn insert(
        &self,
        identity: ModelIdentity,
        request: &pb::DecisionRequest,
        decision: Decision,
    ) {
        let bytes = request.encode_to_vec();
        let decision_bytes = pb::Decision::from(&decision).encoded_len();
        let weight = match bytes.len().checked_add(decision_bytes) {
            Some(value) if value <= MAX_CACHE_BYTES => value,
            _ => return,
        };
        let key = cache_key(identity, &bytes);
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        activate_identity(&mut state, identity);
        if let Some(previous) = state.entries.remove(&key) {
            state.total_bytes = match state.total_bytes.checked_sub(previous.weight) {
                Some(value) => value,
                None => {
                    clear_state(&mut state);
                    0
                }
            };
            state.insertion_order.retain(|existing| existing != &key);
        }
        while state.entries.len() >= CACHE_CAPACITY
            || state
                .total_bytes
                .checked_add(weight)
                .is_none_or(|size| size > MAX_CACHE_BYTES)
        {
            let Some(oldest) = state.insertion_order.pop_front() else {
                break;
            };
            if let Some(previous) = state.entries.remove(&oldest) {
                state.total_bytes = match state.total_bytes.checked_sub(previous.weight) {
                    Some(value) => value,
                    None => {
                        clear_state(&mut state);
                        0
                    }
                };
            }
        }
        if state
            .total_bytes
            .checked_add(weight)
            .is_none_or(|size| size > MAX_CACHE_BYTES)
        {
            return;
        }
        let Some(total_bytes) = state.total_bytes.checked_add(weight) else {
            return;
        };
        state.insertion_order.push_back(key);
        state.total_bytes = total_bytes;
        state.entries.insert(
            key,
            CacheEntry {
                request: bytes,
                decision,
                weight,
            },
        );
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            clear_state(&mut state);
            state.identity = None;
        }
    }
}

fn activate_identity(state: &mut CacheState, identity: ModelIdentity) {
    if state.identity != Some(identity) {
        clear_state(state);
        state.identity = Some(identity);
    }
}

fn clear_state(state: &mut CacheState) {
    state.entries.clear();
    state.insertion_order.clear();
    state.total_bytes = 0;
}

fn lookup_entry(state: &CacheState, key: [u8; 32], request: &[u8]) -> Option<Decision> {
    let entry = state.entries.get(&key)?;
    if entry.request == request {
        Some(entry.decision.clone())
    } else {
        None
    }
}

fn cache_key(identity: ModelIdentity, request: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"polaris-s1-decision-cache-v1\0");
    hasher.update(&identity.digest);
    hasher.update(&identity.generation.to_le_bytes());
    hasher.update(&identity.incarnation);
    hasher.update(request);
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-system-one/crates/client/src/cache.rs"]
mod tests;
