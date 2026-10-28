//! Real TCP fixtures for the explicit vLLM `/tokenize` wrapper.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

use polaris_nebius_cloud_token_factory::{
    ApiKey, BaseUrl, ClientConfig, ReasoningReplay, TokenFactoryClient, VllmChatClient,
};
use polaris_sdk::{
    effort::Effort,
    ids::{ModelId, TokenCount},
    registry::{ChatMessage, ChatModel, ChatRequest, Effects, PromptTokenizer, ToolSpec},
};
use secrecy::SecretString;
use serde_json::{Value, json};

type Fixture = thread::JoinHandle<std::io::Result<Vec<u8>>>;

fn fixture(status: u16, body: String, delay: Duration) -> std::io::Result<(String, Fixture)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        let request = read_request(&mut stream)?;
        thread::sleep(delay);
        let reason = if status == 200 { "OK" } else { "Not Found" };
        write!(
            stream,
            "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{body}",
            body.len()
        )?;
        stream.flush()?;
        Ok(request)
    });
    Ok((format!("http://{address}/v1"), server))
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 2048];
    let mut expected = None;
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let received = buffer.get(..count).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid fixture read length",
            )
        })?;
        request.extend_from_slice(received);
        if expected.is_none()
            && let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let header_bytes = request.get(..header_end).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid fixture header length",
                )
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
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?,
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
        request_timeout: Duration::from_millis(250),
        connect_timeout: Duration::from_millis(100),
        stream_idle_timeout: Duration::from_millis(100),
        max_sse_frame_bytes: 1024,
        max_events_per_chunk: 8,
        max_chat_stream_bytes: 4096,
        max_text_bytes: 2048,
        max_tool_argument_bytes: 1024,
        max_tool_calls: 4,
        max_json_response_bytes: 32_768,
        max_request_bytes: 2048,
        max_429_retries: 0,
        max_5xx_retries: 0,
        retry_base_delay: Duration::from_millis(1),
        max_retry_delay: Duration::from_millis(1),
        reasoning_replay: ReasoningReplay::ReasoningContent,
    }
}

fn request() -> Result<ChatRequest, Box<dyn std::error::Error>> {
    Ok(ChatRequest {
        model: ModelId::new("example/model")?,
        messages: vec![ChatMessage::assistant_calls(
            "thinking-visible".to_owned(),
            Vec::new(),
            Some("private-reasoning".to_owned()),
        )],
        tools: vec![ToolSpec {
            name: "lookup".into(),
            description: "Lookup data".into(),
            input_schema_json: "{\"type\":\"object\"}".into(),
            effects: Effects::PURE,
        }],
        max_output_tokens: Some(TokenCount::new(64)),
        effort: Effort::Medium,
    })
}

fn client(base: String) -> Result<VllmChatClient, Box<dyn std::error::Error>> {
    let base = BaseUrl::parse(&base)?;
    let key = ApiKey::new(SecretString::from("fixture-secret"))?;
    Ok(VllmChatClient::new(TokenFactoryClient::new(
        base,
        key,
        config(),
    )?))
}

fn response() -> String {
    json!({"count": 2, "max_model_len": 4096, "tokens": [11, 12] }).to_string()
}

