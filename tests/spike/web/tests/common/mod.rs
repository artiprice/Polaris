//! A `polaris web` server on an ephemeral loopback port with no model provider, and an SSE reader.

use std::error::Error;
use std::time::Duration;

use futures::{Stream, StreamExt};
use polaris_config::{Layer, LayerInput, TrustGrant, load};
use polaris_ui_web::{WebOptions, WebServer};

pub type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

pub struct Running {
    pub base: String,
    pub token: String,
    _dir: tempfile::TempDir,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Result<(), polaris_ui_web::WebError>>,
}

/// Start the server with `provider.order = []`: replies are impossible, nothing leaves the machine.
pub async fn start() -> TestResult<Running> {
    let config = load(
        &[LayerInput::parse(
            Layer::Global,
            "[provider]\norder = []\n",
            None,
        )?],
        &TrustGrant::none(),
    )?
    .config;
    let dir = tempfile::tempdir()?;
    let workspace = dir.path().join("work");
    std::fs::create_dir_all(&workspace)?;
    let server = WebServer::bind(WebOptions {
        bind: "127.0.0.1:0".parse()?,
        state_dir: workspace.join(".polaris").join("state"),
        workspace,
        config,
        env_file: None,
    })
    .await?;
    let url = server.url().to_owned();
    let (base, token) = url.split_once("/#token=").ok_or("url carries the token")?;
    let (shutdown, signal) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(server.serve(async move {
        let _ = signal.await;
    }));
    Ok(Running {
        base: base.to_owned(),
        token: token.to_owned(),
        _dir: dir,
        shutdown: Some(shutdown),
        task,
    })
}

impl Running {
    pub fn events_url(&self) -> String {
        format!("{}/api/events?token={}", self.base, self.token)
    }

    pub async fn post(&self, client: &reqwest::Client, path: &str) -> TestResult<u16> {
        Ok(client
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(&serde_json::json!({}))
            .send()
            .await?
            .status()
            .as_u16())
    }

    pub async fn stop(mut self) -> TestResult {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        tokio::time::timeout(Duration::from_secs(20), &mut self.task).await???;
        Ok(())
    }
}

/// Server-Sent Events from one open stream, parsed as JSON payloads.
pub struct Events<S> {
    stream: S,
    buffer: String,
    pending: std::collections::VecDeque<serde_json::Value>,
}

impl<S, B> Events<S>
where
    S: Stream<Item = Result<B, reqwest::Error>> + Unpin,
    B: AsRef<[u8]>,
{
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buffer: String::new(),
            pending: std::collections::VecDeque::new(),
        }
    }

    /// The next event of type `kind`, skipping others, within `limit`.
    pub async fn until(&mut self, kind: &str, limit: Duration) -> TestResult<serde_json::Value> {
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            while let Some(event) = self.pending.pop_front() {
                if event["type"] == kind {
                    return Ok(event);
                }
            }
            let chunk = tokio::time::timeout_at(deadline, self.stream.next())
                .await
                .map_err(|_| format!("no {kind} event within {limit:?}"))?
                .ok_or("event stream ended")??;
            self.buffer
                .push_str(&String::from_utf8_lossy(chunk.as_ref()));
            while let Some(end) = self.buffer.find("\n\n") {
                let frame: String = self.buffer.drain(..end + 2).collect();
                for line in frame.lines() {
                    if let Some(data) = line.strip_prefix("data:") {
                        self.pending.push_back(serde_json::from_str(data.trim())?);
                    }
                }
            }
        }
    }
}

/// Resident memory of this process, in bytes (used by `connection_flood` only).
#[allow(dead_code)]
pub fn resident_bytes() -> Option<u64> {
    let pid = sysinfo::Pid::from_u32(std::process::id());
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    system.process(pid).map(sysinfo::Process::memory)
}
