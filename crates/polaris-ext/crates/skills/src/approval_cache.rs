//! Bounded persistence of caller-verified repository skill approvals.

use polaris_sdk::contracts::{ApprovalWitness, Timestamp};
use polaris_sdk::ids::Cid;
use polaris_sdk::registry::{CacheScope, CacheStoreError, ExactCacheStore};

use crate::{SkillError, SkillName};

const MAX_APPROVAL_BYTES: usize = 4 * 1024;
const APPROVAL_TTL_MICROS: i64 = 365 * 24 * 60 * 60 * 1_000_000;
const APPROVAL_KEY_DOMAIN: &[u8] = b"polaris-skill-approval-cache-key-v1\0";

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredApproval {
    version: u8,
    witness: ApprovalWitness,
}

/// Workspace-scoped persistent approvals over the SDK's bounded exact-cache seam.
///
/// The witness remains caller-verified. This store only persists it; it never creates approval
/// authority. A matching source evidence digest and a non-expired timestamp are required on read.
pub struct SkillApprovalCache<S> {
    store: S,
    workspace: Cid,
}

impl<S: ExactCacheStore> SkillApprovalCache<S> {
    /// Creates a workspace-isolated approval cache.
    #[must_use]
    pub const fn new(store: S, workspace: Cid) -> Self {
        Self { store, workspace }
    }

    /// Saves a verified witness only when it binds the exact evidence being approved.
    pub fn save(
        &self,
        name: &SkillName,
        expected_evidence: Cid,
        witness: &ApprovalWitness,
    ) -> Result<(), SkillError> {
        if witness.evidence() != expected_evidence {
            return Err(SkillError::ApprovalMismatch(name.clone()));
        }
        let encoded = serde_saphyr::to_string(&StoredApproval {
            version: 1,
            witness: witness.clone(),
        })
        .map_err(|error| SkillError::ApprovalCache(error.to_string()))?;
        if encoded.len() > MAX_APPROVAL_BYTES {
            return Err(SkillError::ApprovalCache(
                "encoded witness exceeds bound".into(),
            ));
        }
        self.store
            .put(
                approval_key(self.workspace, name),
                encoded.as_bytes(),
                CacheScope::Workspace {
                    workspace: self.workspace,
                },
            )
            .map_err(cache_error)
    }

    /// Loads a witness only if it is bounded, well-formed, current, and unexpired.
    ///
    /// A missing, stale, or expired witness returns `None` and leaves the skill unapproved.
    /// Storage errors and corrupt records return an error so callers cannot accidentally treat
    /// a broken persistent record as successful approval.
    pub fn load(
        &self,
        name: &SkillName,
        expected_evidence: Cid,
        now: Timestamp,
    ) -> Result<Option<ApprovalWitness>, SkillError> {
        let Some(bytes) = self
            .store
            .get_bounded(&approval_key(self.workspace, name), MAX_APPROVAL_BYTES)
            .map_err(cache_error)?
        else {
            return Ok(None);
        };
        if bytes.len() > MAX_APPROVAL_BYTES {
            return Err(SkillError::ApprovalCache(
                "stored witness exceeds bound".into(),
            ));
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|error| SkillError::ApprovalCache(error.to_string()))?;
        let stored: StoredApproval = serde_saphyr::from_str(text)
            .map_err(|error| SkillError::ApprovalCache(error.to_string()))?;
        if stored.version != 1 {
            return Err(SkillError::ApprovalCache(
                "unknown witness record version".into(),
            ));
        }
        if stored.witness.evidence() != expected_evidence {
            return Ok(None);
        }
        let age = now
            .as_microsecond()
            .checked_sub(stored.witness.at().as_microsecond());
        let Some(age) = age else {
            return Ok(None);
        };
        if !(0..=APPROVAL_TTL_MICROS).contains(&age) {
            return Ok(None);
        }
        Ok(Some(stored.witness))
    }
}

fn approval_key(workspace: Cid, name: &SkillName) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(APPROVAL_KEY_DOMAIN);
    hasher.update(workspace.as_bytes());
    hasher.update(name.as_str().as_bytes());
    *hasher.finalize().as_bytes()
}

