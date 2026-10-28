//! One real HTTP/tokenizer/frame composition run against a bounded synthetic loopback server.
//! Token IDs/count and verifier fingerprint are fixtures, not production model evidence.

use polaris_kernel::{
    Kernel, KernelFrameJournal, KernelHandle, KernelPromptAssembler, KernelToolRunner,
    LocalKernelConfig, PromptContext, PromptLimits,
};
use polaris_nebius_cloud::token_factory::{
    ApiKey, BaseUrl, ClientConfig, ReasoningReplay, TokenFactoryClient, VllmChatClient,
};
use polaris_runtime::context::EpistemicScratchpad;
use polaris_runtime::frames::{
    Active, AgentContextFrame, AgentRole, Cancellation, FramePromptBinding, FrameRunInput,
    FrameRunLimits, FrameRunStop, FrameScope, ModelSelection, ResourceQuota, ScratchpadRef,
    StepTool, StepVerifier, Verification,
};
use polaris_sdk::contracts::{Event, FrameStepOutcome};
use polaris_sdk::effort::Effort;
use polaris_sdk::ids::{Cid, FastFingerprint, FrameId, ModelId, Seq, SessionId, TokenCount};
use polaris_sdk::registry::ChatRequest;
use polaris_security::capability::{
    Authority, Clock, ResourceScope, Right, Rights, Scope, SystemClock, Token,
};
use secrecy::SecretString;
use serde_json::{Value, json};
use std::{
    error::Error,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const SYNTHETIC_MODEL: &str = "fixture/synthetic-loopback-model";
const SYNTHETIC_BEARER: &str = "fixture-synthetic-bearer";
const MAX_FIXTURE_REQUEST_BYTES: usize = 2_097_152;
const SYNTHETIC_TOKEN_COUNT: usize = 7;

struct KernelFixture {
    _root: tempfile::TempDir,
    _workspace: tempfile::TempDir,
    session: SessionId,
    kernel: KernelHandle,
    frame: AgentContextFrame<Active>,
    token: Token,
    runner: Arc<KernelToolRunner>,
    journal: KernelFrameJournal,
}

impl KernelFixture {
    async fn new() -> TestResult<Self> {
        let root = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let frame_id = FrameId::new_v7();
        let session = SessionId::new_v7();
        let kernel = Kernel::start(LocalKernelConfig::new(
            root.path(),
            workspace.path(),
            session,
            frame_id,
        ))
        .await?;
        let authority = Authority::new_session()?;
        let scope = Scope {
            resource: ResourceScope::Workspace,
            max_churn_lines: 0,
            ttl: Duration::from_secs(300),
            allow_unsafe: false,
            diff_blake3: None,
            rights: Rights::new(&[Right::Read, Right::ExecuteTool]),
        };
        let token = authority.issue_root(frame_id, &scope, &SystemClock)?;
        let expires_at_micros = SystemClock
            .now()
            .as_microsecond()
            .checked_add(300_000_000)
            .ok_or("fixture frame expiry overflow")?;
        let frame = AgentContextFrame::new_root(
            frame_id,
            AgentRole::Orchestrator,
            token.clone(),
            FrameScope {
                capability: scope,
                expires_at_micros,
            },
            ResourceQuota {
                cpu_millis: 10_000,
                memory_mb: 128,
                network: Default::default(),
                max_steps: 4,
                max_depth: 2,
                token_budget: 1_000,
            },
            ScratchpadRef::new(Cid::from_bytes([1; 32])),
        )
        .activate(
            ModelSelection {
                model: ModelId::new(SYNTHETIC_MODEL)?,
                maximum_effort: Effort::High,
            },
            Effort::High,
        )?;
        let runner = Arc::new(KernelToolRunner::new(
            kernel.clone(),
            frame.tool_binding(),
            authority.public_key(),
            CancellationToken::new(),
        )?);
        let journal = KernelFrameJournal::new(
            kernel.clone(),
            frame.tool_binding(),
            polaris_runtime::frames::FrameJournalLimits::default(),
        )?;
        Ok(Self {
            _root: root,
            _workspace: workspace,
            session,
            kernel,
            frame,
            token,
            runner,
            journal,
        })
    }

    fn prompt_context(&self) -> PromptContext {
        PromptContext {
            system_prompt: "Follow policy; contact safe@example.com for escalations.".to_owned(),
            repo_map_version: "http-fixture-v1".to_owned(),
            repo_map: "Repository owner: owner@example.com".to_owned(),
            active_scope: "src/; reviewer scope@example.com".to_owned(),
            scratchpad: EpistemicScratchpad::default(),
            tool_names: vec![
                "read_file".to_owned(),
                "polaris_expand".to_owned(),
                "polaris_recall".to_owned(),
            ],
        }
    }

    fn prompt_limits(&self) -> PromptLimits {
        PromptLimits {
            max_input_bytes: 65_536,
            max_messages: 32,
            max_tools: 16,
            max_count_requests: 4,
            count_timeout: Duration::from_secs(4),
            max_input_tokens: TokenCount::new(128),
            max_output_tokens: TokenCount::new(32),
            max_archive_bytes: 8_192,
        }
    }
}

#[derive(Debug)]
struct CapturedRequest {
    target: String,
    headers: String,
    body: String,
}

struct LoopbackFixture {
    endpoint: String,
    server: Option<JoinHandle<io::Result<Vec<CapturedRequest>>>>,
}

impl LoopbackFixture {
    fn start() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let server = thread::spawn(move || serve_two_requests(listener));
        Ok(Self {
            endpoint: format!("http://{address}/v1"),
            server: Some(server),
        })
    }

    fn finish(mut self) -> TestResult<Vec<CapturedRequest>> {
        let server = self
            .server
            .take()
            .ok_or("loopback server handle was absent")?;
        let requests = server
            .join()
            .map_err(|_| "loopback fixture thread panicked")??;
        Ok(requests)
    }
}

