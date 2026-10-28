//! Fixture-first contract for the pinned llama.cpp exact chat tokenizer.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    thread,
    time::Duration,
};

use polaris_nebius_cloud_token_factory::{
    BaseUrl, ClientConfig, LlamaCppChatClient, LlamaCppExpectedProfile, ReasoningReplay,
};
use polaris_sdk::{
    effort::Effort,
    ids::{ModelId, TokenCount},
    registry::{
        ChatChunk, ChatChunkSource, ChatMessage, ChatModel, ChatRequest, ChatRole, Effects,
        PromptTokenizer, ToolSpec,
    },
};
use serde_json::{Value, json};

struct Reply {
    status: u16,
    content_type: &'static str,
    body: String,
    headers: Vec<(String, String)>,
}

type FixtureServer = thread::JoinHandle<std::io::Result<Vec<Vec<u8>>>>;

struct CapturedExchange {
    request: Vec<u8>,
    response: Vec<u8>,
}

type CaptureServer = thread::JoinHandle<std::io::Result<Vec<CapturedExchange>>>;

fn fixture(replies: Vec<Reply>) -> std::io::Result<(String, FixtureServer)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = thread::spawn(move || {
        let mut requests = Vec::with_capacity(replies.len());
        for reply in replies {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            requests.push(read_request(&mut stream)?);
            let reason = if reply.status == 200 {
                "OK"
            } else {
                "Bad Request"
            };
            write!(
                stream,
                "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: {}\r\nConnection: close\r\n",
                reply.status,
                reason,
                reply.body.len(),
                reply.content_type
            )?;
            for (name, value) in reply.headers {
                write!(stream, "{name}: {value}\r\n")?;
            }
            write!(stream, "\r\n{}", reply.body)?;
            stream.flush()?;
        }
        Ok(requests)
    });
    Ok((format!("http://{address}/v1"), server))
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 2048];
    let mut target_length = None;
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        request.extend_from_slice(buffer.get(..count).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid fixture read length",
            )
        })?);
        if target_length.is_none()
            && let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let headers = String::from_utf8_lossy(request.get(..header_end).ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid fixture headers")
            })?);
            let content_length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then_some(value.trim())
            });
            let body_len = match content_length {
                Some(value) => value
                    .parse::<usize>()
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?,
                None => 0,
            };
            target_length = header_end
                .checked_add(4)
                .and_then(|length| length.checked_add(body_len));
        }
        if target_length.is_some_and(|length| request.len() >= length) {
            break;
        }
    }
    Ok(request)
}

fn capture_proxy() -> std::io::Result<(String, CaptureServer)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = thread::spawn(move || {
        let mut exchanges = Vec::with_capacity(4);
        for _ in 0..4 {
            let (mut downstream, _) = listener.accept()?;
            downstream.set_read_timeout(Some(Duration::from_secs(90)))?;
            let request = read_request(&mut downstream)?;
            let upstream_request = with_connection_close(&request)?;
            let mut upstream = TcpStream::connect("127.0.0.1:18931")?;
            upstream.set_read_timeout(Some(Duration::from_secs(90)))?;
            upstream.write_all(&upstream_request)?;
            upstream.flush()?;
            let mut response = Vec::new();
            upstream.read_to_end(&mut response)?;
            downstream.write_all(&response)?;
            downstream.flush()?;
            exchanges.push(CapturedExchange { request, response });
        }
        Ok(exchanges)
    });
    Ok((format!("http://{address}/v1"), server))
}

