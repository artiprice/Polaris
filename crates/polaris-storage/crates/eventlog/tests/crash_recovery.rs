//! Subprocess termination checks a real durable redb prefix.
use polaris_sdk::{
    contracts::{Event, EventRecord, Timestamp},
    ids::{Seq, SessionId},
};
use polaris_storage_eventlog::{EventLog, EventLogConfig};
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};
type TestResult = Result<(), Box<dyn std::error::Error>>;
#[test]
fn journal_writer_child() -> TestResult {
    let Some(root) = std::env::var_os("EVENTLOG_TEST_CRASH_ROOT") else {
        return Ok(());
    };
    let root = std::path::Path::new(&root);
    let log = EventLog::open(EventLogConfig::new(root.join("events.redb")))?;
    let record = EventRecord::new(
        Seq::ZERO,
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(1),
        Event::SessionClosed,
    );
    for index in 0..100_000 {
        log.append(&record)?;
        if index == 10 {
            std::fs::write(root.join("committed.ready"), b"11 durable records")?;
        }
    }
    Ok(())
}
#[test]
fn process_kill_keeps_gap_free_committed_prefix() -> TestResult {
    let root = tempfile::tempdir()?;
    let mut child = Command::new(std::env::current_exe()?)
        .args(["--exact", "journal_writer_child", "--nocapture"])
        .env("EVENTLOG_TEST_CRASH_ROOT", root.path())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.path().join("committed.ready").exists() {
        if let Some(status) = child.try_wait()? {
            return Err(format!("writer exited before crash point: {status}").into());
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let _status = child.wait()?;
            return Err("journal writer did not reach crash point".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    child.kill()?;
    let _status = child.wait()?;
    let log = EventLog::open(EventLogConfig::new(root.path().join("events.redb")))?;
    let head = log.head()?;
    assert!(head.get() >= 11);
    let mut count = 0_u64;
    for (expected, record) in (1_u64..).zip(log.replay_iter(head)?) {
        let record = record?;
        assert_eq!(record.seq().get(), expected);
        count = count.saturating_add(1);
    }
    assert_eq!(count, head.get());
    Ok(())
}
