//! Local composition contracts, using real durable stores and cryptographic tokens.
use polaris_kernel::{Kernel, LocalKernelConfig};
use polaris_sdk::{
    contracts::{Command, CommandBody, TokenBytes},
    ids::{CommandId, FrameId, Seq, SessionId},
    registry::{EventSource, KernelApi},
};

#[tokio::test]
async fn invalid_token_appends_no_event() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let handle = Kernel::start(LocalKernelConfig::new(
        root.path(),
        workspace.path(),
        SessionId::from_u128(1),
        FrameId::from_u128(2),
    ))
    .await?;
    let before = handle
        .context()
        .records(Seq::ZERO, Seq::new(u64::MAX))?
        .len();
    let cid = handle.store_text("$websearch(off)").await?;
    let command = Command::new(
        CommandId::from_u128(3),
        handle.frame(),
        TokenBytes::from(vec![1]),
        CommandBody::ApplyDirective { directive: cid },
    );
    assert!(
        handle.submit(command).await.is_err(),
        "invalid signatures must fail closed"
    );
    assert_eq!(
        before,
        handle
            .context()
            .records(Seq::ZERO, Seq::new(u64::MAX))?
            .len(),
        "rejection writes no lifecycle event"
    );
    let _report = handle.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn directive_receipt_matches_committed_event() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let handle = Kernel::start(LocalKernelConfig::new(
        root.path(),
        workspace.path(),
        SessionId::from_u128(1),
        FrameId::from_u128(2),
    ))
    .await?;
    let receipt = handle.apply_directive("$websearch(off)").await?;
    let mut events = handle.subscribe(receipt.accepted_at).await?;
    let event = events
        .next_event()
        .await
        .ok_or("committed directive not replayed")?;
    assert_eq!(
        event.seq(),
        receipt.accepted_at,
        "receipt identifies the committed event"
    );
    let _report = handle.shutdown().await?;
    assert!(
        handle.apply_directive("$websearch(on)").await.is_err(),
        "shutdown closes admission"
    );
    Ok(())
}
