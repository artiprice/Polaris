//! Fuzz arbitrary byte queries through the production bounded FTS and LIKE search API.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use polaris_sdk::{contracts::Timestamp, ids::SessionId, registry::ChatRole};
use polaris_storage_registry::{MAX_SEARCH_QUERY_BYTES, Registry, RegistryError};

static REGISTRY: OnceLock<Option<Registry>> = OnceLock::new();

fn registry() -> Option<&'static Registry> {
    REGISTRY
        .get_or_init(|| {
            let path = std::env::temp_dir()
                .join(format!("polaris-registry-fts-fuzz-{}", std::process::id()));
            let opened = Registry::open(&path);
            assert!(opened.is_ok(), "fuzz fixture database must open");
            let Ok(registry) = opened else {
                return None;
            };
            for (id, text) in [
                (
                    1,
                    "synthetic fixture compiler αβγ unicode İSTANBUL OR \"quoted\"",
                ),
                (2, "synthetic fixture unrelated other-session result"),
            ] {
                assert!(
                    registry
                        .index_message(
                            SessionId::from_u128(id),
                            ChatRole::User,
                            text,
                            Timestamp::UNIX_EPOCH
                        )
                        .is_ok(),
                    "fuzz fixture rows must persist"
                );
            }
            Some(registry)
        })
        .as_ref()
}

fuzz_target!(|data: &[u8]| {
    // Exercise the upper-bound edge while keeping a single libFuzzer input bounded in memory.
    if data.len() > MAX_SEARCH_QUERY_BYTES.saturating_add(1) {
        return;
    }
    let query = String::from_utf8_lossy(data);
    let Some(registry) = registry() else {
        return;
    };
    let all_sessions = registry.search_history(&query, 32);
    assert!(
        all_sessions.is_ok() || matches!(&all_sessions, Err(RegistryError::BadValue(_))),
        "bounded literal query reached a registry or SQL error"
    );
    let one_session = registry.search_history_for_session(SessionId::from_u128(1), &query, 32);
    assert!(
        one_session.is_ok() || matches!(&one_session, Err(RegistryError::BadValue(_))),
        "bounded scoped query reached a registry or SQL error"
    );
});
