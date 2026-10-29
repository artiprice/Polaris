//! Cache Key Derivation (descriptor: specs/descriptors/cache_key_derivation.json).

use std::error::Error;

use polaris_sdk::ids::{Cid, ModelId, SessionId};
use polaris_sdk::registry::CacheScope;
use polaris_storage_cache::{KeyKind, key_for_completion, key_for_embedding, key_for_s1};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn model(name: &str) -> Result<ModelId, Box<dyn Error>> {
    Ok(ModelId::new(name)?)
}

#[test]
fn same_inputs_same_key() -> TestResult {
    let m = model("nvidia/nemotron-3-super-120b-a12b")?;
    let a = key_for_completion(
        CacheScope::Global,
        &m,
        &[7; 32],
        &Cid::from_bytes([1; 32]),
        b"tail",
    );
    let b = key_for_completion(
        CacheScope::Global,
        &m,
        &[7; 32],
        &Cid::from_bytes([1; 32]),
        b"tail",
    );
    assert_eq!(a, b);
    assert_eq!(a.kind(), KeyKind::S2Completion);
    assert_eq!(a.scope(), &CacheScope::Global);
    Ok(())
}

#[test]
fn param_change_changes_key() -> TestResult {
    let m = model("nvidia/nemotron-3-super-120b-a12b")?;
    let prefix = Cid::from_bytes([1; 32]);
    let a = key_for_completion(CacheScope::Global, &m, &[7; 32], &prefix, b"tail");
    let b = key_for_completion(CacheScope::Global, &m, &[8; 32], &prefix, b"tail");
    assert_ne!(a.digest(), b.digest());
    Ok(())
}

#[test]
fn scope_isolates_keys() -> TestResult {
    let scopes = [
        CacheScope::Global,
        CacheScope::Workspace {
            workspace: Cid::from_bytes([1; 32]),
        },
        CacheScope::Workspace {
            workspace: Cid::from_bytes([2; 32]),
        },
        CacheScope::Session {
            session: SessionId::from_u128(42),
        },
    ];
    let digests: Vec<[u8; 32]> = scopes
        .iter()
        .map(|s| *key_for_s1(*s, &[3; 32], b"rm -rf /").digest())
        .collect();
    for (i, a) in digests.iter().enumerate() {
        for b in digests.iter().skip(i + 1) {
            assert_ne!(a, b, "two scopes share a key");
        }
    }
    Ok(())
}

#[test]
fn prefix_cid_changes_key() -> TestResult {
    let m = model("nvidia/nemotron-3-super-120b-a12b")?;
    let a = key_for_completion(
        CacheScope::Global,
        &m,
        &[7; 32],
        &Cid::from_bytes([1; 32]),
        b"tail",
    );
    let b = key_for_completion(
        CacheScope::Global,
        &m,
        &[7; 32],
        &Cid::from_bytes([2; 32]),
        b"tail",
    );
    assert_ne!(a.digest(), b.digest());
    Ok(())
}

#[test]
fn kind_separates_keys() -> TestResult {
    let m = model("ab")?;
    let s1 = key_for_s1(CacheScope::Global, &[0; 32], b"same bytes");
    let emb = key_for_embedding(CacheScope::Global, &m, b"same bytes");
    let s2 = key_for_completion(
        CacheScope::Global,
        &m,
        &[0; 32],
        &Cid::from_bytes([0; 32]),
        b"same bytes",
    );
    assert_eq!(s1.kind(), KeyKind::S1Decision);
    assert_eq!(emb.kind(), KeyKind::Embedding);
    assert_ne!(s1.digest(), emb.digest());
    assert_ne!(s1.digest(), s2.digest());
    assert_ne!(emb.digest(), s2.digest());
    Ok(())
}

#[test]
fn length_prefix_prevents_ambiguity() -> TestResult {
    let a = key_for_embedding(CacheScope::Global, &model("ab")?, b"c");
    let b = key_for_embedding(CacheScope::Global, &model("a")?, b"bc");
    assert_ne!(a.digest(), b.digest());
    Ok(())
}

proptest! {
    #[test]
    fn prop_distinct_inputs_distinct_keys(
        a in proptest::collection::vec(any::<u8>(), 0..64),
        b in proptest::collection::vec(any::<u8>(), 0..64),
    ) {
        prop_assume!(a != b);
        let ka = key_for_s1(CacheScope::Global, &[9; 32], &a);
        let kb = key_for_s1(CacheScope::Global, &[9; 32], &b);
        prop_assert_ne!(ka.digest(), kb.digest());
    }
}
