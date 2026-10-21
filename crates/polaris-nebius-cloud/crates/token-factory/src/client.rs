//! OpenAI-compatible HTTP adapter and SDK trait implementations.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    future::Future,
    net::IpAddr,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt};
use polaris_sdk::{
    ids::{ModelId, TokenCount},
    registry::{ChatChunk, ChatChunkSource, ChatModel, ChatRequest, ChatRole, ChatToolCall, EmbeddingProvider, FinishReason, ModelError, ToolSpec, Usage},
};
use reqwest::{Client, Method, Response, StatusCode, Url};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{error::TfError, sse::{SseEvent, SseParser}};

/// Base URL for an OpenAI-compatible API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseUrl(Url);

impl BaseUrl {
    /// Parse a base URL. HTTPS is required except for literal loopback hosts used by local models.
    /// User information, query strings, and fragments are rejected.
    ///
    /// # Errors
    /// Returns Configuration for invalid, insecure non-loopback, or credential-bearing URLs.
    pub fn parse(value: &str) -> Result<Self, TfError> {
        let mut url = Url::parse(value).map_err(|_| TfError::Configuration)?;
        let host = url.host_str().ok_or(TfError::Configuration)?;
        let loopback = host == "localhost" || is_literal_loopback_ip(host);
        if url.scheme() != "https" && !(url.scheme() == "http" && loopback) { return Err(TfError::Configuration); }
        if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
            return Err(TfError::Configuration);
        }
        if !url.path().ends_with('/') {
            let mut path = url.path().to_owned();
            path.push('/');
            url.set_path(&path);
        }
        Ok(Self(url))
    }

    fn endpoint(&self, endpoint: &str) -> Result<Url, TfError> {
        self.0.join(endpoint).map_err(|_| TfError::Configuration)
    }

    fn is_literal_loopback_ip(&self) -> bool {
        self.0.host_str().is_some_and(is_literal_loopback_ip)
    }
}

fn is_literal_loopback_ip(host: &str) -> bool {
    let address = match host.strip_prefix('[').and_then(|value| value.strip_suffix(']')) {
        Some(unbracketed) => unbracketed,
        None => host,
    };
    match address.parse::<IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => false,
    }
}

/// Opaque API credential. The original string is held in secrecy's zeroizing wrapper.
///
/// Construct this only from an injected SDK secret provider or an explicitly supplied secret;
/// this crate never reads environment variables or keyring state.
pub struct ApiKey(SecretString);

impl ApiKey {
    /// Wrap a credential resolved by the composition root.
    /// # Errors
    /// Returns Configuration for empty credentials or ASCII control characters.
    pub fn new(secret: SecretString) -> Result<Self, TfError> {
        let value = secret.expose_secret();
        if value.is_empty() || value.chars().any(|character| character.is_ascii_control()) { return Err(TfError::Configuration); }
        Ok(Self(secret))
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("ApiKey([REDACTED])") }
}

enum Credentials {
    Bearer(ApiKey),
    AnonymousLoopback,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bearer(_) => f.write_str("Bearer([REDACTED])"),
            Self::AnonymousLoopback => f.write_str("AnonymousLoopback"),
        }
    }
}

/// Explicit bounds and timeout policy for provider requests.
#[derive(Debug, Clone, Copy)]
pub struct ClientConfig {
    /// Total request duration applied by reqwest.
    pub request_timeout: Duration,
    /// Connection establishment timeout.
    pub connect_timeout: Duration,
    /// Maximum idle wait for each streamed body chunk.
    pub stream_idle_timeout: Duration,
    /// Maximum complete SSE frame in bytes.
    pub max_sse_frame_bytes: usize,
    /// Maximum complete SSE events accepted from one transport chunk.
    pub max_events_per_chunk: usize,
    /// Maximum aggregate chat-stream bytes.
    pub max_chat_stream_bytes: usize,
    /// Maximum UTF-8 content or reasoning bytes emitted for one response.
    pub max_text_bytes: usize,
    /// Maximum aggregate tool-argument bytes across one response.
    pub max_tool_argument_bytes: usize,
    /// Maximum response bytes for model-list and embedding JSON.
    pub max_json_response_bytes: usize,
    /// Maximum serialized request bytes.
    pub max_request_bytes: usize,
    /// Number of setup-time 429 retries. No retry occurs after a stream is returned.
    pub max_429_retries: u8,
    /// Exponential retry delay seed when Retry-After is absent.
    pub retry_base_delay: Duration,
    /// Upper bound for Retry-After and exponential backoff waits.
    pub max_retry_delay: Duration,
    /// Whether request history may replay provider reasoning in the reasoning-content field.
    /// This defaults by explicit caller choice because provider support is not guaranteed.
    pub reasoning_replay: ReasoningReplay,
}

