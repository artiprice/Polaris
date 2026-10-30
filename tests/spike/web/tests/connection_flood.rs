//! Hundreds of concurrent page loads and short-lived event streams: the server keeps answering,
//! a fresh stream still gets events, and resident memory stays bounded.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::{Events, TestResult, resident_bytes, start};

const PAGES: usize = 256;
const STREAMS: usize = 128;
const MAX_RSS_GROWTH: u64 = 256 * 1024 * 1024;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connection_flood_leaves_the_server_responsive() -> TestResult {
    let running = start().await?;
    let client = reqwest::Client::new();
    let before = resident_bytes();

    let pages = (0..PAGES).map(|_| {
        let client = client.clone();
        let url = format!("{}/", running.base);
        tokio::spawn(async move {
            let response = client.get(url).send().await?;
            let status = response.status().as_u16();
            let body = response.text().await?;
            Ok::<_, reqwest::Error>((status, body.len()))
        })
    });
    for page in futures::future::join_all(pages).await {
        let (status, bytes) = page??;
        assert_eq!(status, 200);
        assert!(bytes > 1000, "the embedded page is served whole");
    }

    // Streams that connect, read their first event and drop at once.
    let streams = (0..STREAMS).map(|_| {
        let client = client.clone();
        let url = running.events_url();
        tokio::spawn(async move {
            let response = client.get(url).send().await?;
            let mut events = Events::new(response.bytes_stream());
            events
                .until("provider-unavailable", Duration::from_secs(20))
                .await
                .map(|_| ())
        })
    });
    for stream in futures::future::join_all(streams).await {
        stream??;
    }

    let fresh = client.get(running.events_url()).send().await?;
    let mut events = Events::new(fresh.bytes_stream());
    let _provider = events
        .until("provider-unavailable", Duration::from_secs(20))
        .await?;
    assert_eq!(running.post(&client, "/api/open").await?, 202);
    let opened = events.until("opened", Duration::from_secs(20)).await?;
    assert!(opened["id"].as_str().is_some_and(|id| !id.is_empty()));

    if let (Some(before), Some(after)) = (before, resident_bytes()) {
        let growth = after.saturating_sub(before);
        eprintln!("resident memory grew by {} KiB", growth / 1024);
        assert!(
            growth < MAX_RSS_GROWTH,
            "resident memory grew by {growth} bytes"
        );
    }
    drop(events);
    running.stop().await
}