fn serve_two_requests(listener: TcpListener) -> io::Result<Vec<CapturedRequest>> {
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(8))
        .ok_or_else(|| io::Error::other("loopback fixture deadline overflow"))?;
    let mut requests = Vec::new();
    while requests.len() < 2 {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "loopback fixture accept deadline elapsed",
            ));
        }
        match listener.accept() {
            Ok((mut stream, _address)) => {
                // Accepted sockets can inherit nonblocking mode on Windows. The bounded
                // request parser uses blocking I/O; accept polling remains nonblocking.
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                stream.set_write_timeout(Some(Duration::from_secs(3)))?;
                let request = read_bounded_request(&mut stream)?;
                write_fixture_response(&mut stream, &request.target)?;
                requests.push(request);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
    Ok(requests)
}

fn read_bounded_request(stream: &mut TcpStream) -> io::Result<CapturedRequest> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    let mut complete_length = None;
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "loopback request ended before its declared body",
            ));
        }
        let received = buffer.get(..count).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid fixture read length")
        })?;
        let next_length = bytes
            .len()
            .checked_add(received.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "request length overflow"))?;
        if next_length > MAX_FIXTURE_REQUEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "loopback request exceeded fixture byte cap",
            ));
        }
        bytes.extend_from_slice(received);

        if complete_length.is_none()
            && let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let header_bytes = bytes.get(..header_end).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid fixture header boundary",
                )
            })?;
            let header_text = std::str::from_utf8(header_bytes).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid HTTP headers: {error}"),
                )
            })?;
            let body_length = header_text
                .lines()
                .skip(1)
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then_some(value.trim())
                })
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "content length is required")
                })?
                .parse::<usize>()
                .map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("invalid content length: {error}"),
                    )
                })?;
            if body_length > MAX_FIXTURE_REQUEST_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "loopback body exceeded fixture byte cap",
                ));
            }
            complete_length = header_end
                .checked_add(4)
                .and_then(|length| length.checked_add(body_length));
            if complete_length.is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTTP request length overflow",
                ));
            }
        }

        if complete_length.is_some_and(|length| bytes.len() >= length) {
            let expected_length = complete_length.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "missing complete request length",
                )
            })?;
            if bytes.len() != expected_length {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "fixture received bytes beyond one bounded HTTP request",
                ));
            }
            break;
        }
    }

    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "HTTP header terminator missing")
        })?;
    let header_bytes = bytes.get(..header_end).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "invalid fixture header length")
    })?;
    let headers = std::str::from_utf8(header_bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        .to_owned();
    let request_line = headers
        .lines()
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "HTTP request line missing"))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "HTTP method missing"))?;
    let target = request_parts
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "HTTP target missing"))?;
    if method != "POST" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "loopback fixture only accepts POST",
        ));
    }
    let body_start = header_end
        .checked_add(4)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "body start overflow"))?;
    let request_end = complete_length.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "complete request length missing",
        )
    })?;
    let body_bytes = bytes.get(body_start..request_end).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "invalid fixture body boundary")
    })?;
    let body = std::str::from_utf8(body_bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
        .to_owned();
    Ok(CapturedRequest {
        target: target.to_owned(),
        headers,
        body,
    })
}