/// Explicit provider policy for replaying isolated assistant reasoning history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningReplay {
    /// Do not send prior reasoning text.
    Disabled,
    /// Send prior reasoning using the reasoning_content field. Caller asserts endpoint support.
    ReasoningContent,
}

impl ClientConfig {
    /// Validate operational bounds before creating the HTTP client.
    /// # Errors
    /// Returns Configuration if a timeout or capacity is zero, or the retry cap exceeds its bound.
    pub fn validate(&self) -> Result<(), TfError> {
        if self.request_timeout.is_zero() || self.connect_timeout.is_zero() || self.stream_idle_timeout.is_zero()
            || self.max_sse_frame_bytes == 0 || self.max_events_per_chunk == 0 || self.max_chat_stream_bytes == 0
            || self.max_text_bytes == 0 || self.max_tool_argument_bytes == 0 || self.max_json_response_bytes == 0 || self.max_request_bytes == 0
            || self.retry_base_delay.is_zero() || self.max_retry_delay.is_zero() {
            return Err(TfError::Configuration);
        }
        if self.max_429_retries > 8 { return Err(TfError::Configuration); }
        Ok(())
    }
}

struct ClientInner {
    http: Client,
    base: BaseUrl,
    credentials: Credentials,
    config: ClientConfig,
}

/// OpenAI-compatible client. It has no implicit credentials or fallback backend.
#[derive(Clone)]
pub struct TokenFactoryClient {
    inner: Arc<ClientInner>,
}

impl TokenFactoryClient {
    /// Create a client with an explicitly injected credential and bounded HTTP policy.
    /// # Errors
    /// Returns Configuration for invalid settings or when reqwest cannot build its TLS client.
    pub fn new(base: BaseUrl, key: ApiKey, config: ClientConfig) -> Result<Self, TfError> {
        Self::build(base, Credentials::Bearer(key), config)
    }

    /// Create an anonymous client for an IP-literal loopback endpoint only.
    ///
    /// This supports local OpenAI-compatible servers which do not require authentication. It
    /// never sends an Authorization header or uses configured HTTP proxies. Hostnames such as
    /// `localhost`, private network IPs, and remote URLs are rejected even when the endpoint uses HTTPS.
    /// # Errors
    /// Returns Configuration unless the base URL host is the IPv4 or IPv6 loopback address.
    pub fn anonymous_loopback(base: BaseUrl, config: ClientConfig) -> Result<Self, TfError> {
        if !base.is_literal_loopback_ip() { return Err(TfError::Configuration); }
        Self::build(base, Credentials::AnonymousLoopback, config)
    }

    fn build(base: BaseUrl, credentials: Credentials, config: ClientConfig) -> Result<Self, TfError> {
        config.validate()?;
        let builder = Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .redirect(reqwest::redirect::Policy::none());
        let builder = if matches!(&credentials, Credentials::AnonymousLoopback) {
            builder.no_proxy()
        } else {
            builder
        };
        let http = builder.build().map_err(|_| TfError::Configuration)?;
        Ok(Self { inner: Arc::new(ClientInner { http, base, credentials, config }) })
    }

