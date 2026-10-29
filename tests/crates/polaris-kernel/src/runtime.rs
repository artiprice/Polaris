// Unit tests of crates/polaris-kernel/src/runtime.rs (`inspection_owner_tests`); compiled into that module via #[path].
use super::{Kernel, LocalKernelConfig};
use polaris_sdk::ids::{FrameId, SessionId};
use std::{io, sync::mpsc, time::Duration};

#[tokio::test]
async fn inspection_slots_bound_admission_and_shutdown_releases_waiters()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let kernel = Kernel::start(LocalKernelConfig::new(
        root.path(),
        workspace.path(),
        SessionId::new_v7(),
        FrameId::new_v7(),
    ))
    .await?;
    let mut owners = Vec::new();
    for _ in 0..super::MAX_INSPECTION_OWNERS {
        owners.push(kernel.inspection_owner().await?);
    }
    assert_eq!(kernel.shared.inspection_admission.available_permits(), 0);
    let admission_kernel = kernel.clone();
    let mut admission = tokio::spawn(async move { admission_kernel.inspection_owner().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut admission)
            .await
            .is_err(),
        "a sixty-fifth owner must wait"
    );
    let shutdown_kernel = kernel.clone();
    let shutdown = tokio::spawn(async move { shutdown_kernel.shutdown().await });
    let refused = tokio::time::timeout(Duration::from_secs(5), &mut admission).await??;
    assert!(refused.is_err(), "shutdown closes blocked admission");
    drop(owners);
    let _report = tokio::time::timeout(Duration::from_secs(5), shutdown).await???;
    Ok(())
}

#[tokio::test]
async fn shutdown_drains_a_blocking_inspection_owner() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let kernel = Kernel::start(LocalKernelConfig::new(
        root.path(),
        workspace.path(),
        SessionId::new_v7(),
        FrameId::new_v7(),
    ))
    .await?;
    let owner = kernel
        .inspection_owner()
        .await
        .map_err(|error| io::Error::other(format!("inspection admission failed: {error}")))?;
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let inspection = tokio::task::spawn_blocking(move || {
        started_tx
            .send(())
            .map_err(|error| io::Error::other(error.to_string()))?;
        release_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|error| io::Error::other(error.to_string()))?;
        drop(owner);
        Ok::<(), io::Error>(())
    });
    started_rx.recv_timeout(Duration::from_secs(5))?;

    let shutdown_kernel = kernel.clone();
    let mut shutdown = tokio::spawn(async move { shutdown_kernel.shutdown().await });
    let completed_early = tokio::time::timeout(Duration::from_millis(25), &mut shutdown)
        .await
        .is_ok();
    release_tx
        .send(())
        .map_err(|error| io::Error::other(error.to_string()))?;
    inspection.await??;
    if completed_early {
        return Err(io::Error::other("shutdown did not wait for inspection owner").into());
    }
    let _report = shutdown.await??;
    Ok(())
}
