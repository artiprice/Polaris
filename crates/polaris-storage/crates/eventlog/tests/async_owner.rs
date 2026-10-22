//! Real async journal ownership and cancellation contracts.
use polaris_sdk::{
    contracts::{Event, EventRecord, Timestamp},
    ids::{Seq, SessionId},
};
use polaris_storage_eventlog::{AsyncEventLog, EventLog, EventLogConfig};
use std::sync::Arc;

#[tokio::test]
async fn async_owner_assigns_gap_free_sequences() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let log = Arc::new(EventLog::open(EventLogConfig::new(
        root.path().join("events.redb"),
    ))?);
    let owner = AsyncEventLog::new(Arc::clone(&log), 32)?;
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let owner = owner.clone();
        tasks.push(tokio::spawn(async move {
            owner
                .append(EventRecord::new(
                    Seq::ZERO,
                    Timestamp::UNIX_EPOCH,
                    SessionId::from_u128(1),
                    Event::SessionClosed,
                ))
                .await
        }));
    }
    let mut sequences = Vec::new();
    for task in tasks {
        sequences.push(task.await??.get());
    }
    sequences.sort_unstable();
    assert_eq!(sequences, (1..=32).collect::<Vec<_>>());
    owner.shutdown().await?;
    owner.shutdown().await?;
    assert_eq!(log.head()?, Seq::new(32));
    Ok(())
}

#[tokio::test]
async fn cancellation_does_not_cancel_shutdown_owner() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let log = Arc::new(EventLog::open(EventLogConfig::new(
        root.path().join("events.redb"),
    ))?);
    let owner = AsyncEventLog::new(Arc::clone(&log), 1)?;
    let mut shutdown = Box::pin(owner.shutdown());
    std::future::poll_fn(|context| {
        let _first_poll = shutdown.as_mut().poll(context);
        std::task::Poll::Ready(())
    })
    .await;
    drop(shutdown);
    tokio::time::timeout(std::time::Duration::from_secs(2), owner.shutdown()).await??;
    assert_eq!(log.head()?, Seq::ZERO);
    Ok(())
}

#[tokio::test]
async fn shutdown_drains_accepted_records() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let log = Arc::new(EventLog::open(EventLogConfig::new(
        root.path().join("events.redb"),
    ))?);
    let owner = AsyncEventLog::new(Arc::clone(&log), 8)?;
    owner
        .append(EventRecord::new(
            Seq::ZERO,
            Timestamp::UNIX_EPOCH,
            SessionId::from_u128(1),
            Event::SessionClosed,
        ))
        .await?;
    owner.shutdown().await?;
    assert!(
        owner
            .append(EventRecord::new(
                Seq::ZERO,
                Timestamp::UNIX_EPOCH,
                SessionId::from_u128(1),
                Event::SessionClosed
            ))
            .await
            .is_err()
    );
    assert_eq!(log.head()?, Seq::new(1));
    Ok(())
}
