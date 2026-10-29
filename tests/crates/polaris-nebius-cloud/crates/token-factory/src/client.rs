// Unit tests of crates/polaris-nebius-cloud/crates/token-factory/src/client.rs (`tests`); compiled into that module via #[path].
use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    num::NonZeroU32,
    thread,
    time::Duration,
};

use super::{
    ApiKey, BaseUrl, ClientConfig, EmbeddingConfig, RateLimitPolicy, ReasoningReplay,
    TokenFactoryClient, parse_retry_after, retry_delay,
};
use polaris_ratelimit::{DualLimiter, MonotonicClock, Priority, Quota, ShedPolicy};
use polaris_sdk::{
    effort::Effort,
    ids::{ModelId, TokenCount},
    registry::{
        ChatChunk, ChatChunkSource, ChatMessage, ChatModel, ChatRequest, ChatRole, Effects,
        FinishReason, ModelError, ToolSpec,
    },
};
use secrecy::SecretString;

type FixtureServer = thread::JoinHandle<io::Result<Vec<Vec<u8>>>>;
type FixtureResponse = (u16, Vec<(&'static str, &'static str)>, String);

fn fixture_server(responses: Vec<FixtureResponse>) -> io::Result<(String, FixtureServer)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = thread::spawn(move || {
        let mut requests = Vec::with_capacity(responses.len());
        for (status, headers, body) in responses {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            requests.push(read_request(&mut stream)?);
            let reason = match status {
                200 => "OK",
                302 => "Found",
                400 => "Bad Request",
                401 => "Unauthorized",
                429 => "Too Many Requests",
                _ => "Fixture Status",
            };
            write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            )?;
            for (name, value) in headers {
                write!(stream, "{name}: {value}\r\n")?;
            }
            write!(stream, "\r\n{body}")?;
            stream.flush()?;
        }
        Ok(requests)
    });
    Ok((format!("http://{address}/v1"), server))
}

fn read_request(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 2048];
    let mut expected = None;
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let received = buffer.get(..count).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid fixture read length")
        })?;
        request.extend_from_slice(received);
        if expected.is_none()
            && let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let header_bytes = request.get(..header_end).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid fixture header length")
            })?;
            let headers = String::from_utf8_lossy(header_bytes);
            let content_length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then_some(value.trim())
            });
            let body_length = match content_length {
                Some(value) => value
                    .parse::<usize>()
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
                None => 0,
            };
            expected = header_end
                .checked_add(4)
                .and_then(|length| length.checked_add(body_length));
        }
        if expected.is_some_and(|length| request.len() >= length) {
            break;
        }
    }
    Ok(request)
}

fn config() -> ClientConfig {
    ClientConfig {
        request_timeout: Duration::from_secs(5),
        connect_timeout: Duration::from_secs(1),
        stream_idle_timeout: Duration::from_secs(2),
        max_sse_frame_bytes: 16 * 1024,
        max_events_per_chunk: 32,
        max_chat_stream_bytes: 64 * 1024,
        max_text_bytes: 32 * 1024,
        max_tool_argument_bytes: 16 * 1024,
        max_tool_calls: 8,
        max_json_response_bytes: 32 * 1024,
        max_request_bytes: 32 * 1024,
        max_429_retries: 1,
        max_5xx_retries: 1,
        retry_base_delay: Duration::from_millis(1),
        max_retry_delay: Duration::from_millis(10),
        reasoning_replay: ReasoningReplay::Disabled,
    }
}

fn rate_policy() -> Result<RateLimitPolicy, Box<dyn std::error::Error>> {
    let quota = Quota {
        rpm: NonZeroU32::new(120).ok_or("invalid fixture RPM")?,
        tpm: NonZeroU32::new(60_000).ok_or("invalid fixture TPM")?,
        burst_requests: 8,
        burst_tokens: 8_000,
    };
    let limiter = DualLimiter::new(quota, MonotonicClock::default(), ShedPolicy::default())?;
    let estimator = |_request: &ChatRequest| Ok::<TokenCount, super::TfError>(TokenCount::new(24));
    Ok(RateLimitPolicy::new(
        limiter,
        estimator,
        Priority::InteractiveUser,
    ))
}