fn with_connection_close(request: &[u8]) -> std::io::Result<Vec<u8>> {
    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "missing request headers")
        })?;
    let header_bytes = request.get(..header_end).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid request headers")
    })?;
    let headers = String::from_utf8_lossy(header_bytes);
    let mut lines = headers.lines();
    let request_line = lines.next().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "missing request line")
    })?;
    let mut forwarded = Vec::new();
    write!(&mut forwarded, "{request_line}\r\n")?;
    for line in lines {
        let is_connection = line
            .split_once(':')
            .is_some_and(|(name, _)| name.eq_ignore_ascii_case("connection"));
        if !is_connection {
            write!(&mut forwarded, "{line}\r\n")?;
        }
    }
    write!(&mut forwarded, "Connection: close\r\n\r\n")?;
    let body = request.get(header_end + 4..).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "missing request body")
    })?;
    forwarded.extend_from_slice(body);
    Ok(forwarded)
}

fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn reply(status: u16, body: impl Into<String>) -> Reply {
    Reply {
        status,
        content_type: "application/json",
        body: body.into(),
        headers: Vec::new(),
    }
}

fn props(build: &str, alias: &str, n_ctx: u32, tools: bool, calls: bool) -> String {
    json!({
        "build_info": build,
        "model_alias": alias,
        "default_generation_settings": {"n_ctx": n_ctx},
        "chat_template_caps": {
            "supports_tools": tools,
            "supports_tool_calls": calls
        }
    })
    .to_string()
}

fn models(alias: &str, n_ctx: u32) -> String {
    json!({
        "object": "list",
        "data": [{
            "id": alias,
            "object": "model",
            "owned_by": "llamacpp",
            "meta": {"n_ctx": n_ctx},
            "architecture": {"input_modalities": ["text"], "output_modalities": ["text"]}
        }]
    })
    .to_string()
}

fn profile() -> Result<LlamaCppExpectedProfile, Box<dyn std::error::Error>> {
    Ok(LlamaCppExpectedProfile::new(
        "b11804-c7fc50941",
        "qwen2.5-1.5b-instruct",
        TokenCount::new(4096),
    )?)
}

fn config() -> ClientConfig {
    ClientConfig {
        request_timeout: Duration::from_secs(2),
        connect_timeout: Duration::from_secs(1),
        stream_idle_timeout: Duration::from_secs(1),
        max_sse_frame_bytes: 4096,
        max_events_per_chunk: 16,
        max_chat_stream_bytes: 16_384,
        max_text_bytes: 4096,
        max_tool_argument_bytes: 2048,
        max_tool_calls: 4,
        max_json_response_bytes: 16_384,
        max_request_bytes: 16_384,
        max_429_retries: 0,
        max_5xx_retries: 0,
        retry_base_delay: Duration::from_millis(1),
        max_retry_delay: Duration::from_millis(1),
        reasoning_replay: ReasoningReplay::Disabled,
    }
}

fn request() -> Result<ChatRequest, Box<dyn std::error::Error>> {
    Ok(ChatRequest {
        model: ModelId::new("catalog/qwen")?,
        messages: vec![
            ChatMessage::text(ChatRole::System, "Call the function with probe_id exactly as requested.".to_owned()),
            ChatMessage::text(ChatRole::User, "Invoke p3_local_probe with probe_id=adapter-smoke.".to_owned()),
        ],
        tools: vec![ToolSpec {
            name: "p3_local_probe".into(),
            description: "Return the requested probe identifier.".into(),
            input_schema_json: r#"{"type":"object","properties":{"probe_id":{"type":"string"}},"required":["probe_id"]}"#.into(),
            effects: Effects::PURE,
        }],
        max_output_tokens: Some(TokenCount::new(64)),
        effort: Effort::Low,
    })
}

fn headers_and_body(request: &[u8]) -> Result<(String, String), Box<dyn std::error::Error>> {
    let text = String::from_utf8(request.to_vec())?;
    let (headers, body) = text.split_once("\r\n\r\n").ok_or("missing HTTP body")?;
    Ok((headers.to_owned(), body.to_owned()))
}