    /// Fetch available model identifiers from the OpenAI-compatible models endpoint.
    ///
    /// Model context sizes, modalities, and availability are intentionally omitted because they
    /// are not guaranteed by this endpoint and the recorded live catalog is not bundled here.
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, TfError> {
        let url = self.inner.base.endpoint("models")?;
        let response = self.send_with_429_retry(Method::GET, url, None).await?;
        let bytes = read_bounded(response, self.inner.config.max_json_response_bytes, self.inner.config.stream_idle_timeout).await?;
        let result: ModelListWire = serde_json::from_slice(&bytes).map_err(|_| TfError::Json)?;
        let mut seen = BTreeSet::new();
        let mut models = Vec::with_capacity(result.data.len());
        for item in result.data {
            if item.id.is_empty() || item.id.chars().any(char::is_control) || !seen.insert(item.id.clone()) {
                return Err(TfError::Protocol);
            }
            models.push(ModelInfo { id: item.id });
        }
        Ok(models)
    }

    /// Request a batch of embeddings. The response's actual vector dimension is returned and
    /// checked for consistency; no dimension is assumed from a model name.
    pub async fn embeddings(&self, model: &ModelId, batch: &[&str], config: &EmbeddingConfig) -> Result<EmbeddingBatch, TfError> {
        config.validate()?;
        if batch.is_empty() || batch.len() > config.max_batch_items || batch.iter().any(|text| text.len() > config.max_input_bytes) {
            return Err(TfError::Configuration);
        }
        let body = json!({"model": model.as_str(), "input": batch});
        if serde_json::to_vec(&body).map_err(|_| TfError::Json)?.len() > self.inner.config.max_request_bytes { return Err(TfError::TooLarge); }
        let response = self.send_with_429_retry(Method::POST, self.inner.base.endpoint("embeddings")?, Some(body)).await?;
        let bytes = read_bounded(response, self.inner.config.max_json_response_bytes, self.inner.config.stream_idle_timeout).await?;
        let parsed: EmbeddingResponseWire = serde_json::from_slice(&bytes).map_err(|_| TfError::Json)?;
        if parsed.data.len() != batch.len() { return Err(TfError::Protocol); }
        let mut vectors = BTreeMap::new();
        for item in parsed.data {
            if item.index >= batch.len() || vectors.insert(item.index, item.embedding).is_some() { return Err(TfError::Protocol); }
        }
        let mut ordered = Vec::with_capacity(batch.len());
        for expected_index in 0..batch.len() {
            let Some(vector) = vectors.remove(&expected_index) else { return Err(TfError::Protocol); };
            if vector.is_empty() || vector.len() > config.max_dimensions || vector.iter().any(|value| !value.is_finite()) {
                return Err(TfError::Protocol);
            }
            ordered.push(vector);
        }
        let dimension = ordered.first().map(Vec::len).ok_or(TfError::Protocol)?;
        if ordered.iter().any(|vector| vector.len() != dimension) { return Err(TfError::Protocol); }
        Ok(EmbeddingBatch { vectors: ordered, dimension })
    }

    /// Create an SDK embedding adapter that checks each real response against an explicitly
    /// configured dimension observed or otherwise verified by the composition root.
    /// # Errors
    /// Returns Configuration for a zero or unsupported expected dimension.
    pub fn embedding_provider(&self, model: ModelId, config: EmbeddingConfig, expected_dimension: usize) -> Result<EmbeddingClient, TfError> {
        config.validate()?;
        if expected_dimension == 0 || expected_dimension > config.max_dimensions { return Err(TfError::Configuration); }
        Ok(EmbeddingClient { client: self.clone(), model, config, expected_dimension })
    }

