//! Schema Migration (descriptor: specs/descriptors/registry_schema_migration.json).

use std::error::Error;
use std::path::Path;

use polaris_sdk::contracts::Timestamp;
use polaris_sdk::ids::SessionId;
use polaris_storage_registry::{Registry, RegistryError, SCHEMA_VERSION, SessionStatus, migrate};
use rusqlite::Connection;

type TestResult = Result<(), Box<dyn Error>>;

fn user_version(conn: &Connection) -> Result<u32, Box<dyn Error>> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

fn table_sql(conn: &Connection, name: &str) -> Result<Option<String>, Box<dyn Error>> {
    let mut stmt = conn.prepare("SELECT sql FROM sqlite_master WHERE name = ?1")?;
    let mut rows = stmt.query([name])?;
    Ok(match rows.next()? {
        Some(row) => Some(row.get(0)?),
        None => None,
    })
}

fn columns(conn: &Connection, table: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(names)
}

/// The pre-release v0 layout: only four session columns, no history index, `user_version` 0.
const V0_SESSIONS: &str = "CREATE TABLE sessions (\
    id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL, workspace TEXT NOT NULL, \
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL)";

fn seed_v0(path: &Path, extra_column: bool) -> Result<(), Box<dyn Error>> {
    let conn = Connection::open(path)?;
    if extra_column {
        // A stray `base_commit` makes the v0 -> v1 `ALTER TABLE ... ADD COLUMN base_commit` fail
        // after `branch` was already added, which must roll the whole step back.
        conn.execute_batch(&V0_SESSIONS.replace(
            "updated_at INTEGER NOT NULL)",
            "updated_at INTEGER NOT NULL, base_commit TEXT)",
        ))?;
    } else {
        conn.execute_batch(V0_SESSIONS)?;
    }
    for n in 1..=2u128 {
        conn.execute(
            "INSERT INTO sessions (id, name, workspace, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (
                SessionId::from_u128(n).to_string(),
                format!("legacy {n}"),
                "old/work",
                1_000_000_i64 * i64::try_from(n)?,
                2_000_000_i64 * i64::try_from(n)?,
            ),
        )?;
    }
    Ok(())
}

#[test]
fn fresh_db_gets_latest_version() -> TestResult {
    let mut conn = Connection::open_in_memory()?;
    assert_eq!(user_version(&conn)?, 0);
    assert_eq!(migrate(&mut conn)?, SCHEMA_VERSION);
    assert_eq!(user_version(&conn)?, SCHEMA_VERSION);
    let sessions = columns(&conn, "sessions")?;
    for col in [
        "id",
        "name",
        "workspace",
        "branch",
        "base_commit",
        "prompt_tokens",
        "completion_tokens",
        "sandbox_checkpoint",
        "status",
        "created_at",
        "updated_at",
    ] {
        assert!(sessions.iter().any(|c| c == col), "missing column {col}");
    }
    let fts = table_sql(&conn, "session_history_fts")?.ok_or("fts table missing")?;
    assert!(fts.to_ascii_lowercase().contains("trigram"), "{fts}");
    Ok(())
}

#[test]
fn v0_upgrades_to_v1_keeping_rows() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("registry.db");
    seed_v0(&path, false)?;
    {
        let conn = Connection::open(&path)?;
        assert_eq!(user_version(&conn)?, 0);
    }
    let registry = Registry::open(dir.path())?;
    let list = registry.list_sessions(&Default::default())?;
    assert_eq!(list.len(), 2);
    let first = registry
        .get_session(SessionId::from_u128(1))?
        .ok_or("row lost")?;
    assert_eq!(first.name, "legacy 1");
    assert_eq!(first.workspace, Path::new("old/work"));
    assert_eq!(first.branch, "");
    assert_eq!(first.base_commit, "");
    assert_eq!((first.prompt_tokens, first.completion_tokens), (0, 0));
    assert_eq!(first.sandbox_checkpoint, None);
    assert_eq!(first.status, SessionStatus::Active);
    assert_eq!(first.created_at, Timestamp::from_microsecond(1_000_000)?);
    assert_eq!(first.updated_at, Timestamp::from_microsecond(2_000_000)?);
    drop(registry);
    let conn = Connection::open(&path)?;
    assert_eq!(user_version(&conn)?, SCHEMA_VERSION);
    Ok(())
}

#[test]
fn migrate_is_idempotent() -> TestResult {
    let mut conn = Connection::open_in_memory()?;
    migrate(&mut conn)?;
    conn.execute(
        "INSERT INTO sessions (id, name, workspace, created_at, updated_at) VALUES ('x', 'n', 'w', 1, 2)",
        [],
    )?;
    assert_eq!(migrate(&mut conn)?, SCHEMA_VERSION);
    assert_eq!(migrate(&mut conn)?, SCHEMA_VERSION);
    let rows: u32 = conn.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?;
    assert_eq!(rows, 1);
    Ok(())
}

#[test]
fn newer_version_is_refused() -> TestResult {
    let mut conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "user_version", 99)?;
    let err = migrate(&mut conn);
    assert!(
        matches!(
            err,
            Err(RegistryError::SchemaTooNew {
                found: 99,
                supported
            }) if supported == SCHEMA_VERSION
        ),
        "{err:?}"
    );
    assert_eq!(user_version(&conn)?, 99);
    assert_eq!(table_sql(&conn, "sessions")?, None, "nothing was created");
    Ok(())
}

#[test]
fn failed_migration_rolls_back() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("registry.db");
    seed_v0(&path, true)?;
    let before = {
        let conn = Connection::open(&path)?;
        columns(&conn, "sessions")?
    };
    let mut conn = Connection::open(&path)?;
    let err = migrate(&mut conn);
    assert!(
        matches!(err, Err(RegistryError::Migration { from: 0, to: 1, .. })),
        "{err:?}"
    );
    assert_eq!(user_version(&conn)?, 0);
    assert_eq!(columns(&conn, "sessions")?, before, "layout is untouched");
    assert_eq!(table_sql(&conn, "session_history_fts")?, None);
    let rows: u32 = conn.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?;
    assert_eq!(rows, 2, "rows survive the failed attempt");
    Ok(())
}