#[tokio::test]
async fn connect_validates_build_model_context_and_tool_template_capabilities()
-> Result<(), Box<dyn std::error::Error>> {
    let (base, server) = fixture(vec![
        reply(
            200,
            props(
                "b11804-c7fc50941",
                "qwen2.5-1.5b-instruct",
                4096,
                true,
                true,
            ),
        ),
        reply(200, models("qwen2.5-1.5b-instruct", 4096)),
    ])?;
    let adapter = LlamaCppChatClient::connect(BaseUrl::parse(&base)?, profile()?, config()).await?;
    assert_eq!(adapter.profile().model_alias(), "qwen2.5-1.5b-instruct");
    assert_eq!(
        adapter.profile().max_context_tokens(),
        TokenCount::new(4096)
    );
    let captured = server.join().map_err(|_| "fixture thread panicked")??;
    let first = String::from_utf8(captured.first().ok_or("missing props request")?.clone())?;
    let second = String::from_utf8(captured.get(1).ok_or("missing models request")?.clone())?;
    assert!(first.starts_with("GET /props "));
    assert!(second.starts_with("GET /v1/models "));
    Ok(())
}

#[tokio::test]
async fn count_and_chat_dispatch_byte_identical_serialized_body()
-> Result<(), Box<dyn std::error::Error>> {
    let stream = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_adapter\",\"function\":{\"name\":\"p3_local_probe\",\"arguments\":\"{\\\"probe_id\\\":\\\"adapter-smoke\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":321,\"completion_tokens\":12}}\n\n",
        "data: [DONE]\n\n"
    );
    let (base, server) = fixture(vec![
        reply(
            200,
            props(
                "b11804-c7fc50941",
                "qwen2.5-1.5b-instruct",
                4096,
                true,
                true,
            ),
        ),
        reply(200, models("qwen2.5-1.5b-instruct", 4096)),
        reply(
            200,
            json!({"input_tokens":321,"object":"response.input_tokens"}).to_string(),
        ),
        Reply {
            status: 200,
            content_type: "text/event-stream",
            body: stream.to_owned(),
            headers: Vec::new(),
        },
    ])?;
    let adapter = LlamaCppChatClient::connect(BaseUrl::parse(&base)?, profile()?, config()).await?;
    let request = request()?;
    let count = adapter.count_prompt(&request).await?;
    assert_eq!(count.input_tokens, TokenCount::new(321));
    assert_eq!(count.max_context_tokens, TokenCount::new(4096));
    let mut chunks = adapter.stream(request).await?;
    let mut saw_call = false;
    let mut saw_usage = false;
    while let Some(chunk) = chunks.next_chunk().await {
        match chunk? {
            ChatChunk::ToolCallDelta {
                name: Some(name), ..
            } if name == "p3_local_probe" => saw_call = true,
            ChatChunk::Usage(usage) if usage.prompt_tokens == TokenCount::new(321) => {
                saw_usage = true
            }
            _ => {}
        }
    }
    assert!(
        saw_call,
        "the stream should preserve the structured function call"
    );
    assert!(saw_usage, "the stream should preserve provider token usage");
    let captured = server.join().map_err(|_| "fixture thread panicked")??;
    let count_request = captured.get(2).ok_or("missing count request")?;
    let chat_request = captured.get(3).ok_or("missing chat request")?;
    let (count_headers, count_body) = headers_and_body(count_request)?;
    let (chat_headers, chat_body) = headers_and_body(chat_request)?;
    assert!(count_headers.starts_with("POST /v1/chat/completions/input_tokens "));
    assert!(chat_headers.starts_with("POST /v1/chat/completions "));
    assert_eq!(
        count_body.as_bytes(),
        chat_body.as_bytes(),
        "preflight and dispatch bodies must be byte-identical"
    );
    let wire: Value = serde_json::from_str(&count_body)?;
    assert_eq!(wire.get("model"), Some(&json!("qwen2.5-1.5b-instruct")));
    assert_eq!(wire.get("stream"), Some(&json!(true)));
    assert_eq!(wire.get("n"), Some(&json!(1)));
    assert_eq!(
        wire.get("stream_options"),
        Some(&json!({"include_usage": true})),
        "chat must ask llama.cpp to include exact prompt usage in the stream"
    );
    assert!(
        wire.get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.len() == 1)
    );
    Ok(())
}

