//! Message history indexing (the write side of `/sessions` and `polaris_recall` search).
//!
//! **Only masked text should be indexed.** The registry is a plain SQLite file: whatever is passed
//! to [`crate::Registry::index_message`] is stored verbatim. Callers either mask first or use
//! [`crate::Registry::index_message_redacted`], which runs a [`polaris_sdk::registry::Redactor`]
//! before anything reaches SQLite.

use polaris_sdk::contracts::Timestamp;
use polaris_sdk::ids::SessionId;
use polaris_sdk::registry::ChatRole;
use rusqlite::Connection;

use crate::error::RegistryError;
use crate::sessions::to_micros;

const fn role_str(role: ChatRole) -> &'static str {
    match role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Tool => "tool",
    }
}

/// Decodes a stored role name.
pub(crate) fn parse_role(text: &str) -> Result<ChatRole, RegistryError> {
    match text {
        "system" => Ok(ChatRole::System),
        "user" => Ok(ChatRole::User),
        "assistant" => Ok(ChatRole::Assistant),
        "tool" => Ok(ChatRole::Tool),
        other => Err(RegistryError::Corrupt(format!("unknown role {other:?}"))),
    }
}

/// Adds one message to the FTS5 trigram index.
pub(crate) fn index(
    conn: &Connection,
    session: SessionId,
    role: ChatRole,
    text: &str,
    at: Timestamp,
) -> Result<(), RegistryError> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO session_history_fts (text, session, role, at_us) VALUES (?1, ?2, ?3, ?4)",
    )?;
    stmt.execute((text, session.to_string(), role_str(role), to_micros(at)))?;
    Ok(())
}
