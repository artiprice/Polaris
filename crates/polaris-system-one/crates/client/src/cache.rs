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
mod tests {
    use std::collections::{HashMap, VecDeque};

    use polaris_proto::pb;
    use polaris_sdk::system_one::{
        Answer, Decision, LabelScore, Probability, QuestionAnswer, QuestionId,
    };

    use super::{DecisionCache, ModelIdentity};

    fn identity(byte: u8, generation: u64) -> ModelIdentity {
        ModelIdentity {
            digest: [byte; 32],
            generation,
            incarnation: [9; 16],
        }
    }

    fn request(text: &str, context: Option<&str>, question: &str) -> pb::DecisionRequest {
        pb::DecisionRequest {
            questions: vec![pb::Question {
                id: "q".to_owned(),
                kind: Some(pb::question::Kind::Choice(pb::question::Choice {
                    options: vec![question.to_owned()],
                })),
            }],
            input: Some(pb::DecisionInput {
                text: text.to_owned(),
                context: context.map(str::to_owned),
            }),
        }
    }

    fn decision() -> Result<Decision, Box<dyn std::error::Error>> {
        Ok(Decision {
            answers: vec![QuestionAnswer {
                question: QuestionId::new("q")?,
                answer: Answer::Choice {
                    selected: polaris_sdk::system_one::Label::new("bug")?,
                    probs: vec![LabelScore {
                        label: polaris_sdk::system_one::Label::new("bug")?,
                        probability: Probability::new(1.0)?,
                    }],
                },
            }],
            latency_us: 9,
        })
    }

    #[test]
    fn cache_key_binds_model_generation_schema_and_masked_context()
    -> Result<(), Box<dyn std::error::Error>> {
        let cache = DecisionCache::new();
        let baseline = request("masked", Some("context-a"), "bug");
        cache.insert(identity(7, 1), &baseline, decision()?);
        assert!(cache.get(identity(7, 1), &baseline).is_some());
        assert!(cache.get(identity(7, 2), &baseline).is_none());
        assert!(cache.get(identity(8, 1), &baseline).is_none());
        cache.insert(identity(7, 1), &baseline, decision()?);
        assert!(
            cache
                .get(identity(7, 1), &request("masked", Some("context-b"), "bug"))
                .is_none()
        );
        cache.insert(identity(7, 1), &baseline, decision()?);
        assert!(
            cache
                .get(
                    identity(7, 1),
                    &request("masked", Some("context-a"), "feature")
                )
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn cache_capacity_and_identity_change_clear_old_entries()
    -> Result<(), Box<dyn std::error::Error>> {
        let cache = DecisionCache::new();
        let original = request("text-0", None, "bug");
        for index in 0..=256 {
            cache.insert(
                identity(3, 1),
                &request(&format!("text-{index}"), None, "bug"),
                decision()?,
            );
        }
        assert!(cache.get(identity(3, 1), &original).is_none());
        let latest = request("text-256", None, "bug");
        assert!(cache.get(identity(3, 1), &latest).is_some());
        assert!(cache.get(identity(3, 2), &latest).is_none());
        Ok(())
    }

    #[test]
    fn health_identity_requires_a_complete_digest_and_nonzero_generation() {
        let mut response = pb::HealthResponse {
            protocol_version: Some(polaris_proto::PROTOCOL_VERSION.into()),
            build_version: "test".to_owned(),
            ready: true,
            loaded_model_identity_blake3: Vec::new(),
            loaded_model_generation: 0,
            worker_incarnation: vec![9; 16],
            loaded_pii_model_identity_blake3: Vec::new(),
            loaded_pii_model_generation: 0,
        };
        assert_eq!(ModelIdentity::from_health(&response), Ok(None));
        response.loaded_model_identity_blake3 = vec![4; 32];
        response.loaded_model_generation = 1;
        assert_eq!(
            ModelIdentity::from_health(&response),
            Ok(Some(identity(4, 1)))
        );
        response.loaded_model_generation = 0;
        assert!(ModelIdentity::from_health(&response).is_err());
        response.loaded_model_identity_blake3.truncate(31);
        response.loaded_model_generation = 2;
        assert!(ModelIdentity::from_health(&response).is_err());
        response.loaded_model_identity_blake3 = vec![4; 32];
        response.loaded_model_generation = 2;
        response.worker_incarnation.clear();
        assert!(ModelIdentity::from_health(&response).is_err());
    }

    #[test]
    fn worker_restart_changes_cache_identity_even_for_same_model_digest_and_generation() {
        let mut response = pb::HealthResponse {
            protocol_version: Some(polaris_proto::PROTOCOL_VERSION.into()),
            build_version: "worker".to_owned(),
            ready: true,
            loaded_model_identity_blake3: vec![4; 32],
            loaded_model_generation: 1,
            worker_incarnation: vec![7; 16],
            loaded_pii_model_identity_blake3: Vec::new(),
            loaded_pii_model_generation: 0,
        };
        let prior = ModelIdentity::from_health(&response);
        response.worker_incarnation = vec![8; 16];
        let restarted = ModelIdentity::from_health(&response);
        assert_ne!(prior, restarted);
        response.worker_incarnation = vec![8; 15];
        assert!(ModelIdentity::from_health(&response).is_err());
    }

    #[test]
    fn cache_digest_collision_with_different_request_bytes_is_a_miss()
    -> Result<(), Box<dyn std::error::Error>> {
        let decision = decision()?;
        let key = [5_u8; 32];
        let mut state = super::CacheState {
            entries: HashMap::new(),
            insertion_order: VecDeque::new(),
            total_bytes: 0,
            identity: None,
        };
        state.entries.insert(
            key,
            super::CacheEntry {
                request: b"trusted request".to_vec(),
                weight: 0,
                decision,
            },
        );
        assert!(super::lookup_entry(&state, key, b"different request").is_none());
        assert!(super::lookup_entry(&state, key, b"trusted request").is_some());
        Ok(())
    }
}
