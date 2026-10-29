//! Wall-clock cost of `OfflineSession::open` (descriptor: specs/descriptors/ui_session_host.json).
//! This is a measurement, not a gate: it asserts only that every open succeeds. Run with
//! `cargo test -p polaris-ui-session --test open_latency -- --nocapture` (debug build).
use polaris_sdk::ids::SessionId;
use polaris_ui_session::OfflineSession;
use std::time::{Duration, Instant};

fn summary(label: &str, mut samples: Vec<Duration>) -> Result<(), Box<dyn std::error::Error>> {
    samples.sort();
    let (Some(min), Some(median), Some(max)) = (samples.first(), samples.get(2), samples.last())
    else {
        return Err("no samples".into());
    };
    println!(
        "OPEN_LATENCY_DEBUG {label} runs=5 min={min:?} median={median:?} max={max:?} all={samples:?}"
    );
    Ok(())
}

#[tokio::test]
async fn open_latency_is_measured_over_five_runs() -> Result<(), Box<dyn std::error::Error>> {
    let mut fresh: Vec<Duration> = Vec::new();
    let mut reopen: Vec<Duration> = Vec::new();
    for _ in 0..5 {
        let state = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let id = SessionId::new_v7();
        let started = Instant::now();
        let mut session = OfflineSession::open(state.path(), workspace.path(), id).await?;
        fresh.push(started.elapsed());
        session.close().await?;
        drop(session);
        let started = Instant::now();
        let mut again = OfflineSession::open(state.path(), workspace.path(), id).await?;
        reopen.push(started.elapsed());
        again.close().await?;
    }
    summary("new_session", fresh)?;
    summary("resume_session", reopen)
}