#[tokio::test]
async fn tokenizer_sends_chat_equivalent_prompt_and_returns_server_counts()
-> Result<(), Box<dyn std::error::Error>> {
    let (base, server) = fixture(200, response(), Duration::ZERO)?;
    let wrapper = client(base)?;
    let count = wrapper.count_prompt(&request()?).await?;
    assert_eq!(
        count.input_tokens,
        TokenCount::new(2),
        "input token count should match the provider response"
    );
    assert_eq!(
        count.max_context_tokens,
        TokenCount::new(4096),
        "context length should match the provider response"
    );
    let captured = server.join().map_err(|_| "fixture thread failed")??;
    let text = String::from_utf8(captured)?;
    assert!(
        text.starts_with("POST /tokenize "),
        "tokenization should use the origin-root endpoint"
    );
    let body = text.split_once("\r\n\r\n").ok_or("missing HTTP body")?.1;
    let wire: Value = serde_json::from_str(body)?;
    assert_eq!(
        wire.get("model"),
        Some(&json!("example/model")),
        "tokenization should use the requested model"
    );
    let first_message = wire
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| messages.first())
        .ok_or("missing first message")?;
    assert_eq!(
        first_message.get("content"),
        Some(&json!("thinking-visible")),
        "tokenization should preserve chat message content"
    );
    assert_eq!(
        first_message.get("reasoning_content"),
        Some(&json!("private-reasoning")),
        "tokenization should use the configured reasoning projection"
    );
    let first_tool = wire
        .get("tools")
        .and_then(Value::as_array)
        .and_then(|tools| tools.first())
        .ok_or("missing first tool")?;
    let function = first_tool.get("function").ok_or("missing tool function")?;
    assert_eq!(
        function.get("name"),
        Some(&json!("lookup")),
        "tokenization should preserve the tool definition"
    );
    assert_eq!(
        wire.get("add_generation_prompt"),
        Some(&json!(true)),
        "vLLM tokenization should append the generation prompt"
    );
    assert_eq!(
        wire.get("continue_final_message"),
        Some(&json!(false)),
        "vLLM tokenization should not continue the final message"
    );
    assert_eq!(
        wire.get("add_special_tokens"),
        Some(&json!(false)),
        "vLLM tokenization should avoid adding duplicate special tokens"
    );
    assert_eq!(
        wire.get("return_token_strs"),
        Some(&json!(false)),
        "vLLM tokenization should return token IDs without token strings"
    );

    let (chat_base, chat_server) = fixture(200, "data: [DONE]\n\n".to_owned(), Duration::ZERO)?;
    let chat_client = client(chat_base)?;
    let _stream = chat_client.stream(request()?).await?;
    let chat_capture = chat_server
        .join()
        .map_err(|_| "chat fixture thread failed")??;
    let chat_text = String::from_utf8(chat_capture)?;
    let chat_body = chat_text
        .split_once("\r\n\r\n")
        .ok_or("missing chat HTTP body")?
        .1;
    let chat_wire: Value = serde_json::from_str(chat_body)?;
    assert_eq!(
        wire.get("model"),
        chat_wire.get("model"),
        "tokenization and chat should use the same model field"
    );
    assert_eq!(
        wire.get("messages"),
        chat_wire.get("messages"),
        "tokenization and chat should serialize identical messages"
    );
    assert_eq!(
        wire.get("tools"),
        chat_wire.get("tools"),
        "tokenization and chat should serialize identical tools"
    );
    assert!(
        text.to_ascii_lowercase()
            .contains("authorization: bearer fixture-secret"),
        "tokenization should use the configured client authentication"
    );
    Ok(())
}

#[tokio::test]
async fn tokenizer_preserves_oversized_prompt_count_for_trimming()
-> Result<(), Box<dyn std::error::Error>> {
    let (base, _server) = fixture(
        200,
        json!({"count": 5000, "max_model_len": 4096, "tokens": vec![7_u32; 5000] }).to_string(),
        Duration::ZERO,
    )?;
    let count = client(base)?.count_prompt(&request()?).await?;
    assert_eq!(
        count.input_tokens,
        TokenCount::new(5000),
        "counts above the context limit must remain available for trimming"
    );
    assert_eq!(
        count.max_context_tokens,
        TokenCount::new(4096),
        "oversized prompt result should report the provider context limit"
    );
    Ok(())
}

