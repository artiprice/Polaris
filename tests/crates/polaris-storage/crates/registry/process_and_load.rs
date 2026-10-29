//! Synthetic workload measurements and real process crash/concurrency recovery.
use polaris_sdk::{contracts::Timestamp, ids::SessionId, registry::ChatRole};
use polaris_storage_registry::{DB_FILE_NAME, Registry, RegistryConfig, open_connection};
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};
type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn registry_writer_child() -> TestResult {
    let Some(root) = std::env::var_os("REGISTRY_TEST_ROOT") else {
        return Ok(());
    };
    let marker = std::env::var("REGISTRY_TEST_MARKER")?;
    let count: u64 = std::env::var("REGISTRY_TEST_COUNT")?.parse()?;
    let registry = Registry::open(std::path::Path::new(&root))?;
    for index in 0..count {
        registry.index_message(
            SessionId::from_u128(1),
            ChatRole::User,
            &format!("synthetic {marker} record {index}"),
            Timestamp::UNIX_EPOCH,
        )?;
        if index == 10 {
            std::fs::write(
                std::path::Path::new(&root).join(format!("{marker}.ready")),
                b"committed",
            )?;
        }
    }
    Ok(())
}

fn child(
    root: &std::path::Path,
    marker: &str,
    count: &str,
) -> Result<std::process::Child, std::io::Error> {
    Command::new(std::env::current_exe()?)
        .args(["--exact", "registry_writer_child", "--nocapture"])
        .env("REGISTRY_TEST_ROOT", root)
        .env("REGISTRY_TEST_MARKER", marker)
        .env("REGISTRY_TEST_COUNT", count)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
}

#[test]
fn two_process_writers_preserve_all_commits() -> TestResult {
    let root = tempfile::tempdir()?;
    drop(Registry::open(root.path())?);
    let mut left = child(root.path(), "left", "100")?;
    let mut right = child(root.path(), "right", "100")?;
    assert!(left.wait()?.success(), "left writer failed");
    assert!(right.wait()?.success(), "right writer failed");
    let connection = open_connection(&root.path().join(DB_FILE_NAME), &RegistryConfig::default())?;
    let count: i64 =
        connection.query_row("SELECT count(*) FROM session_history_fts", [], |row| {
            row.get(0)
        })?;
    assert_eq!(count, 200);
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    assert_eq!(integrity, "ok");
    Ok(())
}

#[test]
fn kill_during_write_reopens_consistent_database() -> TestResult {
    let root = tempfile::tempdir()?;
    let mut writer = child(root.path(), "crash", "1000000")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.path().join("crash.ready").exists() {
        if let Some(status) = writer.try_wait()? {
            return Err(format!("writer exited before crash point: {status}").into());
        }
        if Instant::now() >= deadline {
            writer.kill()?;
            let _status = writer.wait()?;
            return Err("writer did not reach crash point".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    writer.kill()?;
    let _status = writer.wait()?;
    let connection = open_connection(&root.path().join(DB_FILE_NAME), &RegistryConfig::default())?;
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    assert_eq!(integrity, "ok");
    let count: i64 =
        connection.query_row("SELECT count(*) FROM session_history_fts", [], |row| {
            row.get(0)
        })?;
    assert!(
        count >= 11,
        "observed committed prefix must survive process kill"
    );
    Ok(())
}

#[test]
#[ignore = "explicit one-million-message release workload"]
fn million_message_index_and_search_measurement() -> TestResult {
    let root = tempfile::tempdir()?;
    let registry = Registry::open(root.path())?;
    let start = Instant::now();
    for index in 0..1_000_000_u64 {
        registry.index_message(
            SessionId::from_u128(1),
            ChatRole::User,
            &format!("synthetic corpus record unique{index:07} compiler diagnostics"),
            Timestamp::UNIX_EPOCH,
        )?;
        if index % 100_000 == 99_999 {
            let mut times = Vec::new();
            for query in 0..100_u64 {
                let lookup = format!("unique{:07}", query.saturating_mul(997));
                let before = Instant::now();
                assert!(!registry.search_history(&lookup, 10)?.is_empty());
                times.push(before.elapsed());
            }
            times.sort_unstable();
            let p99 = times.get(98).ok_or("missing latency")?;
            let bytes = std::fs::read_dir(root.path())?.try_fold(
                0_u64,
                |total, entry| -> Result<u64, std::io::Error> {
                    Ok(total.saturating_add(entry?.metadata()?.len()))
                },
            )?;
            eprintln!(
                "synthetic registry messages={} elapsed={:?} search_p99={p99:?} storage_bytes={bytes}",
                index.saturating_add(1),
                start.elapsed()
            );
        }
    }
    Ok(())
}
