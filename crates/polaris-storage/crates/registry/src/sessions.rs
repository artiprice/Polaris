//! Session catalogue rows: metadata types and the SQL behind `upsert/get/list/set_status`.

use std::path::{Path, PathBuf};

use polaris_sdk::contracts::Timestamp;
use polaris_sdk::ids::SessionId;
use rusqlite::{Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::error::RegistryError;

/// Lifecycle state of a session in `/sessions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// In use or resumable at any time.
    Active,
    /// Suspended by the user.
    Paused,
    /// Hidden from the default list; kept for search.
    Archived,
}

impl SessionStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Archived => "archived",
        }
    }

    fn parse(text: &str) -> Result<Self, RegistryError> {
        match text {
            "active" => Ok(Self::Active),
            "paused" => Ok(Self::Paused),
            "archived" => Ok(Self::Archived),
            other => Err(RegistryError::Corrupt(format!(
                "unknown session status {other:?}"
            ))),
        }
    }
}

/// One row of the machine-wide session catalogue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMeta {
    /// Session id (primary key).
    pub id: SessionId,
    /// Display name.
    pub name: String,
    /// Workspace root, stored as UTF-8 text exactly as given (no normalisation).
    pub workspace: PathBuf,
    /// Git branch the session works on.
    pub branch: String,
    /// Commit the session started from.
    pub base_commit: String,
    /// Prompt tokens used so far.
    pub prompt_tokens: u64,
    /// Completion tokens produced so far.
    pub completion_tokens: u64,
    /// Latest sandbox checkpoint id, if any.
    pub sandbox_checkpoint: Option<String>,
    /// Lifecycle state.
    pub status: SessionStatus,
    /// When the session was created; written once, never changed by later upserts.
    pub created_at: Timestamp,
    /// When the session last changed (the `/sessions` sort key).
    pub updated_at: Timestamp,
}

/// Selects which sessions [`crate::Registry::list_sessions`] returns. The default selects all.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionFilter {
    /// Only sessions in this state.
    pub status: Option<SessionStatus>,
    /// Only sessions whose stored workspace equals this path exactly.
    pub workspace: Option<PathBuf>,
    /// At most this many rows (newest first).
    pub limit: Option<u32>,
}

const COLUMNS: &str = "id, name, workspace, branch, base_commit, prompt_tokens, \
                       completion_tokens, sandbox_checkpoint, status, created_at, updated_at";

/// Converts a timestamp to the stored microsecond count.
pub(crate) fn to_micros(at: Timestamp) -> i64 {
    at.as_microsecond()
}

/// Converts a stored microsecond count back to a timestamp.
pub(crate) fn from_micros(us: i64) -> Result<Timestamp, RegistryError> {
    Timestamp::from_microsecond(us)
        .map_err(|e| RegistryError::Corrupt(format!("bad stored timestamp {us}: {e}")))
}

fn path_text(path: &Path) -> Result<&str, RegistryError> {
    path.to_str()
        .ok_or_else(|| RegistryError::BadValue("workspace path is not valid UTF-8".to_owned()))
}

fn counter(value: u64, what: &str) -> Result<i64, RegistryError> {
    i64::try_from(value)
        .map_err(|_| RegistryError::BadValue(format!("{what} {value} exceeds i64::MAX")))
}

fn unsigned(value: i64, what: &str) -> Result<u64, RegistryError> {
    u64::try_from(value)
        .map_err(|_| RegistryError::Corrupt(format!("negative stored {what} {value}")))
}

/// A row exactly as SQLite stores it, before validation.
struct RawSession {
    id: String,
    name: String,
    workspace: String,
    branch: String,
    base_commit: String,
    prompt_tokens: i64,
    completion_tokens: i64,
    sandbox_checkpoint: Option<String>,
    status: String,
    created_at: i64,
    updated_at: i64,
}