#[test]
fn retry_fallback_jitter_stays_within_exponential_and_global_caps() {
    let config = config();
    let cap = config.retry_base_delay.saturating_mul(4);
    for retry in 0..3 {
        let delay = retry_delay(None, retry, config);
        let shift = u32::from(retry);
        let multiplier = if shift < u32::BITS {
            1_u32 << shift
        } else {
            u32::MAX
        };
        let attempt_cap = config.retry_base_delay.saturating_mul(multiplier);
        assert!(delay <= attempt_cap.min(config.max_retry_delay));
        assert!(delay <= cap);
    }
}

#[test]
fn retry_after_is_honored_but_capped() {
    let config = config();
    let header = reqwest::header::HeaderValue::from_static("120");
    assert_eq!(
        retry_delay(Some(&header), 0, config),
        config.max_retry_delay
    );
    let zero = reqwest::header::HeaderValue::from_static("0");
    assert_eq!(retry_delay(Some(&zero), 0, config), Duration::ZERO);
}

#[test]
fn retry_after_accepts_http_date_form() -> Result<(), Box<dyn std::error::Error>> {
    let date = reqwest::header::HeaderValue::from_static("Thu, 01 Jan 2099 00:00:00 GMT");
    let parsed = parse_retry_after(&date).ok_or("valid HTTP date was rejected")?;
    if parsed < Duration::from_secs(60 * 60 * 24 * 365) {
        return Err("future HTTP date did not produce a bounded positive delay".into());
    }
    if retry_delay(Some(&date), 0, config()) != config().max_retry_delay {
        return Err("future HTTP date was not capped by configuration".into());
    }
    Ok(())
}

#[tokio::test]
async fn synthetic_503_retries_within_configured_attempt_budget()
-> Result<(), Box<dyn std::error::Error>> {
    let body = r#"{"data":[{"id":"fixture-model"}]}"#;
    let (base, server) = fixture_server(vec![
        (503, Vec::new(), String::new()),
        (200, Vec::new(), body.to_owned()),
    ])?;
    let mut policy = config();
    policy.max_5xx_retries = 1;
    let client = client(base, policy)?;
    let models = client.list_models().await?;
    let captured = server
        .join()
        .map_err(|_| io::Error::other("fixture server panicked"))??;
    if models.len() != 1 || captured.len() != 2 {
        return Err("bounded 503 retry did not repeat exactly once".into());
    }
    Ok(())
}

#[tokio::test]
async fn slow_loris_stream_is_stopped_by_total_request_deadline()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = thread::spawn(move || -> io::Result<Vec<u8>> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        let request = read_request(&mut stream)?;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
        )?;
        stream.flush()?;
        for _ in 0..12 {
            thread::sleep(Duration::from_millis(25));
            let data = b": ping\n";
            if write!(stream, "{:X}\r\n", data.len())
                .and_then(|()| stream.write_all(data))
                .and_then(|()| stream.write_all(b"\r\n"))
                .and_then(|()| stream.flush())
                .is_err()
            {
                break;
            }
        }
        Ok(request)
    });
    let mut policy = config();
    policy.request_timeout = Duration::from_millis(120);
    policy.stream_idle_timeout = Duration::from_millis(80);
    let client = client(format!("http://{address}/v1"), policy)?;
    let request = ChatRequest {
        model: ModelId::new("fixture-model")?,
        messages: vec![ChatMessage::text(ChatRole::User, "prompt".to_owned())],
        tools: Vec::new(),
        max_output_tokens: Some(TokenCount::new(32)),
        effort: Effort::Low,
    };
    let mut response = client.stream(request).await?;
    let result = response.next_chunk().await;
    if !matches!(result, Some(Err(ModelError::Timeout))) {
        return Err(format!(
            "trickled stream ended unexpectedly before/after deadline: {result:?}"
        )
        .into());
    }
    let captured = server
        .join()
        .map_err(|_| io::Error::other("slow-loris fixture panicked"))??;
    if captured.is_empty() {
        return Err("slow-loris fixture did not receive the request".into());
    }
    Ok(())
}