#[tokio::test]
async fn tokenizer_omits_empty_tools() -> Result<(), Box<dyn std::error::Error>> {
    let (base, server) = fixture(200, response(), Duration::ZERO)?;
    let wrapper = client(base)?;
    let mut request = request()?;
    request.tools.clear();
    let _count = wrapper.count_prompt(&request).await?;
    let captured = server.join().map_err(|_| "fixture thread failed")??;
    let text = String::from_utf8(captured)?;
    let body = text.split_once("\r\n\r\n").ok_or("missing HTTP body")?.1;
    let wire: Value = serde_json::from_str(body)?;
    assert!(
        wire.get("tools").is_none(),
        "empty tools must be omitted from the vLLM tokenization request"
    );
    Ok(())
}

#[tokio::test]
async fn chat_model_wrapper_forwards_stream_to_same_client()
-> Result<(), Box<dyn std::error::Error>> {
    let (base, server) = fixture(200, "data: [DONE]\n\n".to_owned(), Duration::ZERO)?;
    let wrapper = client(base)?;
    let _stream = wrapper.stream(request()?).await?;
    let captured = server.join().map_err(|_| "fixture thread failed")??;
    let text = String::from_utf8(captured)?;
    assert!(
        text.starts_with("POST /v1/chat/completions "),
        "chat streaming should forward through the wrapped client"
    );
    Ok(())
}

#[tokio::test]
async fn tokenizer_rejects_malformed_counts_and_token_ids() -> Result<(), Box<dyn std::error::Error>>
{
    for body in [
        json!({"count": 0, "max_model_len": 20, "tokens": []}),
        json!({"count": 2, "max_model_len": 20, "tokens": [1]}),
        json!({"count": 1, "max_model_len": 0, "tokens": [1]}),
        json!({"count": 1, "max_model_len": 20, "tokens": [4294967296_u64]}),
    ] {
        let (base, _server) = fixture(200, body.to_string(), Duration::ZERO)?;
        let result = client(base)?.count_prompt(&request()?).await;
        assert!(
            result.is_err(),
            "malformed counts, context lengths, and token IDs must be rejected"
        );
    }
    Ok(())
}

#[tokio::test]
async fn tokenizer_bounds_response_and_deadline_and_redacts_failures()
-> Result<(), Box<dyn std::error::Error>> {
    let (base, _server) = fixture(
        404,
        "secret response content must not escape".to_owned(),
        Duration::ZERO,
    )?;
    let wrapper = client(base)?;
    let error = wrapper
        .count_prompt(&request()?)
        .await
        .err()
        .ok_or("404 accepted")?;
    assert!(
        !error.to_string().contains("secret response content"),
        "HTTP error bodies must not appear in tokenizer errors"
    );
    assert!(
        !format!("{wrapper:?}").contains("fixture-secret"),
        "wrapper Debug output must redact configured credentials"
    );

    let (base, _server) = fixture(
        200,
        format!("{{\"padding\":\"{}\"}}", "x".repeat(40_000)),
        Duration::ZERO,
    )?;
    let error = client(base)?
        .count_prompt(&request()?)
        .await
        .err()
        .ok_or("oversized body accepted")?;
    assert!(
        !error.to_string().contains(&"x".repeat(32)),
        "oversized response content must not appear in tokenizer errors"
    );

    let (base, _server) = fixture(200, response(), Duration::from_millis(400))?;
    let error = client(base)?
        .count_prompt(&request()?)
        .await
        .err()
        .ok_or("timeout accepted")?;
    assert!(
        !error.to_string().contains("fixture-secret"),
        "timeout errors must not reveal configured credentials"
    );
    Ok(())
}

#[tokio::test]
async fn tokenizer_rejects_oversized_input_before_network_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    drop(listener);
    let wrapper = client(format!("http://{address}/v1"))?;
    let mut oversized = request()?;
    let first_message = oversized
        .messages
        .get_mut(0)
        .ok_or("missing request message")?;
    first_message.content = "x".repeat(4096);
    let result = wrapper.count_prompt(&oversized).await;
    assert!(
        result.is_err(),
        "oversized prompt input must be rejected before transport dispatch"
    );
    if let Err(error) = result {
        assert!(
            error.to_string().contains("configured bound"),
            "oversized prompt input should fail on its configured bound"
        );
    }
    Ok(())
}
