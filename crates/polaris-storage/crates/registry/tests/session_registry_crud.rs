//! Session Registry CRUD (descriptor: specs/descriptors/registry_session_registry_crud.json).

use std::error::Error;
use std::path::PathBuf;

use polaris_sdk::contracts::Timestamp;
use polaris_sdk::ids::SessionId;
use polaris_storage_registry::{
    Registry, RegistryError, SessionFilter, SessionMeta, SessionStatus,
};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn ts(seconds: i64) -> Result<Timestamp, Box<dyn Error>> {
    Ok(Timestamp::from_second(seconds)?)
}

fn meta(n: u128, updated: i64) -> Result<SessionMeta, Box<dyn Error>> {
    Ok(SessionMeta {
        id: SessionId::from_u128(n),
        name: format!("session {n}"),
        workspace: PathBuf::from("work/space"),
        branch: "main".to_owned(),
        base_commit: "abc123".to_owned(),
        prompt_tokens: 10,
        completion_tokens: 20,
        sandbox_checkpoint: Some("ckpt-1".to_owned()),
        status: SessionStatus::Active,
        created_at: ts(1_000)?,
        updated_at: ts(updated)?,
    })
}

fn open() -> Result<(tempfile::TempDir, Registry), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let registry = Registry::open(dir.path())?;
    Ok((dir, registry))
}

#[test]
fn upsert_then_get_roundtrip() -> TestResult {
    let (_dir, registry) = open()?;
    let m = meta(1, 2_000)?;
    registry.upsert_session(&m)?;
    assert_eq!(registry.get_session(m.id)?, Some(m.clone()));

    let mut without_ckpt = meta(2, 2_000)?;
    without_ckpt.sandbox_checkpoint = None;
    without_ckpt.status = SessionStatus::Paused;
    registry.upsert_session(&without_ckpt)?;
    assert_eq!(
        registry.get_session(without_ckpt.id)?,
        Some(without_ckpt.clone())
    );
    Ok(())
}

#[test]
fn get_unknown_returns_none() -> TestResult {
    let (_dir, registry) = open()?;
    assert_eq!(registry.get_session(SessionId::from_u128(99))?, None);
    Ok(())
}

#[test]
fn upsert_is_idempotent_and_keeps_created_at() -> TestResult {
    let (_dir, registry) = open()?;
    let m = meta(1, 2_000)?;
    registry.upsert_session(&m)?;
    registry.upsert_session(&m)?;
    assert_eq!(registry.list_sessions(&SessionFilter::default())?.len(), 1);

    let mut later = m.clone();
    later.created_at = ts(9_999)?;
    later.updated_at = ts(3_000)?;
    registry.upsert_session(&later)?;
    let stored = registry.get_session(m.id)?.ok_or("session vanished")?;
    assert_eq!(stored.created_at, m.created_at, "created_at is write-once");
    assert_eq!(stored.updated_at, ts(3_000)?);
    Ok(())
}

#[test]
fn upsert_updates_token_counters() -> TestResult {
    let (_dir, registry) = open()?;
    let mut m = meta(1, 2_000)?;
    registry.upsert_session(&m)?;
    m.name = "renamed".to_owned();
    m.branch = "feature".to_owned();
    m.prompt_tokens = 1_000;
    m.completion_tokens = 2_000;
    m.sandbox_checkpoint = None;
    m.status = SessionStatus::Paused;
    registry.upsert_session(&m)?;
    assert_eq!(registry.get_session(m.id)?, Some(m));
    Ok(())
}

#[test]
fn list_orders_by_updated_desc() -> TestResult {
    let (_dir, registry) = open()?;
    registry.upsert_session(&meta(1, 2_000)?)?;
    registry.upsert_session(&meta(2, 5_000)?)?;
    registry.upsert_session(&meta(3, 3_000)?)?;
    let ids: Vec<SessionId> = registry
        .list_sessions(&SessionFilter::default())?
        .iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(
        ids,
        vec![
            SessionId::from_u128(2),
            SessionId::from_u128(3),
            SessionId::from_u128(1)
        ]
    );
    Ok(())
}

#[test]
fn list_filters_by_status_workspace_and_limit() -> TestResult {
    let (_dir, registry) = open()?;
    let mut a = meta(1, 1_000)?;
    a.status = SessionStatus::Archived;
    let mut b = meta(2, 2_000)?;
    b.workspace = PathBuf::from("other");
    let c = meta(3, 3_000)?;
    for m in [&a, &b, &c] {
        registry.upsert_session(m)?;
    }

    let archived = registry.list_sessions(&SessionFilter {
        status: Some(SessionStatus::Archived),
        ..SessionFilter::default()
    })?;
    assert_eq!(archived, vec![a]);

    let other = registry.list_sessions(&SessionFilter {
        workspace: Some(PathBuf::from("other")),
        ..SessionFilter::default()
    })?;
    assert_eq!(other, vec![b]);

    let newest = registry.list_sessions(&SessionFilter {
        limit: Some(1),
        ..SessionFilter::default()
    })?;
    assert_eq!(newest, vec![c]);
    Ok(())
}

#[test]
fn archive_changes_status() -> TestResult {
    let (_dir, registry) = open()?;
    let m = meta(1, 2_000)?;
    registry.upsert_session(&m)?;

    assert!(registry.archive(m.id, ts(4_000)?)?);
    let archived = registry.get_session(m.id)?.ok_or("missing")?;
    assert_eq!(archived.status, SessionStatus::Archived);
    assert_eq!(archived.updated_at, ts(4_000)?);

    assert!(registry.unarchive(m.id, ts(5_000)?)?);
    let back = registry.get_session(m.id)?.ok_or("missing")?;
    assert_eq!(back.status, SessionStatus::Active);
    assert_eq!(back.updated_at, ts(5_000)?);

    assert!(!registry.archive(SessionId::from_u128(404), ts(6_000)?)?);
    assert!(registry.set_status(m.id, SessionStatus::Paused, ts(7_000)?)?);
    assert_eq!(
        registry.get_session(m.id)?.ok_or("missing")?.status,
        SessionStatus::Paused
    );
    Ok(())
}

#[test]
fn oversized_token_counter_is_rejected() -> TestResult {
    let (_dir, registry) = open()?;
    let mut m = meta(1, 2_000)?;
    m.prompt_tokens = u64::MAX;
    let err = registry.upsert_session(&m);
    assert!(matches!(err, Err(RegistryError::BadValue(_))), "{err:?}");
    assert_eq!(registry.get_session(m.id)?, None);
    Ok(())
}

fn roundtrip(name: &str, branch: &str, prompt: u64, completion: u64, secs: i64) -> TestResult {
    let (_dir, registry) = open()?;
    let mut m = meta(7, secs)?;
    m.name = name.to_owned();
    m.branch = branch.to_owned();
    m.prompt_tokens = prompt;
    m.completion_tokens = completion;
    registry.upsert_session(&m)?;
    assert_eq!(registry.get_session(m.id)?, Some(m));
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]
    #[test]
    fn prop_upsert_get_roundtrip(
        name in any::<String>(),
        branch in any::<String>(),
        prompt in 0..=i64::MAX,
        completion in 0..=i64::MAX,
        secs in 0i64..4_000_000_000,
    ) {
        let result = roundtrip(&name, &branch, prompt.unsigned_abs(), completion.unsigned_abs(), secs);
        prop_assert!(result.is_ok(), "{result:?}");
    }
}
