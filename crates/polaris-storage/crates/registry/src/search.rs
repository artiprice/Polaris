//! History search: query sanitising, the trigram `MATCH` path and the `LIKE` fallback.
//!
//! Queries are untrusted text matched as **literal substrings**. A query of three or more
//! characters becomes one quoted FTS5 phrase (so `OR`, `NEAR`, `*`, `col:` and friends carry no
//! meaning); shorter queries cannot form a trigram and use an escaped `LIKE` scan instead. Both
//! paths are case-insensitive for ASCII; the trigram path folds all of Unicode.

use polaris_sdk::contracts::Timestamp;
use polaris_sdk::ids::SessionId;
use polaris_sdk::registry::ChatRole;
use rusqlite::{Connection, Row};
use serde::{Deserialize, Serialize};

use crate::error::RegistryError;
use crate::history::parse_role;
use crate::sessions::from_micros;

/// Most hits one search returns, whatever `limit` the caller asks for.
pub const MAX_SEARCH_LIMIT: u32 = 1000;

/// Longest snippet, in characters.
pub const SNIPPET_CHARS: usize = 160;

/// Largest raw query accepted by search before it is copied to the database thread.
pub const MAX_SEARCH_QUERY_BYTES: usize = 16 * 1024;

/// Characters of context kept before the match in a snippet.
const SNIPPET_LEAD: usize = 40;

/// Fewest characters that form a trigram.
const TRIGRAM_MIN_CHARS: usize = 3;

/// One search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    /// Session the message belongs to.
    pub session: SessionId,
    /// Author of the message.
    pub role: ChatRole,
    /// A window of at most [`SNIPPET_CHARS`] characters around the first match.
    pub snippet: String,
    /// When the message was indexed.
    pub at: Timestamp,
}

/// The query as a single FTS5 phrase: wrapped in double quotes with embedded quotes doubled.
/// NUL characters are dropped first (SQLite would truncate the text there).
#[must_use]
pub fn fts_phrase(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 2);
    out.push('"');
    for c in query.chars().filter(|c| *c != '\0') {
        if c == '"' {
            out.push('"');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The query as a `LIKE` pattern for `ESCAPE '\'`: `%`, `_` and `\` are escaped and the text is
/// wrapped in `%...%`. NUL characters are dropped first.
#[must_use]
pub fn like_pattern(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 2);
    out.push('%');
    for c in query.chars().filter(|c| *c != '\0') {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// A window of `text` around the first occurrence of `query` (or its start when not found).
fn snippet(text: &str, query: &str) -> String {
    let total = text.chars().count();
    if total <= SNIPPET_CHARS {
        return text.to_owned();
    }
    let (lowered, source_chars) = lowercase_with_source_offsets(text);
    let needle = query.to_lowercase();
    let match_char = lowered
        .find(&needle)
        .and_then(|byte| {
            let index = source_chars.partition_point(|(_, end, _)| *end <= byte);
            source_chars
                .get(index)
                .filter(|(start, end, _)| *start <= byte && byte < *end)
                .map(|(_, _, source_char)| *source_char)
        })
        .unwrap_or(0);
    let start = match_char
        .saturating_sub(SNIPPET_LEAD)
        .min(total.saturating_sub(SNIPPET_CHARS));
    text.chars().skip(start).take(SNIPPET_CHARS).collect()
}

/// Lowercases text while retaining a mapping from every folded character span to its source
/// character index. Unicode lowercase can expand one scalar (for example `İ`) into multiple
/// scalars, so counting characters in the folded string does not locate source snippet boundaries.
fn lowercase_with_source_offsets(text: &str) -> (String, Vec<(usize, usize, usize)>) {
    let mut lowered = String::with_capacity(text.len());
    let mut source_chars = Vec::with_capacity(text.chars().count());
    for (source_char, character) in text.chars().enumerate() {
        let start = lowered.len();
        lowered.extend(character.to_lowercase());
        source_chars.push((start, lowered.len(), source_char));
    }
    (lowered, source_chars)
}

/// Runs the search; see [`crate::Registry::search_history`].
pub(crate) fn search(
    conn: &Connection,
    query: &str,
    limit: u32,
) -> Result<Vec<Hit>, RegistryError> {
    search_in_session(conn, None, query, limit)
}

/// Runs a literal search restricted to one session in SQL, before ordering and limiting.
pub(crate) fn search_for_session(
    conn: &Connection,
    session: SessionId,
    query: &str,
    limit: u32,
) -> Result<Vec<Hit>, RegistryError> {
    search_in_session(conn, Some(session), query, limit)
}

type RawHit = (String, String, i64, String);

fn raw_hit(row: &Row<'_>) -> rusqlite::Result<RawHit> {
    Ok((
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, i64>(2)?,
        row.get::<_, String>(3)?,
    ))
}

fn search_in_session(
    conn: &Connection,
    session: Option<SessionId>,
    query: &str,
    limit: u32,
) -> Result<Vec<Hit>, RegistryError> {
    if query.len() > MAX_SEARCH_QUERY_BYTES {
        return Err(RegistryError::BadValue(format!(
            "search query exceeds the {MAX_SEARCH_QUERY_BYTES}-byte limit"
        )));
    }
    let limit = limit.min(MAX_SEARCH_LIMIT);
    let cleaned: String = query.chars().filter(|c| *c != '\0').collect();
    if limit == 0 || cleaned.is_empty() {
        return Ok(Vec::new());
    }
    let (sql, param) = if cleaned.chars().count() < TRIGRAM_MIN_CHARS {
        let sql = if session.is_some() {
            "SELECT session, role, at_us, text FROM session_history_fts \
             WHERE text LIKE ?1 ESCAPE '\\' AND session = ?2 ORDER BY at_us DESC, rowid DESC LIMIT ?3"
        } else {
            "SELECT session, role, at_us, text FROM session_history_fts \
             WHERE text LIKE ?1 ESCAPE '\\' ORDER BY at_us DESC, rowid DESC LIMIT ?2"
        };
        (sql, like_pattern(&cleaned))
    } else {
        let sql = if session.is_some() {
            "SELECT session, role, at_us, text FROM session_history_fts \
             WHERE session_history_fts MATCH ?1 AND session = ?2 ORDER BY at_us DESC, rowid DESC LIMIT ?3"
        } else {
            "SELECT session, role, at_us, text FROM session_history_fts \
             WHERE session_history_fts MATCH ?1 ORDER BY at_us DESC, rowid DESC LIMIT ?2"
        };
        (sql, fts_phrase(&cleaned))
    };
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = match session {
        Some(session) => stmt.query_map((param, session.to_string(), i64::from(limit)), raw_hit)?,
        None => stmt.query_map((param, i64::from(limit)), raw_hit)?,
    };
    let mut hits = Vec::new();
    for row in rows {
        let (session, role, at_us, text) = row?;
        hits.push(Hit {
            session: session
                .parse()
                .map_err(|e| RegistryError::Corrupt(format!("bad stored session id: {e}")))?,
            role: parse_role(&role)?,
            snippet: snippet(&text, &cleaned),
            at: from_micros(at_us)?,
        });
    }
    Ok(hits)
}
