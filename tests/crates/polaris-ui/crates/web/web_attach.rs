//! The web UI attached to a real `polarisd` (P9): the page opens the daemon's session, typed
//! lines are recorded by the daemon and stream back as messages, and the session list is the
//! daemon's one session.

#[path = "../../../../support/attach_daemon.rs"]
mod attach_daemon;

use std::time::Duration;

use attach_daemon::{AnyError, Daemon};
use futures::StreamExt;
use polaris_config::{Layer, LayerInput, TrustGrant, load};
use polaris_ui_web::{WebOptions, WebServer};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_page_records_into_an_attached_daemon_session() -> Result<(), AnyError> {
    let daemon = Daemon::start()?;
    daemon.ready().await?;
    let config = load(
        &[LayerInput::parse(Layer::Global, "", None)?],
        &TrustGrant::none(),
    )?
    .config;
    let dir = tempfile::tempdir()?;
    let server = WebServer::bind_with(
        WebOptions {
            bind: "127.0.0.1:0".parse()?,
            state_dir: dir.path().join("state"),
            workspace: dir.path().to_path_buf(),
            config,
            env_file: None,
        },
        None,
        Some(daemon.target()?),
    )
    .await?;
    let url = server.url().to_owned();
    let (base, token) = url.split_once("/#token=").ok_or("url carries the token")?;
    let (shutdown, signal) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(server.serve(async move {
        let _received = signal.await;
    }));

    let client = reqwest::Client::new();
    let events = client
        .get(format!("{base}/api/events?token={token}"))
        .send()
        .await?;
    assert_eq!(events.status(), 200);
    let mut stream = events.bytes_stream();
    let mut buffer = String::new();
    let mut seen: Vec<serde_json::Value> = Vec::new();
    let (mut opened, mut submitted) = (false, false);
    let session = daemon.session.to_string();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while !seen
        .iter()
        .any(|event| event["type"] == "message" && event["text"] == "hello daemon")
    {
        let chunk = tokio::time::timeout_at(deadline, stream.next())
            .await
            .map_err(|_| format!("timed out; events: {seen:?}"))?
            .ok_or("event stream ended")??;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(end) = buffer.find("\n\n") {
            let frame: String = buffer.drain(..end + 2).collect();
            for line in frame.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    seen.push(serde_json::from_str(data.trim())?);
                }
            }
        }
        if !opened
            && seen.iter().any(|event| {
                event["type"] == "provider-unavailable"
                    && event["reason"]
                        .as_str()
                        .is_some_and(|reason| reason.contains("polarisd"))
            })
        {
            opened = true;
            let status = client
                .post(format!("{base}/api/open"))
                .bearer_auth(token)
                .json(&serde_json::json!({}))
                .send()
                .await?
                .status();
            assert_eq!(status, 202);
        }
        if !submitted
            && seen
                .iter()
                .any(|event| event["type"] == "opened" && event["id"] == session.as_str())
        {
            submitted = true;
            let status = client
                .post(format!("{base}/api/submit"))
                .bearer_auth(token)
                .json(&serde_json::json!({"text": "hello daemon"}))
                .send()
                .await?
                .status();
            assert_eq!(status, 202);
        }
    }
    let message = seen
        .iter()
        .find(|event| event["type"] == "message" && event["text"] == "hello daemon")
        .ok_or("message")?;
    assert_eq!(message["kind"], "user");

    let rows: serde_json::Value = client
        .get(format!("{base}/api/sessions"))
        .bearer_auth(token)
        .send()
        .await?
        .json()
        .await?;
    let rows = rows.as_array().ok_or("session rows")?;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(
        rows.first().map(|row| &row["id"]),
        Some(&serde_json::json!(session))
    );

    let _sent = shutdown.send(());
    drop(stream);
    tokio::time::timeout(Duration::from_secs(20), serving).await???;
    daemon.stop().await
}