fn write_fixture_response(stream: &mut TcpStream, target: &str) -> io::Result<()> {
    if target == "/tokenize" {
        // These IDs, count and capacity are synthetic response fixtures, not tokenizer evidence.
        let response = json!({
            "count": SYNTHETIC_TOKEN_COUNT,
            "max_model_len": 4096,
            "tokens": [1001, 1002, 1003, 1004, 1005, 1006, 1007]
        })
        .to_string();
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        );
        stream.write_all(headers.as_bytes())?;
        return stream.write_all(response.as_bytes());
    }
    if target == "/v1/chat/completions" {
        let response = format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices": [{"delta": {"content": "Synthetic fixture response."}, "finish_reason": null}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "stop"}]})
        );
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        );
        stream.write_all(headers.as_bytes())?;
        return stream.write_all(response.as_bytes());
    }
    let response = "{\"error\":\"unexpected synthetic fixture route\"}";
    let headers = format!(
        "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.len()
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(response.as_bytes())
}

fn client_config() -> ClientConfig {
    ClientConfig {
        request_timeout: Duration::from_secs(5),
        connect_timeout: Duration::from_secs(1),
        stream_idle_timeout: Duration::from_secs(2),
        max_sse_frame_bytes: 4096,
        max_events_per_chunk: 8,
        max_chat_stream_bytes: 16_384,
        max_text_bytes: 2048,
        max_tool_argument_bytes: 2048,
        max_tool_calls: 16,
        max_json_response_bytes: 8192,
        max_request_bytes: 65_536,
        max_429_retries: 0,
        max_5xx_retries: 0,
        retry_base_delay: Duration::from_millis(1),
        max_retry_delay: Duration::from_millis(1),
        reasoning_replay: ReasoningReplay::Disabled,
    }
}

struct SyntheticStepVerifier;

impl StepVerifier for SyntheticStepVerifier {
    type Error = polaris_runtime::frames::FrameError;

    async fn verify(
        &self,
        _frame: &polaris_runtime::frames::FrameView,
        _output: &StepTool,
    ) -> Result<Verification, Self::Error> {
        // This synthetic fingerprint is not independent verification evidence.
        Ok(Verification::Accepted {
            fingerprint: FastFingerprint::new(7),
            verified_diffs: Vec::new(),
        })
    }
}

fn require(condition: bool, message: &'static str) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn parsed_body(request: &CapturedRequest) -> TestResult<Value> {
    Ok(serde_json::from_str(&request.body)?)
}