    async fn send_with_429_retry(&self, method: Method, url: Url, body: Option<Value>) -> Result<Response, TfError> {
        let mut retries = 0_u8;
        let start = tokio::time::Instant::now();
        let Some(deadline) = start.checked_add(self.inner.config.request_timeout) else { return Err(TfError::Configuration); };
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() { return Err(TfError::Timeout); }
            let mut request = self.inner.http.request(method.clone(), url.clone());
            if let Credentials::Bearer(key) = &self.inner.credentials {
                request = request.bearer_auth(key.0.expose_secret());
            }
            if let Some(json_body) = body.as_ref() { request = request.json(json_body); }
            let response = match tokio::time::timeout(remaining, request.send()).await {
                Err(_) => return Err(TfError::Timeout),
                Ok(Err(_)) => return Err(TfError::Transport),
                Ok(Ok(response)) => response,
            };
            let status = response.status();
            if status == StatusCode::TOO_MANY_REQUESTS && retries < self.inner.config.max_429_retries {
                let delay = retry_delay(response.headers().get(reqwest::header::RETRY_AFTER), retries, self.inner.config);
                if delay >= deadline.saturating_duration_since(tokio::time::Instant::now()) {
                    return Err(status_error(response));
                }
                retries = retries.saturating_add(1);
                tokio::time::sleep(delay).await;
                continue;
            }
            if !status.is_success() { return Err(status_error(response)); }
            return Ok(response);
        }
    }
}

impl std::fmt::Debug for TokenFactoryClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenFactoryClient").field("base", &self.inner.base)
            .field("credential", &self.inner.credentials).finish()
    }
}

impl ChatModel for TokenFactoryClient {
    type Stream = ProviderStream;

    fn stream(&self, req: ChatRequest) -> impl Future<Output = Result<Self::Stream, ModelError>> + Send {
        async move {
            let request_body = chat_request_json(&req, self.inner.config.reasoning_replay).map_err(ModelError::from)?;
            if serde_json::to_vec(&request_body).map_err(|_| TfError::Json).map_err(ModelError::from)?.len() > self.inner.config.max_request_bytes {
                return Err(ModelError::from(TfError::TooLarge));
            }
            let url = self.inner.base.endpoint("chat/completions").map_err(ModelError::from)?;
            let response = self.send_with_429_retry(Method::POST, url, Some(request_body)).await.map_err(ModelError::from)?;
            let body: BodyStream = Box::pin(response.bytes_stream());
            let parser = SseParser::new(self.inner.config.max_sse_frame_bytes, self.inner.config.max_events_per_chunk)
                .map_err(ModelError::from)?;
            Ok(ProviderStream {
                body, parser, pending: VecDeque::new(), config: self.inner.config,
                raw_bytes: 0, text_bytes: 0, tool_bytes: 0, tools: BTreeMap::new(),
                finish_reason: None, saw_done: false, terminal: false, eof_flushed: false,
            })
        }
    }
}

/// One model id from the provider's models endpoint. No unverified capabilities are attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    /// Provider model identifier.
    pub id: String,
}

#[derive(Deserialize)]
struct ModelListWire { data: Vec<ModelListEntryWire> }
#[derive(Deserialize)]
struct ModelListEntryWire { id: String }

#[derive(Deserialize)]
struct EmbeddingResponseWire { data: Vec<EmbeddingItemWire> }
#[derive(Deserialize)]
struct EmbeddingItemWire { index: usize, embedding: Vec<f32> }

/// Actual embedding response batch and dimension.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingBatch {
    /// Vectors ordered to match the input batch.
    pub vectors: Vec<Vec<f32>>,
    /// Dimension observed in the response.
    pub dimension: usize,
}

/// Limits for one embedding request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingConfig {
    /// Maximum input strings per request.
    pub max_batch_items: usize,
    /// Maximum UTF-8 bytes in each input string.
    pub max_input_bytes: usize,
    /// Maximum vector dimension accepted from a response.
    pub max_dimensions: usize,
}

impl EmbeddingConfig {
    /// Reject empty input or dimension bounds.
    /// # Errors
    /// Returns Configuration if any bound is zero.
    pub fn validate(&self) -> Result<(), TfError> {
        if self.max_batch_items == 0 || self.max_input_bytes == 0 || self.max_dimensions == 0 {
            return Err(TfError::Configuration);
        }
        Ok(())
    }
}

/// SDK EmbeddingProvider with an explicit expected dimension.
#[derive(Clone)]
pub struct EmbeddingClient {
    client: TokenFactoryClient,
    model: ModelId,
    config: EmbeddingConfig,
    expected_dimension: usize,
}

