//! The web UI's HTTP surface: access control, the embedded app, and a full prompt → streamed
//! reply round trip against a loopback OpenAI-compatible provider.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::error::Error;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt;
use polaris_config::{Layer, LayerInput, TrustGrant, load};
use polaris_ui_web::{WebOptions, WebServer};

type TestResult = Result<(), Box<dyn Error>>;
const REPLY: &str = "Hello from the web model";

/// A streamed completion that says `text`.
fn text_reply(text: &str) -> String {
    let delta = serde_json::json!({"id":"c","choices":[{"index":0,"delta":{"role":"assistant","content":text},"finish_reason":null}]});
    let done =
        serde_json::json!({"id":"c","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
    format!("data: {delta}\n\ndata: {done}\n\ndata: [DONE]\n\n")
}

/// A streamed completion that calls `name` with `arguments`.
fn tool_reply(name: &str, arguments: &serde_json::Value) -> String {
    let call = serde_json::json!({"id":"c","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":null}]});
    let done = serde_json::json!({"id":"c","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]});
    format!("data: {call}\n\ndata: {done}\n\ndata: [DONE]\n\n")
}

fn fake_provider() -> Result<(u16, Arc<AtomicBool>), Box<dyn Error>> {
    scripted_provider(vec![text_reply(REPLY)])
}

/// Completions answer in `replies` order; the last one repeats.
fn scripted_provider(replies: Vec<String>) -> Result<(u16, Arc<AtomicBool>), Box<dyn Error>> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        let mut served = 0_usize;
        while !flag.load(Ordering::Acquire) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            };
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut request = Vec::new();
            let mut buffer = [0_u8; 8192];
            let head_end = loop {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break None,
                    Ok(read) => {
                        request.extend_from_slice(&buffer[..read]);
                        if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                            break Some(at + 4);
                        }
                    }
                }
            };
            let Some(head_end) = head_end else { continue };
            let head = String::from_utf8_lossy(&request[..head_end]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while request.len() < head_end + length {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => request.extend_from_slice(&buffer[..read]),
                }
            }
            let response = if head.starts_with("get ") {
                let body = r#"{"object":"list","data":[{"id":"fake-model","object":"model"}]}"#;
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                let body = replies
                    .get(served.min(replies.len().saturating_sub(1)))
                    .cloned()
                    .unwrap_or_default();
                served += 1;
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}"
                )
            };
            let _ = stream.write_all(response.as_bytes());
        }
    });
    Ok((port, stop))
}

struct Running {
    base: String,
    token: String,
    workspace: std::path::PathBuf,
    _dir: tempfile::TempDir,
    stop: Arc<AtomicBool>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Result<(), polaris_ui_web::WebError>>,
}

async fn start() -> Result<Running, Box<dyn Error>> {
    start_with(fake_provider()?).await
}

async fn start_with((port, stop): (u16, Arc<AtomicBool>)) -> Result<Running, Box<dyn Error>> {
    let settings = format!(
        "[provider]\norder = [\"local\"]\n[local]\nbase_url = \"http://127.0.0.1:{port}/v1/\"\nmodel = \"fake-model\"\ncontext_window = 131072\n"
    );
    let config = load(
        &[LayerInput::parse(Layer::Global, &settings, None)?],
        &TrustGrant::none(),
    )?
    .config;
    let dir = tempfile::tempdir()?;
    let workspace = dir.path().join("work");
    std::fs::create_dir_all(&workspace)?;
    let server = WebServer::bind(WebOptions {
        bind: "127.0.0.1:0".parse()?,
        state_dir: workspace.join(".polaris").join("state"),
        workspace: workspace.clone(),
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
        workspace,
        _dir: dir,
        stop,
        shutdown: Some(shutdown),
        task,
    })
}

