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
