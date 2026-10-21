//! Database Open (descriptor: specs/descriptors/registry_database_open.json).

use std::error::Error;
use std::path::Path;
use std::time::Duration;

use polaris_sdk::contracts::Timestamp;
use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::ids::SessionId;
use polaris_sdk::registry::ChatRole;
use polaris_storage_registry::{
    DB_FILE_NAME, Registry, RegistryConfig, RegistryError, SCHEMA_VERSION, SessionFilter,
    SessionMeta, SessionStatus, open_connection,
};
use rusqlite::Connection;

type TestResult = Result<(), Box<dyn Error>>;

fn ts(seconds: i64) -> Result<Timestamp, Box<dyn Error>> {
    Ok(Timestamp::from_second(seconds)?)
}

fn session(n: u128) -> Result<SessionMeta, Box<dyn Error>> {
    Ok(SessionMeta {
        id: SessionId::from_u128(n),
        name: format!("s{n}"),
        workspace: "w".into(),
        branch: "main".to_owned(),
        base_commit: "c0ffee".to_owned(),
        prompt_tokens: 1,
        completion_tokens: 2,
        sandbox_checkpoint: None,
        status: SessionStatus::Active,
        created_at: ts(1)?,
        updated_at: ts(2)?,
    })
}

#[test]
fn bundled_sqlite_has_fts5_trigram() -> TestResult {
    // Go/no-go gate for the whole crate (PLAN TODO): the bundled SQLite must offer FTS5 + trigram.
    assert!(
        rusqlite::version_number() >= 3_034_000,
        "SQLite {} is older than 3.34 (trigram tokenizer)",
        rusqlite::version()
    );
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(
        "CREATE VIRTUAL TABLE t USING fts5(body, tokenize='trigram');
         INSERT INTO t(body) VALUES ('hello world');",
    )?;
    let hits: u32 = conn.query_row(
        "SELECT count(*) FROM t WHERE t MATCH '\"lo wo\"'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(hits, 1);
    Ok(())
}

#[test]
fn open_creates_wal_database() -> TestResult {
    let root = tempfile::tempdir()?;
    let dir = root.path().join("nested").join("registry");
    let registry = Registry::open(&dir)?;
    let db = dir.join(DB_FILE_NAME);
    assert_eq!(registry.db_path(), db);
    assert!(db.is_file());
    let conn = Connection::open(&db)?;
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    assert_eq!(mode, "wal");
    let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    assert_eq!(version, SCHEMA_VERSION);
    Ok(())
}

#[test]
fn connection_pragmas_are_applied() -> TestResult {
    let dir = tempfile::tempdir()?;
    let cfg = RegistryConfig {
        busy_timeout: Duration::from_millis(1234),
        ..RegistryConfig::default()
    };
    let conn = open_connection(&dir.path().join(DB_FILE_NAME), &cfg)?;
    let pragma = |name: &str| -> Result<i64, rusqlite::Error> {
        conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
    };
    assert_eq!(pragma("foreign_keys")?, 1);
    assert_eq!(pragma("synchronous")?, 1, "NORMAL");
    assert_eq!(pragma("busy_timeout")?, 1234);
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    assert_eq!(mode, "wal");
    Ok(())
}

#[test]
fn reopen_preserves_data() -> TestResult {
    let dir = tempfile::tempdir()?;
    let m = session(1)?;
    {
        let registry = Registry::open(dir.path())?;
        registry.upsert_session(&m)?;
        registry.index_message(m.id, ChatRole::User, "persisted words", ts(3)?)?;
    }
    let registry = Registry::open(dir.path())?;
    assert_eq!(registry.get_session(m.id)?, Some(m));
    assert_eq!(registry.search_history("persisted", 5)?.len(), 1);
    Ok(())
}

#[test]
fn path_with_spaces_and_non_ascii_works() -> TestResult {
    let root = tempfile::tempdir()?;
    let dir = root.path().join("dir with space").join("événements-日本");
    let registry = Registry::open(&dir)?;
    registry.upsert_session(&session(1)?)?;
    registry.index_message(
        SessionId::from_u128(1),
        ChatRole::User,
        "chemin accentué",
        ts(3)?,
    )?;
    assert!(registry.get_session(SessionId::from_u128(1))?.is_some());
    assert_eq!(registry.search_history("accentu", 5)?.len(), 1);
    assert!(dir.join(DB_FILE_NAME).is_file());
    Ok(())
}

#[test]
fn two_handles_share_one_database() -> TestResult {
    let dir = tempfile::tempdir()?;
    let a = Registry::open(dir.path())?;
    let b = Registry::open(dir.path())?;
    a.upsert_session(&session(1)?)?;
    assert!(b.get_session(SessionId::from_u128(1))?.is_some());
    b.index_message(
        SessionId::from_u128(1),
        ChatRole::Tool,
        "from handle b",
        ts(5)?,
    )?;
    assert_eq!(a.search_history("handle b", 5)?.len(), 1);
    Ok(())
}

const WRITES: i64 = 150;

fn write_under_load(path: &Path) -> TestResult {
    let registry = Registry::open(path)?;
    for i in 1..=WRITES {
        registry.index_message(
            SessionId::from_u128(1),
            ChatRole::Assistant,
            &format!("written under load {i}"),
            ts(i)?,
        )?;
    }
    Ok(())
}

fn read_under_load(path: &Path) -> TestResult {
    let registry = Registry::open(path)?;
    for _ in 0..60 {
        assert!(!registry.search_history("seed", 10)?.is_empty());
        registry.search_history("under load", 10)?;
        registry.list_sessions(&SessionFilter::default())?;
    }
    Ok(())
}

#[test]
fn concurrent_readers_while_one_writer_inserts() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path();
    Registry::open(path)?.index_message(
        SessionId::from_u128(1),
        ChatRole::User,
        "seed message",
        ts(0)?,
    )?;
    std::thread::scope(|scope| -> TestResult {
        // Thread results carry `String` errors: `Box<dyn Error>` is not `Send`.
        let writer = scope.spawn(|| write_under_load(path).map_err(|e| e.to_string()));
        let readers: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| read_under_load(path).map_err(|e| e.to_string())))
            .collect();
        for reader in readers {
            reader.join().map_err(|_| "reader thread panicked")??;
        }
        writer.join().map_err(|_| "writer thread panicked")??;
        Ok(())
    })?;
    let registry = Registry::open(path)?;
    let all = registry.search_history("under load", u32::MAX)?;
    assert_eq!(all.len(), usize::try_from(WRITES)?);
    Ok(())
}