impl Running {
    async fn stop(mut self) -> TestResult {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        tokio::time::timeout(Duration::from_secs(20), &mut self.task).await???;
        self.stop.store(true, Ordering::Release);
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn api_requires_the_token_and_a_bound_host() -> TestResult {
    let running = start().await?;
    let client = reqwest::Client::new();
    let page = client.get(format!("{}/", running.base)).send().await?;
    assert_eq!(page.status(), 200);
    let csp = page
        .headers()
        .get("content-security-policy")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        csp.contains("script-src 'self'") && csp.contains("frame-ancestors 'none'"),
        "{csp}"
    );
    assert!(
        page.text()
            .await?
            .contains("What’s on your mind for Polaris?")
    );
    let anonymous = client
        .get(format!("{}/api/sessions", running.base))
        .send()
        .await?;
    assert_eq!(anonymous.status(), 401);
    let wrong = client
        .get(format!("{}/api/sessions", running.base))
        .bearer_auth("0".repeat(48))
        .send()
        .await?;
    assert_eq!(wrong.status(), 401);
    let rebinding = client
        .get(format!("{}/api/sessions", running.base))
        .bearer_auth(&running.token)
        .header("host", "attacker.example:80")
        .send()
        .await?;
    assert_eq!(
        rebinding.status(),
        421,
        "a foreign Host is refused (DNS rebinding)"
    );
    let listed = client
        .get(format!("{}/api/sessions", running.base))
        .bearer_auth(&running.token)
        .send()
        .await?;
    assert_eq!(listed.status(), 200);
    assert_eq!(listed.text().await?, "[]");
    running.stop().await
}

#[tokio::test(flavor = "multi_thread")]
async fn prompt_streams_a_reply_over_server_sent_events() -> TestResult {
    let running = start().await?;
    let client = reqwest::Client::new();
    let events = client
        .get(format!(
            "{}/api/events?token={}",
            running.base, running.token
        ))
        .send()
        .await?;
    assert_eq!(events.status(), 200);
    let mut stream = events.bytes_stream();
    let mut buffer = String::new();
    let mut seen: Vec<serde_json::Value> = Vec::new();
    let mut opened = false;
    let mut submitted = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    while !seen.iter().any(|event| event["type"] == "reply-finished") {
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
        if !opened && seen.iter().any(|event| event["type"] == "provider") {
            opened = true;
            let status = client
                .post(format!("{}/api/open", running.base))
                .bearer_auth(&running.token)
                .json(&serde_json::json!({}))
                .send()
                .await?
                .status();
            assert_eq!(status, 202);
        }
        if !submitted && seen.iter().any(|event| event["type"] == "opened") {
            submitted = true;
            let status = client
                .post(format!("{}/api/submit", running.base))
                .bearer_auth(&running.token)
                .json(&serde_json::json!({"text": "say hello"}))
                .send()
                .await?
                .status();
            assert_eq!(status, 202);
        }
    }
    let kinds: Vec<&str> = seen
        .iter()
        .filter_map(|event| event["type"].as_str())
        .collect();
    let position = |kind: &str| kinds.iter().position(|seen| *seen == kind);
    let message = position("message").ok_or(format!("prompt missing: {kinds:?}"))?;
    let started = position("reply-started").ok_or(format!("no reply: {kinds:?}"))?;
    let text = seen
        .iter()
        .position(|event| event["type"] == "reply-text" && event["text"] == REPLY)
        .ok_or(format!("reply text missing: {seen:?}"))?;
    assert!(message < started && started < text, "{kinds:?}");
    let finished = seen
        .iter()
        .find(|event| event["type"] == "reply-finished")
        .ok_or("finished")?;
    assert!(
        finished["summary"]
            .as_str()
            .unwrap_or_default()
            .starts_with("Worked for")
    );
    // The event stream is still open: stopping must end it instead of waiting forever.
    running.stop().await?;
    drop(stream);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_proposed_change_waits_for_review_and_is_written_after_apply() -> TestResult {
    let running = start_with(scripted_provider(vec![
        tool_reply(
            "write_file",
            &serde_json::json!({"path":"notes.md","contents":"# Notes\n","expected_base":null}),
        ),
        text_reply("Created notes.md."),
    ])?)
    .await?;
    let client = reqwest::Client::new();
    let events = client
        .get(format!(
            "{}/api/events?token={}",
            running.base, running.token
        ))
        .send()
        .await?;
    let mut stream = events.bytes_stream();
    let mut buffer = String::new();
    let mut seen: Vec<serde_json::Value> = Vec::new();
    let (mut opened, mut submitted, mut reviewed) = (false, false, false);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    while !seen.iter().any(|event| event["type"] == "reply-finished") {
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
        if !opened && seen.iter().any(|event| event["type"] == "provider") {
            opened = true;
            client
                .post(format!("{}/api/open", running.base))
                .bearer_auth(&running.token)
                .json(&serde_json::json!({}))
                .send()
                .await?;
        }
        if !submitted && seen.iter().any(|event| event["type"] == "opened") {
            submitted = true;
            client
                .post(format!("{}/api/submit", running.base))
                .bearer_auth(&running.token)
                .json(&serde_json::json!({"text": "create the notes"}))
                .send()
                .await?;
        }
        if !reviewed && let Some(review) = seen.iter().find(|event| event["type"] == "review") {
            reviewed = true;
            let prompt = review["prompt"].as_str().unwrap_or_default();
            assert!(
                prompt.starts_with("Create `notes.md`?") && prompt.contains("+# Notes"),
                "{prompt}"
            );
            assert!(!running.workspace.join("notes.md").exists());
            let status = client
                .post(format!("{}/api/review", running.base))
                .bearer_auth(&running.token)
                .json(&serde_json::json!({"request_id": review["request_id"], "apply": true}))
                .send()
                .await?
                .status();
            assert_eq!(status, 202);
        }
    }
    let kinds: Vec<&str> = seen
        .iter()
        .filter_map(|event| event["type"].as_str())
        .collect();
    let position = |kind: &str| kinds.iter().position(|seen| *seen == kind);
    let review = position("review").ok_or(format!("no review: {kinds:?}"))?;
    let closed = position("review-closed").ok_or(format!("not closed: {kinds:?}"))?;
    let applied = seen
        .iter()
        .position(|event| event["type"] == "applied" && event["path"] == "notes.md")
        .ok_or(format!("not applied: {kinds:?}"))?;
    assert!(review < closed && review < applied, "{kinds:?}");
    assert_eq!(
        std::fs::read_to_string(running.workspace.join("notes.md"))?,
        "# Notes\n"
    );
    let finished = seen
        .iter()
        .find(|event| event["type"] == "reply-finished")
        .ok_or("finished")?;
    assert!(
        finished["summary"]
            .as_str()
            .unwrap_or_default()
            .ends_with("1 file changed"),
        "{finished}"
    );
    running.stop().await?;
    drop(stream);
    Ok(())
}