impl std::fmt::Debug for EmbeddingClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddingClient").field("model", &self.model).field("expected_dimension", &self.expected_dimension).finish()
    }
}

impl EmbeddingClient {
    /// Fetch a batch and return the dimension observed in the provider response.
    pub async fn embeddings(&self, batch: &[&str]) -> Result<EmbeddingBatch, TfError> {
        self.client.embeddings(&self.model, batch, &self.config).await
    }
}

impl EmbeddingProvider for EmbeddingClient {
    fn dimension(&self) -> usize { self.expected_dimension }

    fn embed(&self, batch: &[&str]) -> impl Future<Output = Result<Vec<Vec<f32>>, ModelError>> + Send {
        async move {
            let result = self.embeddings(batch).await.map_err(ModelError::from)?;
            if result.dimension != self.expected_dimension { return Err(ModelError::Protocol("embedding dimension mismatch".to_owned())); }
            Ok(result.vectors)
        }
    }
}

type BodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

/// Pull-based SDK stream for one completion.
pub struct ProviderStream {
    body: BodyStream,
    parser: SseParser,
    pending: VecDeque<Result<ChatChunk, ModelError>>,
    config: ClientConfig,
    raw_bytes: usize,
    text_bytes: usize,
    tool_bytes: usize,
    tools: BTreeMap<u32, ToolCallState>,
    finish_reason: Option<FinishReason>,
    saw_done: bool,
    terminal: bool,
    eof_flushed: bool,
}

impl ChatChunkSource for ProviderStream {
    fn next_chunk(&mut self) -> impl Future<Output = Option<Result<ChatChunk, ModelError>>> + Send {
        async move {
            loop {
                if let Some(chunk) = self.pending.pop_front() { return Some(chunk); }
                if self.terminal { return None; }
                let next = tokio::time::timeout(self.config.stream_idle_timeout, self.body.next()).await;
                match next {
                    Err(_) => return self.fail(TfError::Timeout),
                    Ok(Some(Err(_))) => return self.fail(TfError::Transport),
                    Ok(Some(Ok(bytes))) => {
                        self.raw_bytes = match self.raw_bytes.checked_add(bytes.len()) {
                            Some(total) if total <= self.config.max_chat_stream_bytes => total,
                            _ => return self.fail(TfError::TooLarge),
                        };
                        let mut events = Vec::new();
                        if let Err(error) = self.parser.feed(&bytes, &mut events) { return self.fail(error); }
                        for event in events {
                            if let Err(error) = self.consume_event(event) { return self.fail(error); }
                        }
                    }
                    Ok(None) => {
                        if !self.eof_flushed {
                            let mut events = Vec::new();
                            if let Err(error) = self.parser.finish(&mut events) { return self.fail(error); }
                            self.eof_flushed = true;
                            for event in events {
                                if let Err(error) = self.consume_event(event) { return self.fail(error); }
                            }
                        }
                        if !self.saw_done || self.finish_reason.is_none() { return self.fail(TfError::StreamTruncated); }
                        if let Err(error) = self.validate_tool_calls() { return self.fail(error); }
                        self.terminal = true;
                        let Some(reason) = self.finish_reason else { return self.fail(TfError::StreamTruncated); };
                        return Some(Ok(ChatChunk::Finished(reason)));
                    }
                }
            }
        }
    }
}

impl ProviderStream {
    fn fail(&mut self, error: TfError) -> Option<Result<ChatChunk, ModelError>> {
        self.terminal = true;
        self.pending.clear();
        Some(Err(ModelError::from(error)))
    }