#[tokio::test]
async fn retry_after_penalty_blocks_later_admission_within_request_deadline()
-> Result<(), Box<dyn std::error::Error>> {
    let (base, server) = fixture_server(vec![(429, vec![("Retry-After", "1")], String::new())])?;
    let mut config = config();
    config.max_429_retries = 0;
    config.request_timeout = Duration::from_millis(20);
    let policy = rate_policy()?;
    let client = client(base, config)?.with_rate_limit(policy.clone());
    let first = client.list_models().await;
    if !matches!(first, Err(super::TfError::Http { status: 429, .. })) {
        return Err("initial 429 was not returned after retry budget was exhausted".into());
    }
    let request = ChatRequest {
        model: ModelId::new("fixture-model")?,
        messages: vec![ChatMessage::text(ChatRole::User, "prompt".to_owned())],
        tools: Vec::new(),
        max_output_tokens: Some(TokenCount::new(32)),
        effort: Effort::Low,
    };
    if !matches!(client.stream(request).await, Err(ModelError::Timeout)) {
        return Err("shared Retry-After penalty did not bound later local admission".into());
    }
    let _captured = server
        .join()
        .map_err(|_| io::Error::other("fixture server panicked"))??;
    if policy.window().requests != 0 {
        return Err("a request that timed out before dispatch was charged".into());
    }
    Ok(())
}

fn client(
    base: String,
    config: ClientConfig,
) -> Result<TokenFactoryClient, Box<dyn std::error::Error>> {
    let base = BaseUrl::parse(&base)?;
    let key = ApiKey::new(SecretString::from("synthetic-fixture-key".to_owned()))?;
    Ok(TokenFactoryClient::new(base, key, config)?)
}

fn request_bytes(request: &[u8]) -> Result<&str, Box<dyn std::error::Error>> {
    let request = std::str::from_utf8(request)?;
    Ok(request)
}

#[test]
fn base_url_requires_tls_for_non_loopback_and_rejects_embedded_credentials()
-> Result<(), super::TfError> {
    if !matches!(
        BaseUrl::parse("http://api.example.test/v1"),
        Err(super::TfError::Configuration)
    ) || !matches!(
        BaseUrl::parse("https://user:secret@api.example.test/v1"),
        Err(super::TfError::Configuration)
    ) || BaseUrl::parse("http://127.0.0.1:11434/v1").is_err()
    {
        return Err(super::TfError::Protocol);
    }
    Ok(())
}

#[test]
fn responses_request_replays_linked_function_call_and_result() -> Result<(), super::TfError> {
    let request = ChatRequest {
        model: ModelId::new("fixture-model").map_err(|_| super::TfError::BadRequest)?,
        messages: vec![
            ChatMessage::text(ChatRole::User, "lookup".to_owned()),
            ChatMessage::assistant_calls(
                String::new(),
                vec![polaris_sdk::registry::ChatToolCall {
                    id: "call_1".to_owned(),
                    name: "lookup".to_owned(),
                    arguments_json: r#"{"q":"x"}"#.to_owned(),
                }],
                None,
            ),
            ChatMessage::tool_result("call_1".to_owned(), "found".to_owned()),
        ],
        tools: Vec::new(),
        max_output_tokens: None,
        effort: Effort::Low,
    };
    let body = super::responses_request_json(&request)?;
    let input = body
        .get("input")
        .and_then(serde_json::Value::as_array)
        .ok_or(super::TfError::Protocol)?;
    if input.len() != 4
        || input
            .get(2)
            .and_then(|item| item.get("type"))
            .and_then(serde_json::Value::as_str)
            != Some("function_call")
        || input
            .get(2)
            .and_then(|item| item.get("call_id"))
            .and_then(serde_json::Value::as_str)
            != Some("call_1")
        || input
            .get(3)
            .and_then(|item| item.get("type"))
            .and_then(serde_json::Value::as_str)
            != Some("function_call_output")
        || input
            .get(3)
            .and_then(|item| item.get("call_id"))
            .and_then(serde_json::Value::as_str)
            != Some("call_1")
    {
        return Err(super::TfError::Protocol);
    }
    Ok(())
}

#[test]
fn empty_chat_history_is_rejected_by_both_endpoints() -> Result<(), super::TfError> {
    let request = ChatRequest {
        model: ModelId::new("fixture-model").map_err(|_| super::TfError::BadRequest)?,
        messages: Vec::new(),
        tools: Vec::new(),
        max_output_tokens: None,
        effort: Effort::Low,
    };
    if !matches!(
        super::chat_request_json(&request, ReasoningReplay::Disabled),
        Err(super::TfError::BadRequest)
    ) || !matches!(
        super::responses_request_json(&request),
        Err(super::TfError::BadRequest)
    ) {
        return Err(super::TfError::Protocol);
    }
    Ok(())
}

