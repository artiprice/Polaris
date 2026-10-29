//! One command reaches 64 open event streams; delivery latency is bounded at p99.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use common::{Events, TestResult, start};
use hdrhistogram::Histogram;

const CLIENTS: usize = 64;
const P99_BOUND: Duration = Duration::from_secs(2);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_command_reaches_every_open_stream() -> TestResult {
    let running = Arc::new(start().await?);
    let client = reqwest::Client::new();
    let ready = Arc::new(tokio::sync::Barrier::new(CLIENTS + 1));
    let sent_at = Arc::new(OnceLock::<Instant>::new());
    let mut readers = Vec::with_capacity(CLIENTS);
    for _ in 0..CLIENTS {
        let client = client.clone();
        let url = running.events_url();
        let ready = Arc::clone(&ready);
        let sent_at = Arc::clone(&sent_at);
        readers.push(tokio::spawn(async move {
            let response = client.get(url).send().await?;
            let mut events = Events::new(response.bytes_stream());
            // The provider state is replayed first: the stream is live once it arrives.
            let _provider = events
                .until("provider-unavailable", Duration::from_secs(20))
                .await?;
            ready.wait().await;
            let _opened = events.until("opened", Duration::from_secs(20)).await?;
            let start = sent_at.get().ok_or("command time not set")?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(start.elapsed())
        }));
    }
    ready.wait().await;
    let _ = sent_at.set(Instant::now());
    assert_eq!(running.post(&client, "/api/open").await?, 202);

    let mut latency = Histogram::<u64>::new(3)?;
    for reader in readers {
        let elapsed = reader.await??;
        latency.record(u64::try_from(elapsed.as_micros())?)?;
    }
    eprintln!(
        "SSE fan-out to {CLIENTS} streams: p50 {}µs, p99 {}µs, max {}µs",
        latency.value_at_quantile(0.5),
        latency.value_at_quantile(0.99),
        latency.max()
    );
    assert_eq!(latency.len(), u64::try_from(CLIENTS)?);
    assert!(
        latency.value_at_quantile(0.99) < u64::try_from(P99_BOUND.as_micros())?,
        "p99 {}µs exceeds {P99_BOUND:?}",
        latency.value_at_quantile(0.99)
    );
    Arc::into_inner(running)
        .ok_or("server handle still shared")?
        .stop()
        .await
}