#[tokio::test]
async fn separate_count_and_chat_requests_keep_their_own_generation_options()
-> Result<(), Box<dyn std::error::Error>> {
    let stream = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":322,\"completion_tokens\":1}}\n\n",
        "data: [DONE]\n\n"
    );
    let (base, server) = fixture(vec![
        reply(
            200,
            props(
                "b11804-c7fc50941",
                "qwen2.5-1.5b-instruct",
                4096,
                true,
                true,
            ),
        ),
        reply(200, models("qwen2.5-1.5b-instruct", 4096)),
        reply(
            200,
            json!({"input_tokens":321,"object":"response.input_tokens"}).to_string(),
        ),
        Reply {
            status: 200,
            content_type: "text/event-stream",
            body: stream.to_owned(),
            headers: Vec::new(),
        },
    ])?;
    let adapter = LlamaCppChatClient::connect(BaseUrl::parse(&base)?, profile()?, config()).await?;
    let mut counted_request = request()?;
    counted_request.max_output_tokens = Some(TokenCount::new(32));
    let mut dispatched_request = request()?;
    dispatched_request.max_output_tokens = Some(TokenCount::new(48));
    let _count = adapter.count_prompt(&counted_request).await?;
    let mut stream = adapter.stream(dispatched_request).await?;
    while let Some(chunk) = stream.next_chunk().await {
        let _chunk = chunk?;
    }
    let captured = server.join().map_err(|_| "fixture thread panicked")??;
    let (_, count_body) = headers_and_body(captured.get(2).ok_or("missing count request")?)?;
    let (_, chat_body) = headers_and_body(captured.get(3).ok_or("missing chat request")?)?;
    let count_wire: Value = serde_json::from_str(&count_body)?;
    let chat_wire: Value = serde_json::from_str(&chat_body)?;
    assert_eq!(count_wire.get("max_tokens"), Some(&json!(32)));
    assert_eq!(chat_wire.get("max_tokens"), Some(&json!(48)));
    assert_ne!(
        count_body, chat_body,
        "a later request must not reuse an earlier count body"
    );
    Ok(())
}

#[tokio::test]
async fn startup_rejects_wrong_build_and_missing_tool_call_capability()
-> Result<(), Box<dyn std::error::Error>> {
    for (build, tools, calls) in [
        ("wrong-build", true, true),
        ("b11804-c7fc50941", true, false),
        ("b11804-c7fc50941", false, true),
    ] {
        let (base, server) = fixture(vec![reply(
            200,
            props(build, "qwen2.5-1.5b-instruct", 4096, tools, calls),
        )])?;
        let result =
            LlamaCppChatClient::connect(BaseUrl::parse(&base)?, profile()?, config()).await;
        assert!(
            result.is_err(),
            "unmatched build or unsupported tool-call template must fail closed"
        );
        let _requests = server.join().map_err(|_| "fixture thread panicked")??;
    }
    Ok(())
}

#[tokio::test]
async fn startup_rejects_alias_and_context_mismatches_in_props_and_model_catalog()
-> Result<(), Box<dyn std::error::Error>> {
    for (alias, n_ctx) in [("another-alias", 4096), ("qwen2.5-1.5b-instruct", 2048)] {
        let (base, server) = fixture(vec![reply(
            200,
            props("b11804-c7fc50941", alias, n_ctx, true, true),
        )])?;
        let result =
            LlamaCppChatClient::connect(BaseUrl::parse(&base)?, profile()?, config()).await;
        assert!(
            result.is_err(),
            "props alias/context must match the pinned profile"
        );
        let _requests = server.join().map_err(|_| "fixture thread panicked")??;
    }

    for (alias, n_ctx) in [("another-alias", 4096), ("qwen2.5-1.5b-instruct", 2048)] {
        let (base, server) = fixture(vec![
            reply(
                200,
                props(
                    "b11804-c7fc50941",
                    "qwen2.5-1.5b-instruct",
                    4096,
                    true,
                    true,
                ),
            ),
            reply(200, models(alias, n_ctx)),
        ])?;
        let result =
            LlamaCppChatClient::connect(BaseUrl::parse(&base)?, profile()?, config()).await;
        assert!(
            result.is_err(),
            "model catalog alias/context must match the pinned profile"
        );
        let _requests = server.join().map_err(|_| "fixture thread panicked")??;
    }
    Ok(())
}