    fn consume_event(&mut self, event: SseEvent) -> Result<(), TfError> {
        if self.saw_done { return Err(TfError::Protocol); }
        match event {
            SseEvent::Done => {
                if self.finish_reason.is_none() { return Err(TfError::StreamTruncated); }
                self.validate_tool_calls()?;
                self.saw_done = true;
            }
            SseEvent::Data(data) => {
                let chunk: ChatCompletionChunk = serde_json::from_str(&data).map_err(|_| TfError::Json)?;
                if chunk.choices.len() > 1 { return Err(TfError::Protocol); }
                if let Some(usage) = chunk.usage { self.pending.push_back(Ok(ChatChunk::Usage(usage.into()))); }
                for choice in chunk.choices {
                    if choice.index != 0 { return Err(TfError::Protocol); }
                    let has_delta = choice.delta.content.is_some() || choice.delta.reasoning_content.is_some()
                        || !choice.delta.tool_calls.is_empty();
                    if self.finish_reason.is_some() && has_delta { return Err(TfError::Protocol); }
                    if let Some(content) = choice.delta.content {
                        self.add_text_bytes(content.len())?;
                        if !content.is_empty() { self.pending.push_back(Ok(ChatChunk::Text(content))); }
                    }
                    if let Some(reasoning) = choice.delta.reasoning_content {
                        self.add_text_bytes(reasoning.len())?;
                        if !reasoning.is_empty() { self.pending.push_back(Ok(ChatChunk::Reasoning(reasoning))); }
                    }
                    for delta in choice.delta.tool_calls { self.add_tool_delta(delta)?; }
                    if let Some(reason) = choice.finish_reason {
                        let mapped = parse_finish_reason(&reason);
                        if self.finish_reason.is_some_and(|existing| existing != mapped) { return Err(TfError::Protocol); }
                        self.finish_reason = Some(mapped);
                    }
                }
            }
        }
        Ok(())
    }

    fn add_text_bytes(&mut self, bytes: usize) -> Result<(), TfError> {
        self.text_bytes = self.text_bytes.checked_add(bytes).ok_or(TfError::TooLarge)?;
        if self.text_bytes > self.config.max_text_bytes { return Err(TfError::TooLarge); }
        Ok(())
    }

    fn add_tool_delta(&mut self, delta: ToolCallDeltaWire) -> Result<(), TfError> {
        self.tool_bytes = self.tool_bytes.checked_add(delta.function.arguments.len()).ok_or(TfError::TooLarge)?;
        if self.tool_bytes > self.config.max_tool_argument_bytes { return Err(TfError::TooLarge); }
        let state = self.tools.entry(delta.index).or_default();
        set_consistent(&mut state.id, delta.id.clone())?;
        set_consistent(&mut state.name, delta.function.name.clone())?;
        state.arguments.push_str(&delta.function.arguments);
        self.pending.push_back(Ok(ChatChunk::ToolCallDelta {
            index: delta.index,
            id: delta.id,
            name: delta.function.name,
            arguments_fragment: delta.function.arguments,
        }));
        Ok(())
    }

    fn validate_tool_calls(&self) -> Result<(), TfError> {
        let mut seen_ids = BTreeSet::new();
        for call in self.tools.values() {
            let id = call.id.as_deref().filter(|id| !id.is_empty()).ok_or(TfError::StreamTruncated)?;
            let name = call.name.as_deref().filter(|name| !name.is_empty())
                .ok_or(TfError::StreamTruncated)?;
            if !seen_ids.insert(id) || !name.chars().all(|character| character.is_ascii_alphanumeric() || character == '_') {
                return Err(TfError::Protocol);
            }
            let args: Value = serde_json::from_str(&call.arguments).map_err(|_| TfError::StreamTruncated)?;
            if !args.is_object() { return Err(TfError::StreamTruncated); }
        }
        Ok(())
    }
}

#[derive(Default)]
struct ToolCallState { id: Option<String>, name: Option<String>, arguments: String }

fn set_consistent(target: &mut Option<String>, update: Option<String>) -> Result<(), TfError> {
    if let Some(update) = update {
        if let Some(existing) = target.as_ref() {
            if existing != &update { return Err(TfError::Protocol); }
        } else { *target = Some(update); }
    }
    Ok(())
}