/// Shape recorded from OpenRouter (nemotron-3.5-lightning:free, 2026-10-28): `reasoning`
/// instead of `reasoning_content`, extra provider fields, and a usage chunk that repeats
/// `"content":""` together with the already-sent finish reason.
#[tokio::test]
async fn openrouter_stream_shape_is_accepted() -> Result<(), Box<dyn std::error::Error>> {
    let body = concat!(
        ": OPENROUTER PROCESSING\n\n",
        "data: {\"id\":\"gen-1\",\"provider\":\"Nvidia\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\",\"role\":\"assistant\",\"reasoning\":\"We need\",\"reasoning_details\":[{\"type\":\"reasoning.text\",\"text\":\"We need\",\"format\":\"unknown\",\"index\":0}]},\"finish_reason\":null,\"native_finish_reason\":null}]}\n\n",
        "data: {\"id\":\"gen-1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":null,\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"\"}}]},\"finish_reason\":null,\"native_finish_reason\":null}]}\n\n",
        "data: {\"id\":\"gen-1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":null,\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\\\"Cargo.toml\\\"}\"}}]},\"finish_reason\":null,\"native_finish_reason\":null}]}\n\n",
        "data: {\"id\":\"gen-1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\",\"role\":\"assistant\",\"reasoning\":null},\"finish_reason\":\"tool_calls\",\"native_finish_reason\":\"tool_calls\"}]}\n\n",
        "data: {\"id\":\"gen-1\",\"service_tier\":null,\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\",\"role\":\"assistant\"},\"finish_reason\":\"tool_calls\",\"native_finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":272,\"completion_tokens\":81,\"total_tokens\":353,\"cost\":0,\"is_byok\":false,\"prompt_tokens_details\":{\"cached_tokens\":0,\"cache_write_tokens\":0}}}\n\n",
        "data: [DONE]\n\n"
    );
    let (base, server) = fixture_server(vec![(
        200,
        vec![("Content-Type", "text/event-stream")],
        body.to_owned(),
    )])?;
    let client = client(base, config())?;
    let request = ChatRequest {
        model: ModelId::new("nvidia/nemotron-3.5-lightning:free")?,
        messages: vec![ChatMessage::text(ChatRole::User, "prompt".to_owned())],
        tools: Vec::new(),
        max_output_tokens: Some(TokenCount::new(64)),
        effort: Effort::Medium,
    };
    let mut stream = client.stream(request).await?;
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next_chunk().await {
        chunks.push(chunk?);
    }
    let _captured = server
        .join()
        .map_err(|_| io::Error::other("fixture server panicked"))??;
    assert!(chunks.contains(&ChatChunk::Reasoning("We need".to_owned())));
    assert!(chunks.contains(&ChatChunk::ToolCallDelta {
        index: 0,
        id: Some("call-1".to_owned()),
        name: Some("read_file".to_owned()),
        arguments_fragment: String::new(),
    }));
    assert!(chunks.contains(&ChatChunk::Finished(FinishReason::ToolCalls)));
    assert!(
        chunks
            .iter()
            .any(|chunk| matches!(chunk, ChatChunk::Usage(_)))
    );
    Ok(())
}

#[tokio::test]
async fn non_empty_content_after_finish_is_still_rejected() -> Result<(), Box<dyn std::error::Error>>
{
    let body = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"late\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let (base, server) = fixture_server(vec![(
        200,
        vec![("Content-Type", "text/event-stream")],
        body.to_owned(),
    )])?;
    let client = client(base, config())?;
    let request = ChatRequest {
        model: ModelId::new("fixture-model")?,
        messages: vec![ChatMessage::text(ChatRole::User, "prompt".to_owned())],
        tools: Vec::new(),
        max_output_tokens: Some(TokenCount::new(64)),
        effort: Effort::Low,
    };
    let mut stream = client.stream(request).await?;
    let mut failed = false;
    while let Some(chunk) = stream.next_chunk().await {
        if chunk.is_err() {
            failed = true;
        }
    }
    let _captured = server
        .join()
        .map_err(|_| io::Error::other("fixture server panicked"))??;
    assert!(
        failed,
        "real content after the finish reason stays a protocol error"
    );
    Ok(())
}

