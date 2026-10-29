//! History Indexing (descriptor: specs/descriptors/registry_history_indexing.json).

use std::error::Error;
use std::path::Path;

use polaris_sdk::contracts::Timestamp;
use polaris_sdk::ids::SessionId;
use polaris_sdk::registry::{ChatRole, RedactStats, Redactor};
use polaris_storage_registry::Registry;

type TestResult = Result<(), Box<dyn Error>>;

const SECRET: &str = "sk-live-9f8e7d6c5b4a3210";

/// Test double: masks one known secret.
struct MaskSecret;

impl Redactor for MaskSecret {
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
        let text = String::from_utf8_lossy(input);
        let masked = text.replace(SECRET, "[REDACTED]");
        out.extend_from_slice(masked.as_bytes());
        RedactStats {
            bytes_in: input.len(),
            replacements: u32::try_from(text.matches(SECRET).count()).unwrap_or(u32::MAX),
        }
    }
}

fn ts(seconds: i64) -> Result<Timestamp, Box<dyn Error>> {
    Ok(Timestamp::from_second(seconds)?)
}

/// True when any file under `dir` (db, -wal, -shm) contains `needle`.
fn any_file_contains(dir: &Path, needle: &str) -> Result<bool, Box<dyn Error>> {
    for entry in std::fs::read_dir(dir)? {
        let bytes = std::fs::read(entry?.path())?;
        if bytes.windows(needle.len()).any(|w| w == needle.as_bytes()) {
            return Ok(true);
        }
    }
    Ok(false)
}

#[test]
fn indexed_message_is_searchable() -> TestResult {
    let dir = tempfile::tempdir()?;
    let registry = Registry::open(dir.path())?;
    let session = SessionId::from_u128(1);
    registry.index_message(
        session,
        ChatRole::User,
        "please refactor the parser module",
        ts(100)?,
    )?;
    let hits = registry.search_history("refactor", 10)?;
    assert_eq!(hits.len(), 1);
    let hit = hits.first().ok_or("no hit")?;
    assert_eq!(hit.session, session);
    assert_eq!(hit.role, ChatRole::User);
    assert_eq!(hit.at, ts(100)?);
    assert!(hit.snippet.contains("refactor"));
    Ok(())
}

#[test]
fn roles_roundtrip() -> TestResult {
    let dir = tempfile::tempdir()?;
    let registry = Registry::open(dir.path())?;
    let session = SessionId::from_u128(1);
    let roles = [
        ChatRole::System,
        ChatRole::User,
        ChatRole::Assistant,
        ChatRole::Tool,
    ];
    for (i, role) in roles.iter().enumerate() {
        let at = ts(i64::try_from(i)?)?;
        registry.index_message(session, *role, &format!("role marker {i}"), at)?;
    }
    for (i, role) in roles.iter().enumerate() {
        let hits = registry.search_history(&format!("marker {i}"), 10)?;
        assert_eq!(hits.len(), 1, "marker {i}");
        assert_eq!(hits.first().map(|h| h.role), Some(*role));
    }
    Ok(())
}

#[test]
fn index_does_not_require_a_session_row() -> TestResult {
    let dir = tempfile::tempdir()?;
    let registry = Registry::open(dir.path())?;
    assert_eq!(registry.get_session(SessionId::from_u128(5))?, None);
    registry.index_message(
        SessionId::from_u128(5),
        ChatRole::Assistant,
        "orphan message",
        ts(1)?,
    )?;
    assert_eq!(registry.search_history("orphan", 5)?.len(), 1);
    Ok(())
}

#[test]
fn redacted_index_never_persists_secret() -> TestResult {
    let dir = tempfile::tempdir()?;
    let session = SessionId::from_u128(1);
    {
        let registry = Registry::open(dir.path())?;
        registry.index_message_redacted(
            session,
            ChatRole::User,
            &MaskSecret,
            &format!("my key is {SECRET} please use it"),
            ts(10)?,
        )?;
        // Live handle: the data may still be in the -wal file, which is scanned too.
        assert!(
            !any_file_contains(dir.path(), SECRET)?,
            "secret in live files"
        );
        assert!(registry.search_history(SECRET, 10)?.is_empty());
        assert_eq!(registry.search_history("[REDACTED]", 10)?.len(), 1);
    }
    assert!(
        !any_file_contains(dir.path(), SECRET)?,
        "secret in files after close"
    );
    Ok(())
}

#[test]
fn unmasked_index_does_persist_text_control() -> TestResult {
    // Negative control: proves the file scan above can fail. `index_message` stores text verbatim
    // (callers must mask), so the seeded secret is found.
    let dir = tempfile::tempdir()?;
    {
        let registry = Registry::open(dir.path())?;
        registry.index_message(
            SessionId::from_u128(1),
            ChatRole::User,
            &format!("my key is {SECRET}"),
            ts(10)?,
        )?;
    }
    assert!(any_file_contains(dir.path(), SECRET)?);
    Ok(())
}

#[test]
fn unicode_text_roundtrips() -> TestResult {
    let dir = tempfile::tempdir()?;
    let registry = Registry::open(dir.path())?;
    let text = "naïve café 日本語のテキスト 🚀 rocket";
    registry.index_message(SessionId::from_u128(1), ChatRole::User, text, ts(1)?)?;
    for query in ["café", "日本語", "🚀 r", "ïve"] {
        assert_eq!(registry.search_history(query, 5)?.len(), 1, "{query}");
    }
    Ok(())
}
