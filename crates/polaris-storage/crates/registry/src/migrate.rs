//! Forward-only schema migrations driven by `PRAGMA user_version`.
//!
//! Layout history:
//!
//! - **v0** (pre-release): `sessions(id, name, workspace, created_at, updated_at)` only, no history
//!   index. A brand-new empty database is also `user_version` 0, so the 0 -> 1 step creates the
//!   v0 table when it is missing and then upgrades it; fresh and legacy files share one code path.
//! - **v1**: adds branch, base commit, token counters, sandbox checkpoint and status to
//!   `sessions`, an `updated_at` index, and the `session_history_fts` FTS5 trigram table.

use rusqlite::{Connection, TransactionBehavior};

use crate::error::RegistryError;

/// Schema version written by this build.
pub const SCHEMA_VERSION: u32 = 1;

/// `MIGRATIONS[n]` upgrades schema `n` to `n + 1`.
const MIGRATIONS: &[&str] = &[
    // 0 -> 1
    "CREATE TABLE IF NOT EXISTS sessions (
         id TEXT PRIMARY KEY NOT NULL,
         name TEXT NOT NULL,
         workspace TEXT NOT NULL,
         created_at INTEGER NOT NULL,
         updated_at INTEGER NOT NULL
     );
     ALTER TABLE sessions ADD COLUMN branch TEXT NOT NULL DEFAULT '';
     ALTER TABLE sessions ADD COLUMN base_commit TEXT NOT NULL DEFAULT '';
     ALTER TABLE sessions ADD COLUMN prompt_tokens INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE sessions ADD COLUMN completion_tokens INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE sessions ADD COLUMN sandbox_checkpoint TEXT;
     ALTER TABLE sessions ADD COLUMN status TEXT NOT NULL DEFAULT 'active';
     CREATE INDEX IF NOT EXISTS sessions_by_updated ON sessions (updated_at DESC);
     CREATE VIRTUAL TABLE session_history_fts USING fts5(
         text,
         session UNINDEXED,
         role UNINDEXED,
         at_us UNINDEXED,
         tokenize = 'trigram'
     );",
];

/// Brings `conn` up to [`SCHEMA_VERSION`] and returns the resulting version.
///
/// The version is read inside an immediate (write) transaction, so two processes opening a new
/// registry at once serialise and the loser sees the finished schema. All pending steps and the
/// `user_version` bump commit together: a failing step leaves the file exactly as it was.
///
/// # Errors
/// [`RegistryError::SchemaTooNew`] when the file is newer than this build,
/// [`RegistryError::Migration`] when a step fails (rolled back), [`RegistryError::Busy`] when the
/// write lock cannot be taken, [`RegistryError::Sql`] otherwise.
pub fn migrate(conn: &mut Connection) -> Result<u32, RegistryError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let found: u32 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if found > SCHEMA_VERSION {
        return Err(RegistryError::SchemaTooNew {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    if found == SCHEMA_VERSION {
        return Ok(found);
    }
    let mut version = found;
    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let from = u32::try_from(index).unwrap_or(u32::MAX);
        if from < version {
            continue;
        }
        let to = from.saturating_add(1);
        tx.execute_batch(sql)
            .map_err(|e| RegistryError::Migration {
                from: found,
                to,
                reason: e.to_string(),
            })?;
        version = to;
    }
    if version != SCHEMA_VERSION {
        return Err(RegistryError::Migration {
            from: found,
            to: SCHEMA_VERSION,
            reason: "migration table does not reach the current schema version".to_owned(),
        });
    }
    tx.pragma_update(None, "user_version", version)?;
    tx.commit()?;
    Ok(version)
}