impl RawSession {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            workspace: row.get(2)?,
            branch: row.get(3)?,
            base_commit: row.get(4)?,
            prompt_tokens: row.get(5)?,
            completion_tokens: row.get(6)?,
            sandbox_checkpoint: row.get(7)?,
            status: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    }

    fn into_meta(self) -> Result<SessionMeta, RegistryError> {
        Ok(SessionMeta {
            id: self
                .id
                .parse()
                .map_err(|e| RegistryError::Corrupt(format!("bad stored session id: {e}")))?,
            name: self.name,
            workspace: PathBuf::from(self.workspace),
            branch: self.branch,
            base_commit: self.base_commit,
            prompt_tokens: unsigned(self.prompt_tokens, "prompt_tokens")?,
            completion_tokens: unsigned(self.completion_tokens, "completion_tokens")?,
            sandbox_checkpoint: self.sandbox_checkpoint,
            status: SessionStatus::parse(&self.status)?,
            created_at: from_micros(self.created_at)?,
            updated_at: from_micros(self.updated_at)?,
        })
    }
}

/// Inserts the session or updates every column except `created_at`.
pub(crate) fn upsert(conn: &Connection, meta: &SessionMeta) -> Result<(), RegistryError> {
    let workspace = path_text(&meta.workspace)?;
    let prompt = counter(meta.prompt_tokens, "prompt_tokens")?;
    let completion = counter(meta.completion_tokens, "completion_tokens")?;
    let mut stmt = conn.prepare_cached(
        "INSERT INTO sessions (id, name, workspace, branch, base_commit, prompt_tokens, \
             completion_tokens, sandbox_checkpoint, status, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         ON CONFLICT (id) DO UPDATE SET \
             name = excluded.name, workspace = excluded.workspace, branch = excluded.branch, \
             base_commit = excluded.base_commit, prompt_tokens = excluded.prompt_tokens, \
             completion_tokens = excluded.completion_tokens, \
             sandbox_checkpoint = excluded.sandbox_checkpoint, status = excluded.status, \
             updated_at = excluded.updated_at",
    )?;
    stmt.execute((
        meta.id.to_string(),
        &meta.name,
        workspace,
        &meta.branch,
        &meta.base_commit,
        prompt,
        completion,
        &meta.sandbox_checkpoint,
        meta.status.as_str(),
        to_micros(meta.created_at),
        to_micros(meta.updated_at),
    ))?;
    Ok(())
}

/// The session with `id`, if any.
pub(crate) fn get(conn: &Connection, id: SessionId) -> Result<Option<SessionMeta>, RegistryError> {
    let mut stmt = conn.prepare_cached(&format!("SELECT {COLUMNS} FROM sessions WHERE id = ?1"))?;
    let raw = stmt
        .query_row([id.to_string()], RawSession::from_row)
        .optional()?;
    raw.map(RawSession::into_meta).transpose()
}

/// Sessions matching `filter`, newest `updated_at` first.
pub(crate) fn list(
    conn: &Connection,
    filter: &SessionFilter,
) -> Result<Vec<SessionMeta>, RegistryError> {
    let workspace = filter.workspace.as_deref().map(path_text).transpose()?;
    let limit = filter.limit.map_or(-1, i64::from);
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM sessions \
         WHERE (?1 IS NULL OR status = ?1) AND (?2 IS NULL OR workspace = ?2) \
         ORDER BY updated_at DESC, id DESC LIMIT ?3"
    ))?;
    let rows = stmt.query_map(
        (filter.status.map(SessionStatus::as_str), workspace, limit),
        RawSession::from_row,
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?.into_meta()?);
    }
    Ok(out)
}

/// Sets the status and `updated_at`; false when the session does not exist.
pub(crate) fn set_status(
    conn: &Connection,
    id: SessionId,
    status: SessionStatus,
    at: Timestamp,
) -> Result<bool, RegistryError> {
    let mut stmt =
        conn.prepare_cached("UPDATE sessions SET status = ?2, updated_at = ?3 WHERE id = ?1")?;
    let changed = stmt.execute((id.to_string(), status.as_str(), to_micros(at)))?;
    Ok(changed > 0)
}