fn cache_error(error: CacheStoreError) -> SkillError {
    SkillError::ApprovalCache(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use polaris_sdk::contracts::{ApprovalWitness, ApproverId, Timestamp};
    use polaris_sdk::ids::Cid;
    use polaris_sdk::registry::{CacheScope, CacheStoreError, ExactCacheStore};

    use super::SkillApprovalCache;
    use crate::SkillName;

    #[derive(Clone, Default)]
    struct MemoryCache(Arc<Mutex<BTreeMap<[u8; 32], Vec<u8>>>>);

    impl ExactCacheStore for MemoryCache {
        fn get(&self, key: &[u8; 32]) -> Result<Option<Vec<u8>>, CacheStoreError> {
            self.0
                .lock()
                .map(|values| values.get(key).cloned())
                .map_err(|error| CacheStoreError::Io(error.to_string()))
        }

        fn get_bounded(
            &self,
            key: &[u8; 32],
            max_bytes: usize,
        ) -> Result<Option<Vec<u8>>, CacheStoreError> {
            let values = self
                .0
                .lock()
                .map_err(|error| CacheStoreError::Io(error.to_string()))?;
            let Some(value) = values.get(key) else {
                return Ok(None);
            };
            if value.len() > max_bytes {
                return Err(CacheStoreError::TooLarge {
                    bytes: u64::try_from(value.len()).unwrap_or(u64::MAX),
                    limit: u64::try_from(max_bytes).unwrap_or(u64::MAX),
                });
            }
            Ok(Some(value.clone()))
        }

        fn put(
            &self,
            key: [u8; 32],
            value: &[u8],
            _scope: CacheScope,
        ) -> Result<(), CacheStoreError> {
            self.0
                .lock()
                .map(|mut values| {
                    values.insert(key, value.to_vec());
                })
                .map_err(|error| CacheStoreError::Io(error.to_string()))
        }

        fn clear_scope(&self, _scope: &CacheScope) -> Result<u64, CacheStoreError> {
            let mut values = self
                .0
                .lock()
                .map_err(|error| CacheStoreError::Io(error.to_string()))?;
            let count = u64::try_from(values.len()).unwrap_or(u64::MAX);
            values.clear();
            Ok(count)
        }
    }

    fn witness(evidence: Cid, at: Timestamp) -> ApprovalWitness {
        ApprovalWitness::new(
            ApproverId::new("trusted-reviewer").expect("valid fixture approver"),
            evidence,
            at,
        )
    }

    #[test]
    fn approval_cache_roundtrips_by_workspace_and_source_evidence() {
        let cache = MemoryCache::default();
        let one = SkillApprovalCache::new(cache.clone(), Cid::from_bytes([1; 32]));
        let other = SkillApprovalCache::new(cache, Cid::from_bytes([2; 32]));
        let name = SkillName::new("repo-skill").expect("valid skill name");
        let evidence = Cid::from_bytes([7; 32]);
        let now = Timestamp::from_microsecond(2_000_000_000_000_000).expect("valid test timestamp");
        let approval = witness(evidence, now);

        one.save(&name, evidence, &approval)
            .expect("save verified approval");
        assert_eq!(
            one.load(&name, evidence, now).expect("load approval"),
            Some(approval)
        );
        assert_eq!(
            other.load(&name, evidence, now).expect("other workspace"),
            None
        );
        assert_eq!(
            one.load(&name, Cid::from_bytes([8; 32]), now)
                .expect("changed source"),
            None
        );
    }

    #[test]
    fn expired_and_future_approvals_fail_closed() {
        let store = MemoryCache::default();
        let cache = SkillApprovalCache::new(store, Cid::from_bytes([1; 32]));
        let name = SkillName::new("repo-skill").expect("valid skill name");
        let evidence = Cid::from_bytes([7; 32]);
        let now = Timestamp::from_microsecond(2_000_000_000_000_000).expect("valid test timestamp");
        let expired = witness(
            evidence,
            Timestamp::from_microsecond(now.as_microsecond() - 366 * 24 * 60 * 60 * 1_000_000)
                .expect("valid expired timestamp"),
        );
        cache
            .save(&name, evidence, &expired)
            .expect("save old approval");
        assert_eq!(
            cache
                .load(&name, evidence, now)
                .expect("expired is unapproved"),
            None
        );

        let future = witness(
            evidence,
            Timestamp::from_microsecond(now.as_microsecond() + 1).expect("valid future timestamp"),
        );
        cache
            .save(&name, evidence, &future)
            .expect("save future-dated approval");
        assert_eq!(
            cache
                .load(&name, evidence, now)
                .expect("future is unapproved"),
            None
        );
    }
}