#[tokio::test]
async fn startup_rejects_malformed_props_and_model_schemas()
-> Result<(), Box<dyn std::error::Error>> {
    let (base, props_server) = fixture(vec![reply(200, "{}")])?;
    let result = LlamaCppChatClient::connect(BaseUrl::parse(&base)?, profile()?, config()).await;
    assert!(result.is_err(), "malformed /props must fail closed");
    let _props_requests = props_server
        .join()
        .map_err(|_| "fixture thread panicked")??;

    let (base, models_server) = fixture(vec![
        reply(
            200,
            props(
                "b11804-c7fc50941",
                "qwen2.5-1.5b-instruct",
                4096,
                true,
                true,
            ),
        ),
        reply(
            200,
            json!({"data":[{"id":"qwen2.5-1.5b-instruct"}]}).to_string(),
        ),
    ])?;
    let result = LlamaCppChatClient::connect(BaseUrl::parse(&base)?, profile()?, config()).await;
    assert!(
        result.is_err(),
        "malformed /v1/models metadata must fail closed"
    );
    let _models_requests = models_server
        .join()
        .map_err(|_| "fixture thread panicked")??;
    Ok(())
}

#[test]
fn loopback_socket_authority_requires_literal_ip_and_explicit_port()
-> Result<(), Box<dyn std::error::Error>> {
    let ipv4 = BaseUrl::parse("http://127.0.0.1:18931/v1")?.literal_loopback_socket_addr()?;
    assert_eq!(ipv4.to_string(), "127.0.0.1:18931");
    let ipv6 = BaseUrl::parse("http://[::1]:18931/v1")?.literal_loopback_socket_addr()?;
    assert_eq!(ipv6.to_string(), "[::1]:18931");
    assert!(
        BaseUrl::parse("http://localhost:18931/v1")?
            .literal_loopback_socket_addr()
            .is_err()
    );
    assert!(
        BaseUrl::parse("http://127.0.0.1/v1")?
            .literal_loopback_socket_addr()
            .is_err()
    );
    assert!(
        BaseUrl::parse("http://127.0.0.1:0/v1")?
            .literal_loopback_socket_addr()
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn adapter_rejects_non_http_or_non_v1_base_before_network_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    for base in [
        "https://127.0.0.1:18931/v1",
        "http://127.0.0.1:18931/custom",
    ] {
        let result = LlamaCppChatClient::connect(BaseUrl::parse(base)?, profile()?, config()).await;
        assert!(
            result.is_err(),
            "unsupported local endpoint shape must fail closed"
        );
    }
    Ok(())
}

#[tokio::test]
async fn tokenizer_rejects_malformed_counts_without_estimate_fallback()
-> Result<(), Box<dyn std::error::Error>> {
    let (base, server) = fixture(vec![
        reply(
            200,
            props(
                "b11804-c7fc50941",
                "qwen2.5-1.5b-instruct",
                4096,
                true,
                true,
            ),
        ),
        reply(200, models("qwen2.5-1.5b-instruct", 4096)),
        reply(
            200,
            json!({"input_tokens":0,"object":"response.input_tokens"}).to_string(),
        ),
    ])?;
    let adapter = LlamaCppChatClient::connect(BaseUrl::parse(&base)?, profile()?, config()).await?;
    assert!(adapter.count_prompt(&request()?).await.is_err());
    let _requests = server.join().map_err(|_| "fixture thread panicked")??;
    Ok(())
}

#[tokio::test]
async fn oversized_chat_is_rejected_before_post_dispatch() -> Result<(), Box<dyn std::error::Error>>
{
    let (base, server) = fixture(vec![
        reply(
            200,
            props(
                "b11804-c7fc50941",
                "qwen2.5-1.5b-instruct",
                4096,
                true,
                true,
            ),
        ),
        reply(200, models("qwen2.5-1.5b-instruct", 4096)),
    ])?;
    let mut bounded = config();
    bounded.max_request_bytes = 64;
    let adapter = LlamaCppChatClient::connect(BaseUrl::parse(&base)?, profile()?, bounded).await?;
    assert!(adapter.stream(request()?).await.is_err());
    let requests = server.join().map_err(|_| "fixture thread panicked")??;
    assert_eq!(
        requests.len(),
        2,
        "oversized chat must issue no provider POST"
    );
    Ok(())
}

#[tokio::test]
async fn tool_count_limit_is_rejected_before_post_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    let (base, server) = fixture(vec![
        reply(
            200,
            props(
                "b11804-c7fc50941",
                "qwen2.5-1.5b-instruct",
                4096,
                true,
                true,
            ),
        ),
        reply(200, models("qwen2.5-1.5b-instruct", 4096)),
    ])?;
    let mut bounded = config();
    bounded.max_tool_calls = 1;
    let adapter = LlamaCppChatClient::connect(BaseUrl::parse(&base)?, profile()?, bounded).await?;
    let mut oversized = request()?;
    oversized.tools.push(ToolSpec {
        name: "second_tool".into(),
        description: "Exceeds the configured tool count.".into(),
        input_schema_json: r#"{"type":"object"}"#.into(),
        effects: Effects::PURE,
    });
    assert!(adapter.stream(oversized).await.is_err());
    let requests = server.join().map_err(|_| "fixture thread panicked")??;
    assert_eq!(
        requests.len(),
        2,
        "too many tools must issue no provider POST"
    );
    Ok(())
}

