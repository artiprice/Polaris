//! Explicit real-redb load and append durability measurements on synthetic fixed-shape events.
use polaris_sdk::{
    contracts::{Event, EventRecord, Timestamp},
    ids::{Seq, SessionId},
};
use polaris_storage_eventlog::{Durability, EventLog, EventLogConfig};
use std::time::Instant;
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn event() -> EventRecord {
    EventRecord::new(
        Seq::ZERO,
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(1),
        Event::SessionClosed,
    )
}
#[test]
#[ignore = "explicit one-million-event release workload"]
fn million_events_and_replay() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("events.redb");
    let mut config = EventLogConfig::new(&path);
    config.durability = Durability::Eventual;
    let log = EventLog::open(config)?;
    let records = vec![event(); 1000];
    let overall = Instant::now();
    for batch in 1..=1000_u64 {
        log.append_batch(&records)?;
        if batch % 100 == 0 {
            let count = batch.saturating_mul(1000);
            let start = Instant::now();
            let replayed = log.replay_iter(Seq::new(count / 2))?.try_fold(
                0_u64,
                |total, record| -> Result<u64, polaris_storage_eventlog::EventLogError> {
                    let _record = record?;
                    Ok(total.saturating_add(1))
                },
            )?;
            assert_eq!(replayed, count / 2);
            eprintln!(
                "synthetic eventlog eventual records={count} append_elapsed={:?} half_replay={:?} file_bytes={}",
                overall.elapsed(),
                start.elapsed(),
                std::fs::metadata(&path)?.len()
            );
        }
    }
    let start = Instant::now();
    assert_eq!(
        log.replay_iter(Seq::new(100_000))?
            .collect::<Result<Vec<_>, _>>()?
            .len(),
        100_000
    );
    eprintln!("synthetic eventlog replay100k={:?}", start.elapsed());
    Ok(())
}
#[test]
#[ignore = "explicit append latency measurement with actual configured durability"]
fn append_latency_for_each_durability() -> TestResult {
    for durability in [Durability::Eventual, Durability::Immediate] {
        let root = tempfile::tempdir()?;
        let mut config = EventLogConfig::new(root.path().join("events.redb"));
        config.durability = durability;
        let log = EventLog::open(config)?;
        let mut latency = Vec::with_capacity(1000);
        for _ in 0..1000 {
            let start = Instant::now();
            log.append(&event())?;
            latency.push(start.elapsed());
        }
        latency.sort_unstable();
        eprintln!(
            "synthetic eventlog durability={durability:?} append_p50={:?} append_p99={:?}",
            latency.get(500).ok_or("p50")?,
            latency.get(990).ok_or("p99")?
        );
    }
    Ok(())
}
