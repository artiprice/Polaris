// Unit tests of crates/polaris-search/crates/providers/src/runtime.rs (`tests`); compiled into that module via #[path].
use super::ProviderRuntime;
use polaris_ratelimit::{Quota, ShedPolicy};
use std::{error::Error, num::NonZeroU32, time::Duration};

#[tokio::test]
async fn worker_guard_holds_concurrency_after_call_drop() -> Result<(), Box<dyn Error>> {
    let quota = Quota {
        rpm: NonZeroU32::new(60).ok_or("invalid test quota")?,
        tpm: NonZeroU32::new(60).ok_or("invalid test quota")?,
        burst_requests: 4,
        burst_tokens: 4,
    };
    let runtime = ProviderRuntime::new(quota, 1, ShedPolicy::default())?;
    let call = runtime.acquire(Duration::from_secs(1)).await?;
    let worker_guard = call.worker_guard();
    drop(call);
    let (started, started_rx) = tokio::sync::oneshot::channel();
    let worker = tokio::task::spawn_blocking(move || {
        let _worker_guard = worker_guard;
        let _ = started.send(());
        std::thread::sleep(Duration::from_millis(100));
    });
    started_rx.await?;
    if runtime.acquire(Duration::from_millis(10)).await.is_ok() {
        return Err("worker guard released its concurrency permit after call drop".into());
    }
    worker.await?;
    let next_call = runtime.acquire(Duration::from_secs(1)).await?;
    next_call.finish()?;
    Ok(())
}
