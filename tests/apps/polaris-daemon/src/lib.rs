// Unit tests of apps/polaris-daemon/src/lib.rs (`tests`); compiled into that module via #[path].
use super::{DaemonReadiness, Readiness, close_readiness_when_signaled};

#[tokio::test]
async fn readiness_closes_when_shutdown_is_requested() -> Result<(), &'static str> {
    let readiness = DaemonReadiness::new();
    readiness.set(true);
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let shutdown = close_readiness_when_signaled(
        async move {
            let _received = stop_rx.await;
        },
        readiness.clone(),
    );
    let serving = tokio::spawn(shutdown);
    if !readiness.is_ready() {
        return Err("readiness must remain true while the daemon serves");
    }
    stop_tx
        .send(())
        .map_err(|_| "shutdown listener disappeared")?;
    serving
        .await
        .map_err(|_| "readiness shutdown task failed")?;
    if readiness.is_ready() {
        return Err("readiness must close before transport draining");
    }
    Ok(())
}
