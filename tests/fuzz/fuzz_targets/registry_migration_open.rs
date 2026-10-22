//! Fuzz damaged SQLite headers through the production registry open/migration path.
#![no_main]

use std::sync::atomic::{AtomicU64, Ordering};

use libfuzzer_sys::fuzz_target;
use polaris_storage_registry::{RegistryConfig, open_connection};

static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
const MAX_DATABASE_INPUT: usize = 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_DATABASE_INPUT {
        return;
    }
    let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "polaris-registry-migration-fuzz-{}-{id}.db",
        std::process::id()
    ));
    if std::fs::write(&path, data).is_err() {
        return;
    }

    let config = RegistryConfig::default();
    if let Ok(connection) = open_connection(&path, &config) {
        let _: Result<String, _> =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(0));
        drop(connection);
    }

    let _ = std::fs::remove_file(&path);
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(sidecar);
    }
});