#[tokio::test]
async fn synthetic_chat_fixture_streams_reasoning_tool_deltas_and_usage()
-> Result<(), Box<dyn std::error::Error>> {
    let body = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\",\"reasoning_content\":\"brief\"}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{\\\"q\\\":\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"x\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":8,\"completion_tokens\":4,\"prompt_tokens_details\":{\"cached_tokens\":3}}}\n\n",
        "data: [DONE]\n\n"
    );
    let (base, server) = fixture_server(vec![(
        200,
        vec![("Content-Type", "text/event-stream")],
        body.to_owned(),
    )])?;
    let rate_limit = rate_policy()?;
    let client = client(base, config())?.with_rate_limit(rate_limit.clone());
    let request = ChatRequest {
        model: ModelId::new("fixture-model")?,
        messages: vec![ChatMessage::text(ChatRole::User, "prompt".to_owned())],
        tools: vec![ToolSpec {
            name: "lookup".into(),
            description: "Search the fixture index".into(),
            input_schema_json:
                r#"{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}"#.into(),
            effects: Effects::PURE,
        }],
        max_output_tokens: Some(TokenCount::new(64)),
        effort: Effort::Low,
    };
    let mut stream = client.stream(request).await?;
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next_chunk().await {
        chunks.push(chunk?);
    }
    let captured = server
        .join()
        .map_err(|_| io::Error::other("fixture server panicked"))??;
    let raw_request = request_bytes(captured.first().ok_or("missing captured request")?)?;
    if !raw_request.starts_with("POST /v1/chat/completions ")
        || !raw_request
            .to_ascii_lowercase()
            .contains("authorization: bearer synthetic-fixture-key")
        || !raw_request.contains("\"stream\":true")
        || !raw_request.contains("\"lookup\"")
    {
        return Err("chat fixture did not receive the expected authenticated request".into());
    }
    let debug = format!("{client:?}");
    if debug.contains("synthetic-fixture-key") {
        return Err("client debug output exposed the fixture credential".into());
    }
    if !chunks.contains(&ChatChunk::Text("hello".to_owned()))
        || !chunks.contains(&ChatChunk::Reasoning("brief".to_owned()))
        || !chunks.contains(&ChatChunk::ToolCallDelta {
            index: 0,
            id: Some("call_1".to_owned()),
            name: Some("lookup".to_owned()),
            arguments_fragment: "{\"q\":".to_owned(),
        })
        || !chunks.contains(&ChatChunk::ToolCallDelta {
            index: 0,
            id: None,
            name: None,
            arguments_fragment: "\"x\"}".to_owned(),
        })
        || !chunks.contains(&ChatChunk::Usage(polaris_sdk::registry::Usage {
            prompt_tokens: TokenCount::new(8),
            completion_tokens: TokenCount::new(4),
            cached_tokens: TokenCount::new(3),
        }))
        || !chunks.contains(&ChatChunk::Finished(
            polaris_sdk::registry::FinishReason::ToolCalls,
        ))
    {
        return Err("chat fixture chunks did not preserve the expected response fields".into());
    }
    let usage_window = rate_limit.window();
    if usage_window.requests != 1
        || usage_window.reserved_tokens != 88
        || usage_window.actual_tokens != 12
    {
        return Err(format!("unexpected rate-limit reconciliation: {usage_window:?}").into());
    }
    Ok(())
}

#[tokio::test]
async fn synthetic_models_and_embeddings_fixture_orders_vectors_and_retries_429()
-> Result<(), Box<dyn std::error::Error>> {
    let (base, server) = fixture_server(vec![
        (429, vec![("Retry-After", "0")], String::new()),
        (
            200,
            vec![("Content-Type", "application/json")],
            r#"{"data":[{"id":"fixture-embed"}]}"#.to_owned(),
        ),
        (
            200,
            vec![("Content-Type", "application/json")],
            r#"{"data":[{"index":1,"embedding":[0.0,1.0]},{"index":0,"embedding":[1.0,0.0]}]}"#
                .to_owned(),
        ),
    ])?;
    let client = client(base, config())?;
    let models = client.list_models().await?;
    if models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<Vec<_>>()
        != ["fixture-embed"]
    {
        return Err("models fixture response was not preserved".into());
    }
    let model = ModelId::new("fixture-embed")?;
    let batch = client
        .embeddings(
            &model,
            &["first", "second"],
            &EmbeddingConfig {
                max_batch_items: 4,
                max_input_bytes: 64,
                max_dimensions: 8,
            },
        )
        .await?;
    if batch.dimension != 2 || batch.vectors != [vec![1.0, 0.0], vec![0.0, 1.0]] {
        return Err("embedding vectors were not validated and reordered by response index".into());
    }
    let captured = server
        .join()
        .map_err(|_| io::Error::other("fixture server panicked"))??;
    if captured.len() != 3
        || !request_bytes(captured.first().ok_or("missing first request")?)?
            .starts_with("GET /v1/models ")
        || !request_bytes(captured.get(1).ok_or("missing retried request")?)?
            .starts_with("GET /v1/models ")
        || !request_bytes(captured.get(2).ok_or("missing embeddings request")?)?
            .starts_with("POST /v1/embeddings ")
    {
        return Err(
            "bounded 429 retry or embeddings route did not match the fixture contract".into(),
        );
    }
    Ok(())
}