#[tokio::test]
async fn local_adapter_does_not_follow_redirect_to_remote_authority()
-> Result<(), Box<dyn std::error::Error>> {
    let (base, server) = fixture(vec![Reply {
        status: 302,
        content_type: "application/json",
        body: "{}".to_owned(),
        headers: vec![(
            "Location".to_owned(),
            "https://example.invalid/collect".to_owned(),
        )],
    }])?;
    let result = LlamaCppChatClient::connect(BaseUrl::parse(&base)?, profile()?, config()).await;
    assert!(
        result.is_err(),
        "loopback startup redirect must fail closed"
    );
    let requests = server.join().map_err(|_| "fixture thread panicked")??;
    assert_eq!(
        requests.len(),
        1,
        "redirect target must not receive another request"
    );
    let request_text = String::from_utf8(requests.first().ok_or("missing props request")?.clone())?;
    assert!(request_text.starts_with("GET /props "));
    Ok(())
}

#[tokio::test]
#[ignore = "requires the pinned local llama.cpp server at 127.0.0.1:18931"]
async fn real_llama_cpp_exact_count_and_chat_usage_parity() -> Result<(), Box<dyn std::error::Error>>
{
    let expected = profile()?;
    let mut live_config = config();
    live_config.connect_timeout = Duration::from_secs(5);
    live_config.request_timeout = Duration::from_secs(60);
    live_config.stream_idle_timeout = Duration::from_secs(30);
    let (proxy_base, capture) = capture_proxy()?;
    let adapter =
        LlamaCppChatClient::connect(BaseUrl::parse(&proxy_base)?, expected, live_config).await?;
    let request = request()?;
    let count = adapter.count_prompt(&request).await?;
    let mut stream = adapter.stream(request).await?;
    let mut usage_prompt = None;
    let mut saw_tool_call = false;
    let mut call_id = None;
    let mut call_name = None;
    let mut call_arguments = String::new();
    while let Some(chunk) = stream.next_chunk().await {
        match chunk? {
            ChatChunk::ToolCallDelta {
                id,
                name,
                arguments_fragment,
                ..
            } => {
                if id.is_some() {
                    call_id = id;
                }
                if name.is_some() {
                    call_name = name;
                }
                call_arguments.push_str(&arguments_fragment);
                if call_name.as_deref() == Some("p3_local_probe") {
                    saw_tool_call = true;
                }
            }
            ChatChunk::Usage(usage) => usage_prompt = Some(usage.prompt_tokens),
            _ => {}
        }
    }
    assert!(
        saw_tool_call,
        "the actual local model should emit the requested structured function call"
    );
    assert_eq!(
        usage_prompt,
        Some(count.input_tokens),
        "exact preflight count must equal the model-reported chat prompt usage"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&call_arguments)?
            .get("probe_id")
            .and_then(Value::as_str),
        Some("adapter-smoke"),
        "the structured tool arguments should preserve the requested probe id"
    );

    let exchanges = capture
        .join()
        .map_err(|_| "capture proxy thread panicked")??;
    let count_exchange = exchanges.get(2).ok_or("missing captured count exchange")?;
    let chat_exchange = exchanges.get(3).ok_or("missing captured chat exchange")?;
    let (count_headers, count_body) = headers_and_body(&count_exchange.request)?;
    let (chat_headers, chat_body) = headers_and_body(&chat_exchange.request)?;
    assert!(count_headers.starts_with("POST /v1/chat/completions/input_tokens "));
    assert!(chat_headers.starts_with("POST /v1/chat/completions "));
    assert_eq!(count_body.as_bytes(), chat_body.as_bytes());

    let evidence_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../@prime/evidence/phase3-20261027-local/provider");
    std::fs::create_dir_all(&evidence_root)?;
    let run_id = std::process::id().to_string();
    let count_request_name = format!("live-{run_id}-count-request.json");
    let count_response_name = format!("live-{run_id}-count-response.http");
    let chat_request_name = format!("live-{run_id}-chat-request.json");
    let chat_response_name = format!("live-{run_id}-chat-response.http");
    let count_request_path = evidence_root.join(&count_request_name);
    let count_response_path = evidence_root.join(&count_response_name);
    let chat_request_path = evidence_root.join(&chat_request_name);
    let chat_response_path = evidence_root.join(&chat_response_name);
    write_new(&count_request_path, count_body.as_bytes())?;
    write_new(&count_response_path, &count_exchange.response)?;
    write_new(&chat_request_path, chat_body.as_bytes())?;
    write_new(&chat_response_path, &chat_exchange.response)?;
    let run_metadata = json!({
        "schema": 1,
        "model_repository": "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
        "model_revision": "91cad51170dc346986eccefdc2dd33a9da36ead9",
        "model_sha256": "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
        "server_release": "llama.cpp b11804 / c7fc50941",
        "server_sha256": "3a3e417a0816329829081bdd443e3045ff3b2dbe866fcef89a1d919e941ce23f",
        "count_tokens": count.input_tokens.get(),
        "chat_usage_prompt_tokens": usage_prompt.map(|tokens| tokens.get()),
        "chat_tool_call": {
            "id": call_id,
            "name": call_name,
            "arguments_json": call_arguments
        },
        "request_body_equal": true,
        "evidence_files": {
            "count_request": count_request_name,
            "count_response": count_response_name,
            "chat_request": chat_request_name,
            "chat_response": chat_response_name
        }
    });
    let run_path = evidence_root.join(format!("live-{run_id}-run.json"));
    let run_bytes = serde_json::to_vec_pretty(&run_metadata)?;
    write_new(&run_path, &run_bytes)?;
    Ok(())
}
