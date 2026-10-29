//! Desktop bridge with chat enabled: provider selection, a streamed reply and the work summary,
//! against a loopback OpenAI-compatible provider.

use std::error::Error;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use polaris_config::{Layer, LayerInput, TrustGrant, load};
use polaris_desktop::bridge;
use polaris_ui_gui::backend::{BackendEvent, BackendRequest, MessageKind};

type TestResult = Result<(), Box<dyn Error>>;

const WAIT: Duration = Duration::from_secs(30);
const REPLY: &str = "Hello from the desktop model";

fn until(
    events: &Receiver<BackendEvent>,
    done: impl Fn(&BackendEvent) -> bool,
) -> Result<Vec<BackendEvent>, Box<dyn Error>> {
    let deadline = Instant::now() + WAIT;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(event) => {
                let stop = done(&event);
                seen.push(event);
                if stop {
                    return Ok(seen);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err(format!("timed out; events so far: {seen:?}").into());
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(format!("bridge stopped; events so far: {seen:?}").into());
            }
        }
    }
}

/// Answers `/v1/models` and streams [`REPLY`] for every chat request until stopped.
fn fake_provider() -> Result<(u16, Arc<AtomicBool>, std::thread::JoinHandle<()>), Box<dyn Error>> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
        while !flag.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _blocking = stream.set_nonblocking(false);
                    let _timeout = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut request = Vec::new();
                    let mut buffer = [0_u8; 8192];
                    let head_end = loop {
                        match stream.read(&mut buffer) {
                            Ok(0) | Err(_) => break None,
                            Ok(read) => {
                                request.extend_from_slice(buffer.get(..read).unwrap_or_default());
                                if let Some(at) =
                                    request.windows(4).position(|window| window == b"\r\n\r\n")
                                {
                                    break Some(at + 4);
                                }
                            }
                        }
                    };
                    let Some(head_end) = head_end else { continue };
                    let head = String::from_utf8_lossy(request.get(..head_end).unwrap_or_default())
                        .to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    while request.len() < head_end + length {
                        match stream.read(&mut buffer) {
                            Ok(0) | Err(_) => break,
                            Ok(read) => {
                                request.extend_from_slice(buffer.get(..read).unwrap_or_default());
                            }
                        }
                    }
                    let response = if head.starts_with("get ") {
                        let body =
                            r#"{"object":"list","data":[{"id":"fake-model","object":"model"}]}"#;
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        )
                    } else {
                        format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: {{\"id\":\"c\",\"choices\":[{{\"index\":0,\"delta\":{{\"role\":\"assistant\",\"content\":\"{REPLY}\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"id\":\"c\",\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
                        )
                    };
                    let _written = stream.write_all(response.as_bytes());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });
    Ok((port, stop, thread))
}

#[test]
fn typed_message_is_recorded_then_answered_by_the_provider() -> TestResult {
    let (port, stop, server) = fake_provider()?;
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
    let (link, bridge) = bridge::spawn_with_chat(
        bridge::default_state_dir(&workspace),
        workspace,
        Some(bridge::ChatOptions {
            config,
            env_file: None,
        }),
    )?;
    let seen = until(&link.events, |e| {
        matches!(
            e,
            BackendEvent::ProviderReady { .. } | BackendEvent::ProviderUnavailable(_)
        )
    })?;
    assert_eq!(
        seen.last(),
        Some(&BackendEvent::ProviderReady {
            provider: "local".to_owned(),
            model: "fake-model".to_owned(),
        })
    );
    link.requests.send(BackendRequest::Open { resume: None })?;
    let _opened = until(&link.events, |e| matches!(e, BackendEvent::Opened { .. }))?;
    link.requests.send(BackendRequest::Submit {
        text: "say hello".to_owned(),
    })?;
    let seen = until(&link.events, |e| {
        matches!(
            e,
            BackendEvent::ReplyFinished { .. } | BackendEvent::ReplyFailed(_)
        )
    })?;
    let position = |wanted: &dyn Fn(&BackendEvent) -> bool| seen.iter().position(|e| wanted(e));
    let prompt = position(&|e| {
        matches!(e, BackendEvent::Message { kind: MessageKind::UserInput, text } if text == "say hello")
    })
    .ok_or(format!("prompt missing: {seen:?}"))?;
    let started = position(&|e| matches!(e, BackendEvent::ReplyStarted))
        .ok_or(format!("reply never started: {seen:?}"))?;
    let streamed = position(&|e| matches!(e, BackendEvent::ReplyText(text) if text == REPLY))
        .ok_or(format!("reply text missing: {seen:?}"))?;
    assert!(prompt < started && started < streamed, "ordered: {seen:?}");
    match seen.last() {
        Some(BackendEvent::ReplyFinished { summary, .. }) => {
            assert!(summary.starts_with("Worked for"), "{summary}");
        }
        other => return Err(format!("reply did not finish: {other:?}").into()),
    }
    drop(link.requests);
    let _closed = until(&link.events, |e| matches!(e, BackendEvent::Closed))?;
    bridge.join()?;
    stop.store(true, Ordering::Release);
    server.join().map_err(|_| "fake provider panicked")?;
    Ok(())
}