#[tokio::test]
async fn synthetic_unauthorized_fixture_maps_without_echoing_response_body()
-> Result<(), Box<dyn std::error::Error>> {
    let (base, server) = fixture_server(vec![(
        401,
        Vec::new(),
        r#"{"error":{"message":"credential synthetic-secret-value rejected"}}"#.to_owned(),
    )])?;
    let client = client(base, config())?;
    let error = client
        .list_models()
        .await
        .err()
        .ok_or("unauthorized response unexpectedly succeeded")?;
    if !matches!(error, super::TfError::Unauthorized)
        || error.to_string().contains("synthetic-secret-value")
    {
        return Err("401 mapping exposed a response body or used the wrong error".into());
    }
    let captured = server
        .join()
        .map_err(|_| io::Error::other("fixture server panicked"))??;
    let raw_request = request_bytes(captured.first().ok_or("missing captured request")?)?;
    if !raw_request
        .to_ascii_lowercase()
        .contains("authorization: bearer synthetic-fixture-key")
    {
        return Err("credential was not attached to the authenticated fixture request".into());
    }
    Ok(())
}

#[tokio::test]
async fn synthetic_redirect_is_not_followed_with_bearer_credentials()
-> Result<(), Box<dyn std::error::Error>> {
    let (base, server) = fixture_server(vec![(
        302,
        vec![("Location", "http://127.0.0.1:9/capture")],
        String::new(),
    )])?;
    let client = client(base, config())?;
    let error = client
        .list_models()
        .await
        .err()
        .ok_or("redirect response unexpectedly succeeded")?;
    if !matches!(error, super::TfError::Http { status: 302, .. }) {
        return Err("redirect was followed or mapped to the wrong status".into());
    }
    let captured = server
        .join()
        .map_err(|_| io::Error::other("fixture server panicked"))??;
    if captured.len() != 1 {
        return Err("client issued an unexpected request after the redirect".into());
    }
    Ok(())
}

#[tokio::test]
async fn streamed_tool_call_state_respects_configured_count_bound()
-> Result<(), Box<dyn std::error::Error>> {
    let body = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[",
        "{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{}\"}},",
        "{\"index\":1,\"id\":\"call_2\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let (base, server) = fixture_server(vec![(
        200,
        vec![("Content-Type", "text/event-stream")],
        body.to_owned(),
    )])?;
    let mut config = config();
    config.max_tool_calls = 1;
    let client = client(base, config)?;
    let request = ChatRequest {
        model: ModelId::new("fixture-model")?,
        messages: vec![ChatMessage::text(ChatRole::User, "prompt".to_owned())],
        tools: vec![ToolSpec {
            name: "lookup".into(),
            description: "Search the fixture index".into(),
            input_schema_json: r#"{"type":"object"}"#.into(),
            effects: Effects::PURE,
        }],
        max_output_tokens: None,
        effort: Effort::Low,
    };
    let mut stream = client.stream(request).await?;
    let first = stream
        .next_chunk()
        .await
        .ok_or("bounded tool-call stream ended without a result")?;
    if !matches!(first, Err(ModelError::Protocol(_))) {
        return Err("tool-call count overflow did not fail with a protocol error".into());
    }
    let _captured = server
        .join()
        .map_err(|_| io::Error::other("fixture server panicked"))??;
    Ok(())
}

