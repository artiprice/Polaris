use polaris_kernel::{Kernel, LocalKernelConfig};
use polaris_sdk::{
    contracts::Event,
    ids::{FrameId, Seq, SessionId},
    registry::{EventSource, KernelApi},
};
use std::{path::Path, time::Duration};

fn config(root: &Path, workspace: &Path, id: u128) -> LocalKernelConfig {
    LocalKernelConfig::new(
        root,
        workspace,
        SessionId::from_u128(id),
        FrameId::from_u128(id + 1),
    )
}

#[tokio::test]
async fn failed_startup_releases_session_claim_and_writes_no_lifecycle_event()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let missing = root.path().join("missing-workspace");
    assert!(
        Kernel::start(config(root.path(), &missing, 31))
            .await
            .is_err()
    );

    let handle = Kernel::start(config(root.path(), workspace.path(), 31)).await?;
    let records = handle.context().records(Seq::ZERO, Seq::new(u64::MAX))?;
    assert_eq!(
        records.len(),
        2,
        "retry commits only configure and start after failed startup"
    );
    let _report = handle.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn canceled_shutdown_waiter_does_not_cancel_owner_cleanup()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let handle = Kernel::start(config(root.path(), workspace.path(), 41)).await?;
    let context = handle.context().clone();
    let shutdown_handle = handle.clone();
    let waiter = tokio::spawn(async move { shutdown_handle.shutdown().await });
    tokio::task::yield_now().await;
    waiter.abort();

    assert!(handle.apply_directive("$websearch(off)").await.is_err());
    tokio::time::timeout(Duration::from_secs(5), handle.shutdown()).await??;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if context
                .records(Seq::ZERO, Seq::new(u64::MAX))
                .is_ok_and(|records| {
                    records
                        .iter()
                        .any(|record| matches!(record.event(), Event::SessionPaused))
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    drop(handle);
    Ok(())
}

#[tokio::test]
async fn dropping_last_handle_drains_and_closes_event_stream()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let handle = Kernel::start(config(root.path(), workspace.path(), 46)).await?;
    let mut events = handle.subscribe(Seq::new(3)).await?;
    drop(handle);

    let pause = tokio::time::timeout(Duration::from_secs(5), events.next_event())
        .await?
        .ok_or("last-handle shutdown did not persist SessionPaused")?;
    assert!(matches!(pause.event(), Event::SessionPaused));
    assert!(
        tokio::time::timeout(Duration::from_secs(5), events.next_event())
            .await?
            .is_none()
    );
    drop(events);
    let reopened = tokio::time::timeout(
        Duration::from_secs(5),
        Kernel::start(config(root.path(), workspace.path(), 46)),
    )
    .await??;
    let _report = reopened.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn durable_replay_bridges_delayed_subscription_to_live_events_without_duplicates()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let handle = Kernel::start(config(root.path(), workspace.path(), 51)).await?;
    let mut accepted = Vec::new();
    for _ in 0..32 {
        accepted.push(handle.apply_directive("$websearch(off)").await?.accepted_at);
    }

    let mut events = handle.subscribe(Seq::new(1)).await?;
    let expected_count = 2 + accepted.len();
    for expected in 1..=expected_count as u64 {
        let record = tokio::time::timeout(Duration::from_secs(2), events.next_event())
            .await?
            .ok_or("durable replay ended before reaching the journal head")?;
        assert_eq!(
            record.seq().get(),
            expected,
            "replay must be contiguous and duplicate-free"
        );
    }

    let receipt = handle.apply_directive("$websearch(on)").await?;
    let live = tokio::time::timeout(Duration::from_secs(2), events.next_event())
        .await?
        .ok_or("live event was lost after replay caught up")?;
    assert_eq!(live.seq(), receipt.accepted_at);
    let _report = handle.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn missing_registry_projection_is_rebuilt_from_durable_lifecycle_records()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let id = SessionId::from_u128(61);
    let handle = Kernel::start(LocalKernelConfig::new(
        root.path(),
        workspace.path(),
        id,
        FrameId::from_u128(62),
    ))
    .await?;
    let _report = handle.shutdown().await?;
    drop(handle);

    std::fs::remove_file(root.path().join("registry.db"))?;
    let recovered = Kernel::start(LocalKernelConfig::new(
        root.path(),
        workspace.path(),
        id,
        FrameId::from_u128(62),
    ))
    .await?;
    assert!(
        recovered.context().session_meta(id)?.is_some(),
        "event replay restores the rebuildable registry row"
    );
    let _report = recovered.shutdown().await?;
    Ok(())
}
