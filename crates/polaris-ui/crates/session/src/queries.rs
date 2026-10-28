//! Registry queries that need no running kernel.
use crate::SessionHostError;
use polaris_kernel::storage::registry::{Hit, Registry, SessionFilter, SessionMeta};
use polaris_sdk::ids::SessionId;
use std::path::{Path, PathBuf};

/// Sessions held by the state directory, newest first, at most `limit` rows. Opens (and creates
/// if absent) the registry file under `root`; no kernel is started.
/// # Errors
/// Registry/schema/query or worker failure.
pub async fn list_sessions(root: &Path, limit: u32) -> Result<Vec<SessionMeta>, SessionHostError> {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let registry = Registry::open(&root)?;
        registry.list_sessions(&SessionFilter {
            limit: Some(limit),
            ..SessionFilter::default()
        })
    })
    .await
    .map_err(|_| SessionHostError::Unavailable("registry query owner failed"))?
    .map_err(|e| SessionHostError::Ui(format!("session registry query failed: {e}")))
}

/// Test one exact registry identity without applying any display-list limit.
/// # Errors
/// Registry/schema/query or worker failure.
pub async fn session_exists(root: &Path, id: SessionId) -> Result<bool, SessionHostError> {
    let root: PathBuf = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let registry = Registry::open(&root)?;
        registry.get_session(id).map(|session| session.is_some())
    })
    .await
    .map_err(|_| SessionHostError::Unavailable("session existence query owner failed"))?
    .map_err(|error| SessionHostError::Ui(format!("session existence query failed: {error}")))
}

/// Literal, case-insensitive search of the registry's message index, newest first, at most
/// `limit` hits (the registry caps it). The query is untrusted and bounded by the registry.
///
/// Only messages that were indexed are found. The kernel does not index typed user input yet, so
/// a freshly recorded message is not a hit: an empty result is normal, not an error.
/// # Errors
/// Registry/schema/query, oversize query or worker failure.
pub async fn search_history(
    root: &Path,
    query: &str,
    limit: u32,
) -> Result<Vec<Hit>, SessionHostError> {
    let root = root.to_path_buf();
    let query = query.to_owned();
    tokio::task::spawn_blocking(move || {
        let registry = Registry::open(&root)?;
        registry.search_history(&query, limit)
    })
    .await
    .map_err(|_| SessionHostError::Unavailable("history search owner failed"))?
    .map_err(|error| SessionHostError::Ui(format!("history search failed: {error}")))
}