async fn durable_transcript(
    kernel: &KernelHandle,
    session: SessionId,
    frame: FrameId,
) -> TestResult<(Cid, Value)> {
    let records = kernel.context().records(Seq::new(1), Seq::new(u64::MAX))?;
    let mut captured = None;
    for record in &records {
        if let Event::FrameStepRecorded {
            frame: event_frame,
            step,
            transcript,
            outcome,
        } = record.event()
        {
            require(
                record.session() == session,
                "durable frame event used another session",
            )?;
            require(
                *event_frame == frame,
                "durable frame event used another frame",
            )?;
            require(*step == 1, "durable frame event did not record step one")?;
            require(
                *outcome == FrameStepOutcome::Accepted,
                "synthetic accepted turn was not durably marked accepted",
            )?;
            if captured.replace(*transcript).is_some() {
                return Err("expected exactly one frame event".into());
            }
        }
    }
    let cid = captured.ok_or("durable frame event was missing")?;
    let expanded = kernel
        .invoke_tool("polaris_expand", json!({"reference": cid.to_hex()}))
        .await?;
    let text = expanded
        .value
        .get("text")
        .and_then(Value::as_str)
        .ok_or("durable transcript was absent from actual kernel CAS")?;
    require(
        polaris_kernel::storage::cas::content_id(text.as_bytes()) == cid,
        "durable transcript bytes did not match the event CID",
    )?;
    let payload = serde_json::from_str(text)?;
    Ok((cid, payload))
}

