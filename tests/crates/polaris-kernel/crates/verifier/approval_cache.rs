//! Skill approval cache contract tests consume witnesses produced by a real verifier.
use polaris_ext_skills::{SkillApprovalCache, SkillName};
use polaris_kernel_verifier::{EvidenceError, EvidenceStore, LocalDirectiveVerifier};
use polaris_sdk::{
    contracts::{ApprovalWitness, Command, CommandBody, Timestamp},
    ids::{Cid, CommandId, FrameId, SessionId},
    registry::{CacheScope, CacheStoreError, ExactCacheStore},
};
use std::{
    collections::BTreeMap,
    error::Error,
    sync::{Arc, Mutex},
    time::Duration,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

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
            let bytes = u64::try_from(value.len())
                .map_err(|_| CacheStoreError::Io("cache length overflow".to_owned()))?;
            let limit = u64::try_from(max_bytes)
                .map_err(|_| CacheStoreError::Io("cache limit overflow".to_owned()))?;
            return Err(CacheStoreError::TooLarge { bytes, limit });
        }
        Ok(Some(value.clone()))
    }

    fn put(&self, key: [u8; 32], value: &[u8], _scope: CacheScope) -> Result<(), CacheStoreError> {
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
        let count = u64::try_from(values.len())
            .map_err(|_| CacheStoreError::Io("cache count overflow".to_owned()))?;
        values.clear();
        Ok(count)
    }
}

#[derive(Default)]
struct MemoryEvidence(Mutex<BTreeMap<Cid, Vec<u8>>>);

impl EvidenceStore for MemoryEvidence {
    fn put(&self, bytes: &[u8]) -> Result<Cid, EvidenceError> {
        let cid = Cid::from_bytes(*blake3::hash(bytes).as_bytes());
        self.0
            .lock()
            .map_err(|_| EvidenceError::Backend)?
            .insert(cid, bytes.to_vec());
        Ok(cid)
    }

    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, EvidenceError> {
        self.0
            .lock()
            .map(|values| values.get(cid).cloned())
            .map_err(|_| EvidenceError::Backend)
    }
}

fn verifier_witness() -> TestResult<ApprovalWitness> {
    let frame = FrameId::from_u128(1);
    let verifier = LocalDirectiveVerifier::new(SessionId::from_u128(1), frame)?;
    let evidence = MemoryEvidence::default();
    let directive = Cid::from_bytes([3; 32]);
    let token = verifier.credential(Duration::from_secs(30))?;
    let command = Command::new(
        CommandId::from_u128(1),
        frame,
        token,
        CommandBody::ApplyDirective { directive },
    );
    let verified = verifier.verify(command, &evidence)?;
    Ok(verified.witness().clone())
}

#[tokio::test]
async fn approval_cache_roundtrips_by_workspace_and_source_evidence() -> TestResult {
    let approval = verifier_witness()?;
    let now = approval.at();
    let cache = MemoryCache::default();
    let one = SkillApprovalCache::new(cache.clone(), Cid::from_bytes([1; 32]));
    let other = SkillApprovalCache::new(cache, Cid::from_bytes([2; 32]));
    let name = SkillName::new("repo-skill")?;
    let evidence = approval.evidence();

    one.save(&name, evidence, &approval)?;
    if one.load(&name, evidence, now)? != Some(approval) {
        return Err("same workspace and source evidence did not roundtrip".into());
    }
    if other.load(&name, evidence, now)?.is_some() {
        return Err("approval leaked across workspace scope".into());
    }
    if one.load(&name, Cid::from_bytes([8; 32]), now)?.is_some() {
        return Err("approval matched changed source evidence".into());
    }
    Ok(())
}

#[tokio::test]
async fn expired_and_future_approvals_fail_closed() -> TestResult {
    let expired = verifier_witness()?;
    let expired_read_time = expired
        .at()
        .as_microsecond()
        .checked_add(366 * 24 * 60 * 60 * 1_000_000)
        .ok_or("expired read time overflow")?;
    let expired_read_time = Timestamp::from_microsecond(expired_read_time)?;
    let cache = SkillApprovalCache::new(MemoryCache::default(), Cid::from_bytes([1; 32]));
    let name = SkillName::new("repo-skill")?;
    let evidence = expired.evidence();
    cache.save(&name, evidence, &expired)?;
    if cache.load(&name, evidence, expired_read_time)?.is_some() {
        return Err("expired approval remained active".into());
    }

    let future = verifier_witness()?;
    let future_read_time = future
        .at()
        .as_microsecond()
        .checked_sub(1)
        .ok_or("future read time underflow")?;
    let future_read_time = Timestamp::from_microsecond(future_read_time)?;
    let future_evidence = future.evidence();
    cache.save(&name, future_evidence, &future)?;
    if cache
        .load(&name, future_evidence, future_read_time)?
        .is_some()
    {
        return Err("future-dated approval remained active".into());
    }
    Ok(())
}