#[derive(Deserialize)]
struct ChatCompletionChunk { #[serde(default)] choices: Vec<ChoiceWire>, usage: Option<UsageWire> }
#[derive(Deserialize)]
struct ChoiceWire { #[serde(default)] index: u32, #[serde(default)] delta: DeltaWire, finish_reason: Option<String> }
#[derive(Deserialize, Default)]
struct DeltaWire {
    content: Option<String>,
    reasoning_content: Option<String>,
    #[serde(default)] tool_calls: Vec<ToolCallDeltaWire>,
}
#[derive(Deserialize)]
struct ToolCallDeltaWire { index: u32, id: Option<String>, function: FunctionDeltaWire }
#[derive(Deserialize)]
struct FunctionDeltaWire { name: Option<String>, #[serde(default)] arguments: String }
#[derive(Deserialize)]
struct UsageWire {
    prompt_tokens: u32,
    completion_tokens: u32,
    #[serde(default)] prompt_tokens_details: Option<PromptTokenDetailsWire>,
}
#[derive(Deserialize)]
struct PromptTokenDetailsWire { cached_tokens: Option<u32> }

impl From<UsageWire> for Usage {
    fn from(value: UsageWire) -> Self {
        let cached = value.prompt_tokens_details.and_then(|details| details.cached_tokens).map_or(0, |tokens| tokens);
        Self {
            prompt_tokens: TokenCount::new(value.prompt_tokens),
            completion_tokens: TokenCount::new(value.completion_tokens),
            cached_tokens: TokenCount::new(cached),
        }
    }
}

fn parse_finish_reason(value: &str) -> FinishReason {
    match value {
        "stop" => FinishReason::Stop,
        "length" => FinishReason::Length,
        "tool_calls" => FinishReason::ToolCalls,
        "content_filter" => FinishReason::ContentFilter,
        _ => FinishReason::Other,
    }
}

fn chat_request_json(req: &ChatRequest, reasoning_replay: ReasoningReplay) -> Result<Value, TfError> {
    validate_history(&req.messages)?;
    let messages: Vec<Value> = req.messages.iter().map(|message| {
        let mut wire = serde_json::Map::new();
        let role = match message.role {
            ChatRole::System => "system", ChatRole::User => "user", ChatRole::Assistant => "assistant", ChatRole::Tool => "tool",
        };
        wire.insert("role".to_owned(), json!(role));
        wire.insert("content".to_owned(), json!(message.content));
        if let Some(call_id) = &message.tool_call_id { wire.insert("tool_call_id".to_owned(), json!(call_id)); }
        if !message.tool_calls.is_empty() {
            let calls: Vec<Value> = message.tool_calls.iter().map(tool_call_message_json).collect();
            wire.insert("tool_calls".to_owned(), Value::Array(calls));
        }
        if reasoning_replay == ReasoningReplay::ReasoningContent {
            if let Some(reasoning) = &message.reasoning { wire.insert("reasoning_content".to_owned(), json!(reasoning)); }
        }
        Value::Object(wire)
    }).collect();
    let mut body = serde_json::Map::new();
    body.insert("model".to_owned(), json!(req.model.as_str()));
    body.insert("messages".to_owned(), Value::Array(messages));
    body.insert("stream".to_owned(), json!(true));
    body.insert("n".to_owned(), json!(1));
    if let Some(max_tokens) = req.max_output_tokens { body.insert("max_tokens".to_owned(), json!(max_tokens.get())); }
    if !req.tools.is_empty() {
        let mut names = BTreeSet::new();
        if req.tools.iter().any(|tool| !names.insert(tool.name.as_ref())) { return Err(TfError::BadRequest); }
        let tools = req.tools.iter().map(tool_json).collect::<Result<Vec<_>, _>>()?;
        body.insert("tools".to_owned(), Value::Array(tools));
    }
    // The SDK carries effort, but the provider-specific reasoning control is unverified (S0-5).
    // Do not invent a reasoning_effort or budget field.
    let _requested_effort = req.effort;
    Ok(Value::Object(body))
}

fn validate_history(messages: &[polaris_sdk::registry::ChatMessage]) -> Result<(), TfError> {
    let mut known_calls = BTreeSet::new();
    let mut outstanding_calls = BTreeSet::new();
    for message in messages {
        if message.reasoning.is_some() && message.role != ChatRole::Assistant { return Err(TfError::BadRequest); }
        if !outstanding_calls.is_empty() && message.role != ChatRole::Tool { return Err(TfError::BadRequest); }
        match message.role {
            ChatRole::Assistant => {
                if message.tool_call_id.is_some() { return Err(TfError::BadRequest); }
                for call in &message.tool_calls {
                    if call.id.is_empty() || call.name.is_empty() || !known_calls.insert(call.id.clone())
                        || !outstanding_calls.insert(call.id.clone()) { return Err(TfError::BadRequest); }
                    let args: Value = serde_json::from_str(&call.arguments_json).map_err(|_| TfError::BadRequest)?;
                    if !args.is_object() { return Err(TfError::BadRequest); }
                }
            }
            ChatRole::Tool => {
                if !message.tool_calls.is_empty() { return Err(TfError::BadRequest); }
                let Some(call_id) = message.tool_call_id.as_ref() else { return Err(TfError::BadRequest); };
                if !outstanding_calls.remove(call_id) { return Err(TfError::BadRequest); }
            }
            ChatRole::System | ChatRole::User => {
                if !message.tool_calls.is_empty() || message.tool_call_id.is_some() { return Err(TfError::BadRequest); }
            }
        }
    }
    if !outstanding_calls.is_empty() { return Err(TfError::BadRequest); }
    Ok(())
}

fn tool_call_message_json(call: &ChatToolCall) -> Value {
    json!({"id": call.id, "type": "function", "function": {"name": call.name, "arguments": call.arguments_json}})
}

fn tool_json(tool: &ToolSpec) -> Result<Value, TfError> {
    let parameters: Value = serde_json::from_str(&tool.input_schema_json).map_err(|_| TfError::BadRequest)?;
    if tool.name.is_empty() || !tool.name.chars().all(|character| character.is_ascii_alphanumeric() || character == '_')
        || !parameters.is_object() { return Err(TfError::BadRequest); }
    Ok(json!({"type": "function", "function": {
        "name": tool.name, "description": tool.description, "parameters": parameters
    }}))
}

async fn read_bounded(response: Response, maximum: usize, idle_timeout: Duration) -> Result<Bytes, TfError> {
    let mut stream = Box::pin(response.bytes_stream());
    let mut buffer = BytesMut::new();
    loop {
        match tokio::time::timeout(idle_timeout, stream.next()).await {
            Err(_) => return Err(TfError::Timeout),
            Ok(None) => return Ok(buffer.freeze()),
            Ok(Some(Err(_))) => return Err(TfError::Transport),
            Ok(Some(Ok(bytes))) => {
                let length = buffer.len().checked_add(bytes.len()).ok_or(TfError::TooLarge)?;
                if length > maximum { return Err(TfError::TooLarge); }
                buffer.extend_from_slice(&bytes);
            }
        }
    }
}

fn status_error(response: Response) -> TfError {
    let status = response.status();
        let retry_after_ms = response.headers().get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .and_then(|seconds| seconds.checked_mul(1_000));
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => TfError::Unauthorized,
        StatusCode::BAD_REQUEST | StatusCode::PAYLOAD_TOO_LARGE | StatusCode::UNPROCESSABLE_ENTITY => TfError::BadRequest,
        _ => TfError::Http { status: status.as_u16(), retry_after_ms },
    }
}

fn retry_delay(header: Option<&reqwest::header::HeaderValue>, retry: u8, config: ClientConfig) -> Duration {
    let server = header.and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .and_then(|seconds| seconds.checked_mul(1_000))
        .map(Duration::from_millis);
    let multiplier = 1_u32.checked_shl(u32::from(retry)).map_or(u32::MAX, |value| value);
    let exponential = config.retry_base_delay.saturating_mul(multiplier);
    match server { Some(delay) => delay, None => exponential }.min(config.max_retry_delay)
}