#[tokio::test]
async fn counted_prompt_matches_dispatched_http_and_durable_frame() -> TestResult {
    let http = LoopbackFixture::start()?;
    let mut fixture = KernelFixture::new().await?;
    let base = BaseUrl::parse(&http.endpoint)?;
    let api_key = ApiKey::new(SecretString::from(SYNTHETIC_BEARER))?;
    let client = Arc::new(VllmChatClient::new(TokenFactoryClient::new(
        base,
        api_key,
        client_config(),
    )?));
    let binding: FramePromptBinding = fixture.frame.prompt_binding()?;
    let assembler = KernelPromptAssembler::new(
        fixture.kernel.clone(),
        binding,
        fixture.token.clone(),
        Arc::clone(&fixture.runner),
        Arc::clone(&client),
        fixture.prompt_context(),
        fixture.prompt_limits(),
    )
    .await?;
    let goal = "Review contact person@example.com in src/lib.rs and summarize.";
    let cancellation = Cancellation::new();
    let outcome = tokio::time::timeout(
        Duration::from_secs(12),
        assembler.run_frame(
            FrameRunInput {
                frame: &mut fixture.frame,
                goal,
                cancellation: &cancellation,
                limits: FrameRunLimits {
                    maximum_attempts: 1,
                    maximum_retries: 0,
                    maximum_tool_rounds: 1,
                    max_goal_bytes: 1024,
                    max_prompt_bytes: 65_536,
                    max_output_bytes: 4096,
                    max_tool_calls: 8,
                    max_tool_argument_bytes: 2048,
                    max_tool_result_bytes: 8192,
                    max_stream_chunks: 8,
                },
            },
            &SyntheticStepVerifier,
            &fixture.journal,
        ),
    )
    .await;
    let requests = http.finish()?;
    let outcome = outcome??;
    require(
        outcome.stop == FrameRunStop::ModelTurnComplete
            && outcome.attempts == 1
            && outcome.tool_rounds == 0,
        "synthetic Stop stream did not complete exactly one ordinary frame turn",
    )?;

    require(
        requests.len() == 2,
        "fixture did not receive exactly two HTTP requests",
    )?;
    let tokenizer_request = requests.first().ok_or("tokenizer request missing")?;
    let dispatch_request = requests.get(1).ok_or("chat dispatch request missing")?;
    require(
        tokenizer_request.target == "/tokenize",
        "first HTTP request did not reach /tokenize",
    )?;
    require(
        dispatch_request.target == "/v1/chat/completions",
        "second HTTP request did not reach /v1/chat/completions",
    )?;
    require(
        tokenizer_request
            .headers
            .to_ascii_lowercase()
            .contains(&format!("authorization: bearer {SYNTHETIC_BEARER}")),
        "tokenizer HTTP request did not use the explicit synthetic fixture credential",
    )?;

    let counted = parsed_body(tokenizer_request)?;
    let dispatched = parsed_body(dispatch_request)?;
    for field in ["messages", "tools", "model"] {
        require(
            counted.get(field) == dispatched.get(field),
            "counted and dispatched HTTP requests differed in messages, tools, or model",
        )?;
    }
    require(
        counted.get("model") == Some(&json!(SYNTHETIC_MODEL)),
        "HTTP request did not use the model selected by the active frame",
    )?;
    require(
        counted.get("add_generation_prompt") == Some(&Value::Bool(true))
            && dispatched.get("stream") == Some(&Value::Bool(true)),
        "tokenize and chat HTTP routes did not use their expected provider modes",
    )?;
    let wire_bodies = format!("{}{}", tokenizer_request.body, dispatch_request.body);
    for secret in [
        "safe@example.com",
        "owner@example.com",
        "scope@example.com",
        "person@example.com",
    ] {
        require(
            !wire_bodies.contains(secret),
            "detected goal or context PII reached a tokenizer or chat HTTP request",
        )?;
    }
    require(
        tokenizer_request.body.contains("<PII1:") && dispatch_request.body.contains("<PII1:"),
        "masked request bodies did not retain reversible PII-vault markers",
    )?;

    let (_transcript_cid, transcript) =
        durable_transcript(&fixture.kernel, fixture.session, fixture.frame.view().id).await?;
    let request_value = transcript
        .get("request")
        .cloned()
        .ok_or("durable transcript omitted the exact chat request")?;
    let durable_request: ChatRequest = serde_json::from_value(request_value)?;
    require(
        durable_request.model.as_str() == SYNTHETIC_MODEL && durable_request.effort == Effort::High,
        "durable request did not retain the frame-selected model and high effort",
    )?;
    require(
        transcript.get("goal").and_then(Value::as_str) == Some(goal),
        "durable transcript did not retain the input goal for the audit record",
    )?;
    let response = transcript
        .get("assistant_response")
        .ok_or("durable transcript omitted the captured assistant response")?;
    require(
        response.get("text").and_then(Value::as_str) == Some("Synthetic fixture response."),
        "durable transcript omitted the synthetic response received over SSE",
    )?;
    let _shutdown = fixture.kernel.shutdown().await?;
    Ok(())
}

#[test]
fn loopback_fixture_reads_deliberately_fragmented_request_bodies() -> TestResult {
    let fixture = LoopbackFixture::start()?;
    let address = fixture
        .endpoint
        .strip_prefix("http://")
        .and_then(|url| url.strip_suffix("/v1"))
        .ok_or("fixture endpoint malformed")?;
    let network = (|| -> io::Result<()> {
        for target in ["/tokenize", "/v1/chat/completions"] {
            let mut stream = TcpStream::connect(address)?;
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            stream.set_write_timeout(Some(Duration::from_secs(3)))?;
            stream.write_all(format!("POST {target} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\nConnection: close\r\n\r\n").as_bytes())?;
            thread::sleep(Duration::from_millis(50));
            stream.write_all(b"{}")?;
            let mut response = Vec::new();
            stream.take(65_536).read_to_end(&mut response)?;
            if !response.starts_with(b"HTTP/1.1 200 OK") {
                return Err(io::Error::other(
                    "fixture did not return a successful HTTP response",
                ));
            }
        }
        Ok(())
    })();
    let captured = fixture.finish()?;
    network?;
    require(
        captured.len() == 2 && captured.iter().all(|request| request.body == "{}"),
        "fixture lost a fragmented request body",
    )?;
    Ok(())
}