#[tokio::test]
async fn synthetic_responses_fixture_streams_text_tool_call_and_usage()
-> Result<(), Box<dyn std::error::Error>> {
    let body = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"delta\":\"answer\"}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"output_index\":1,\"item\":{\"type\":\"function_call\",\"call_id\":\"call_1\",\"name\":\"lookup\",\"arguments\":\"\"}}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"output_index\":1,\"call_id\":\"call_1\",\"delta\":\"{\\\"q\\\":\\\"x\\\"}\"}\n\n",
        "data: {\"type\":\"response.function_call_arguments.done\",\"output_index\":1,\"call_id\":\"call_1\",\"name\":\"lookup\",\"arguments\":\"{\\\"q\\\":\\\"x\\\"}\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":8,\"output_tokens\":4,\"input_tokens_details\":{\"cached_tokens\":3}}}}\n\n"
    );
    let (base, server) = fixture_server(vec![(
        200,
        vec![("Content-Type", "text/event-stream")],
        body.to_owned(),
    )])?;
    let client = client(base, config())?;
    let request = ChatRequest {
        model: ModelId::new("fixture-model")?,
        messages: vec![ChatMessage::text(ChatRole::User, "prompt".to_owned())],
        tools: vec![ToolSpec {
            name: "lookup".into(),
            description: "Search the fixture index".into(),
            input_schema_json: r#"{"type":"object","properties":{"q":{"type":"string"}}}"#.into(),
            effects: Effects::PURE,
        }],
        max_output_tokens: Some(TokenCount::new(64)),
        effort: Effort::Low,
    };
    let mut stream = client.stream_responses(request).await?;
    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next_chunk().await {
        chunks.push(chunk?);
    }
    let captured = server
        .join()
        .map_err(|_| io::Error::other("fixture server panicked"))??;
    let raw_request = request_bytes(captured.first().ok_or("missing captured request")?)?;
    if !raw_request.starts_with("POST /v1/responses ")
        || !raw_request.contains("\"input\"")
        || !raw_request.contains("\"max_output_tokens\":64")
        || !raw_request.contains("\"tool_choice\":\"auto\"")
    {
        return Err("Responses fixture did not receive the expected OpenAI-style request".into());
    }
    if !chunks.contains(&ChatChunk::Text("answer".to_owned()))
        || !chunks.contains(&ChatChunk::ToolCallDelta {
            index: 1,
            id: Some("call_1".to_owned()),
            name: Some("lookup".to_owned()),
            arguments_fragment: "{\"q\":\"x\"}".to_owned(),
        })
        || !chunks.contains(&ChatChunk::Usage(polaris_sdk::registry::Usage {
            prompt_tokens: TokenCount::new(8),
            completion_tokens: TokenCount::new(4),
            cached_tokens: TokenCount::new(3),
        }))
        || !chunks.contains(&ChatChunk::Finished(
            polaris_sdk::registry::FinishReason::ToolCalls,
        ))
    {
        return Err("Responses event deltas or final usage were not preserved".into());
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires the explicitly installed local Ollama nemotron-mini:latest model"]
async fn local_ollama_chat_model_smoke() -> Result<(), Box<dyn std::error::Error>> {
    let mut policy = config();
    policy.request_timeout = Duration::from_secs(180);
    policy.stream_idle_timeout = Duration::from_secs(90);
    let base = BaseUrl::parse("http://127.0.0.1:11434/v1")?;
    let client = TokenFactoryClient::anonymous_loopback(base, policy)?;
    let request = ChatRequest {
            model: ModelId::new("nemotron-mini:latest")?,
            messages: vec![ChatMessage::text(
                ChatRole::User,
                "Call lookup exactly once with query polaris. Do not answer in prose.".to_owned(),
            )],
            tools: vec![ToolSpec {
                name: "lookup".into(),
                description: "Look up a short query in the local fixture index.".into(),
                input_schema_json: r#"{"type":"object","properties":{"query":{"type":"string"}},"required":["query"],"additionalProperties":false}"#.into(),
                effects: Effects::PURE,
            }],
            max_output_tokens: Some(TokenCount::new(64)),
            effort: Effort::Low,
        };
    let mut stream = client.stream(request).await?;
    let mut called_lookup = false;
    let mut finished = false;
    let mut observed = Vec::new();
    while let Some(chunk) = stream.next_chunk().await {
        let chunk = chunk?;
        observed.push(format!("{chunk:?}"));
        match chunk {
            ChatChunk::ToolCallDelta { name, .. } if name.as_deref() == Some("lookup") => {
                called_lookup = true;
            }
            ChatChunk::Finished(FinishReason::ToolCalls) => finished = true,
            _ => {}
        }
    }
    if !finished || !called_lookup {
        return Err(format!(
            "local model did not complete the tool-call adapter probe: {observed:?}"
        )
        .into());
    }
    Ok(())
}