#[test]
fn busy_is_reported_when_writer_is_blocked() -> TestResult {
    let dir = tempfile::tempdir()?;
    let cfg = RegistryConfig {
        busy_timeout: Duration::from_millis(20),
        busy_retries: 2,
        retry_backoff: Duration::from_millis(10),
    };
    let registry = Registry::open_with(dir.path(), &cfg)?;
    let blocker = Connection::open(dir.path().join(DB_FILE_NAME))?;
    blocker.execute_batch("BEGIN IMMEDIATE")?;

    let blocked =
        registry.index_message(SessionId::from_u128(1), ChatRole::User, "blocked", ts(1)?);
    let err = blocked.err().ok_or("write unexpectedly succeeded")?;
    assert!(matches!(err, RegistryError::Busy), "{err:?}");
    assert_eq!(err.kind(), ErrorKind::Transient);
    // Reads are not blocked by the writer in WAL mode.
    assert!(registry.get_session(SessionId::from_u128(1))?.is_none());

    blocker.execute_batch("COMMIT")?;
    registry.index_message(SessionId::from_u128(1), ChatRole::User, "unblocked", ts(2)?)?;
    assert_eq!(registry.search_history("unblocked", 5)?.len(), 1);
    assert!(
        registry.search_history("blocked", 5)?.len() == 1,
        "only the second write landed"
    );
    Ok(())
}

#[test]
fn garbage_file_is_rejected() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join(DB_FILE_NAME);
    let garbage = b"this is definitely not a sqlite database file".repeat(40);
    std::fs::write(&path, &garbage)?;
    let err = Registry::open(dir.path())
        .err()
        .ok_or("garbage was accepted")?;
    assert_eq!(err.kind(), ErrorKind::Fatal, "{err:?}");
    assert_eq!(std::fs::read(&path)?, garbage, "the file is left untouched");
    Ok(())
}

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn handle_is_send_and_sync() -> TestResult {
    assert_send_sync::<Registry>();
    Ok(())
}

#[test]
fn error_kinds_are_classified() -> TestResult {
    assert_eq!(RegistryError::Busy.kind(), ErrorKind::Transient);
    assert_eq!(
        RegistryError::SchemaTooNew {
            found: 9,
            supported: 1
        }
        .kind(),
        ErrorKind::Fatal
    );
    assert_eq!(RegistryError::Corrupt("x".into()).kind(), ErrorKind::Fatal);
    assert_eq!(
        RegistryError::BadValue("x".into()).kind(),
        ErrorKind::Permanent
    );
    assert_eq!(RegistryError::Open("x".into()).kind(), ErrorKind::Permanent);
    assert_eq!(RegistryError::Closed.kind(), ErrorKind::Permanent);
    Ok(())
}
