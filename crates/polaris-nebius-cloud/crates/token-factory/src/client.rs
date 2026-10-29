//! OpenAI-compatible HTTP adapter and SDK trait implementations.

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, VecDeque},
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt};
use polaris_ratelimit::{DualLimiter, Estimate, MonotonicClock, Permit, Priority, RateLimitError};
use polaris_sdk::{
    ids::{ModelId, TokenCount},
    registry::{
        ChatChunk, ChatChunkSource, ChatModel, ChatRequest, ChatRole, ChatToolCall,
        EmbeddingProvider, FinishReason, ModelError, PromptTokenCount, PromptTokenizer, ToolSpec,
        Usage,
    },
};
use rand::RngExt;
use reqwest::{Client, Method, Response, StatusCode, Url};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    error::TfError,
    sse::{SseEventRef, SseParser},
};

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
        if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
            return Err(TfError::Configuration);
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(TfError::Configuration);
        }
        if !url.path().ends_with('/') {
            let mut path = url.path().to_owned();
            path.push('/');
            url.set_path(&path);
        }
        Ok(Self(url))
    }

    /// Return the numeric loopback socket authority with an explicit port.
    ///
    /// This deliberately rejects DNS names (including `localhost`) so process supervision can
    /// prove the configured endpoint targets the owned local listener without DNS resolution.
    /// # Errors
    /// Returns Configuration unless the URL has an explicit port and literal loopback IP host.
    pub fn literal_loopback_socket_addr(&self) -> Result<SocketAddr, TfError> {
        let host = self.0.host().ok_or(TfError::Configuration)?;
        let ip = match host {
            url::Host::Ipv4(address) => IpAddr::V4(address),
            url::Host::Ipv6(address) => IpAddr::V6(address),
            url::Host::Domain(_) => return Err(TfError::Configuration),
        };
        if !ip.is_loopback() {
            return Err(TfError::Configuration);
        }
        let port = self.0.port().ok_or(TfError::Configuration)?;
        if port == 0 {
            return Err(TfError::Configuration);
        }
        Ok(SocketAddr::new(ip, port))
    }

    fn endpoint(&self, endpoint: &str) -> Result<Url, TfError> {
        self.0.join(endpoint).map_err(|_| TfError::Configuration)
    }

    fn origin_endpoint(&self, endpoint: &str) -> Result<Url, TfError> {
        let mut origin_root = self.0.clone();
        origin_root.set_path("/");
        origin_root
            .join(endpoint)
            .map_err(|_| TfError::Configuration)
    }

    fn is_literal_loopback_ip(&self) -> bool {
        self.0.host_str().is_some_and(is_literal_loopback_ip)
    }
}

fn is_literal_loopback_ip(host: &str) -> bool {
    let address = match host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
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
    /// Resolve exactly one explicitly configured SDK location into an opaque API credential.
    /// No environment, keyring, anonymous, or alternative-location fallback is performed here.
    ///
    /// This synchronous source read belongs on a blocking initialization task when used by an
    /// asynchronous composition root; the HTTP adapter itself never reads host credential state.
    /// # Errors
    /// Returns a typed provider failure, missing value, or invalid credential error.
    pub fn from_source(
        source: &(impl polaris_sdk::secrets::SecretSource + ?Sized),
        location: &polaris_sdk::secrets::SecretLocation,
    ) -> Result<Self, crate::CredentialError> {
        let secret = source
            .load(location)?
            .ok_or(crate::CredentialError::Missing)?;
        Self::new(secret).map_err(|_| crate::CredentialError::Invalid)
    }

    /// Wrap a credential resolved by the composition root.
    /// # Errors
    /// Returns Configuration for empty credentials or ASCII control characters.
    pub fn new(secret: SecretString) -> Result<Self, TfError> {
        let value = secret.expose_secret();
        if value.is_empty() || value.chars().any(|character| character.is_ascii_control()) {
            return Err(TfError::Configuration);
        }
        Ok(Self(secret))
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey([REDACTED])")
    }
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
    /// Maximum number of tool calls accepted in one request or response.
    pub max_tool_calls: usize,
    /// Maximum response bytes for model-list, embedding, and vLLM `/tokenize` JSON, including its
    /// full token-ID array. Callers must size this for the largest prompt they intend to count.
    pub max_json_response_bytes: usize,
    /// Maximum serialized request bytes.
    pub max_request_bytes: usize,
    /// Number of setup-time 429 retries. No retry occurs after a stream is returned.
    pub max_429_retries: u8,
    /// Number of setup-time 5xx retries. No retry occurs after a stream is returned.
    pub max_5xx_retries: u8,
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

/// Injected conservative prompt-token estimator used only when a local dual limiter is enabled.
pub trait PromptTokenEstimator: Send + Sync + 'static {
    /// Return an upper-bound token estimate for the assembled prompt and tool definitions.
    fn estimate_prompt_tokens(&self, request: &ChatRequest) -> Result<TokenCount, TfError>;
}

impl<F> PromptTokenEstimator for F
where
    F: Fn(&ChatRequest) -> Result<TokenCount, TfError> + Send + Sync + 'static,
{
    fn estimate_prompt_tokens(&self, request: &ChatRequest) -> Result<TokenCount, TfError> {
        self(request)
    }
}

/// Explicit local admission policy. Quotas are caller-supplied and are never inferred from
/// provider headers. Clones share the limiter and estimator.
#[derive(Clone)]
pub struct RateLimitPolicy {
    limiter: DualLimiter<MonotonicClock>,
    estimator: Arc<dyn PromptTokenEstimator>,
    priority: Priority,
}

impl RateLimitPolicy {
    /// Build a policy from a caller-configured limiter and trusted prompt estimator.
    #[must_use]
    pub fn new(
        limiter: DualLimiter<MonotonicClock>,
        estimator: impl PromptTokenEstimator,
        priority: Priority,
    ) -> Self {
        Self {
            limiter,
            estimator: Arc::new(estimator),
            priority,
        }
    }

    /// Read the shared local request and token window ledger.
    #[must_use]
    pub fn window(&self) -> polaris_ratelimit::WindowSnapshot {
        self.limiter.window()
    }
}

impl ClientConfig {
    /// Validate operational bounds before creating the HTTP client.
    /// # Errors
    /// Returns Configuration if a timeout or capacity is zero, or the retry cap exceeds its bound.
    pub fn validate(&self) -> Result<(), TfError> {
        if self.request_timeout.is_zero()
            || self.connect_timeout.is_zero()
            || self.stream_idle_timeout.is_zero()
            || self.max_sse_frame_bytes == 0
            || self.max_events_per_chunk == 0
            || self.max_chat_stream_bytes == 0
            || self.max_text_bytes == 0
            || self.max_tool_argument_bytes == 0
            || self.max_tool_calls == 0
            || self.max_json_response_bytes == 0
            || self.max_request_bytes == 0
            || self.retry_base_delay.is_zero()
            || self.max_retry_delay.is_zero()
        {
            return Err(TfError::Configuration);
        }
        if self.max_429_retries > 8 {
            return Err(TfError::Configuration);
        }
        if self.max_5xx_retries > 8 {
            return Err(TfError::Configuration);
        }
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
    rate_limit: Option<Arc<RateLimitPolicy>>,
}

/// Explicit vLLM-compatible chat wrapper with provider-backed prompt tokenization.
///
/// Construction opts into the same-origin `/tokenize` endpoint. Token Factory deployments do
/// not necessarily expose this vLLM endpoint; unsupported endpoints return their HTTP error.
pub struct VllmChatClient {
    client: TokenFactoryClient,
}

/// Startup-pinned llama.cpp server identity required for exact local chat counting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlamaCppExpectedProfile {
    build_info: String,
    model_alias: String,
    max_context_tokens: TokenCount,
}

impl LlamaCppExpectedProfile {
    /// Build an expected server profile; every field must identify one concrete loaded model.
    /// # Errors
    /// Returns Configuration when a profile field is empty or context is zero.
    pub fn new(
        build_info: impl Into<String>,
        model_alias: impl Into<String>,
        max_context_tokens: TokenCount,
    ) -> Result<Self, TfError> {
        let build_info = build_info.into();
        let model_alias = model_alias.into();
        if build_info.is_empty() || model_alias.is_empty() || max_context_tokens.get() == 0 {
            return Err(TfError::Configuration);
        }
        Ok(Self {
            build_info,
            model_alias,
            max_context_tokens,
        })
    }

    /// Pinned llama.cpp build identifier, including release and commit.
    #[must_use]
    pub fn build_info(&self) -> &str {
        &self.build_info
    }

    /// Pinned model alias exposed by both `/props` and `/v1/models`.
    #[must_use]
    pub fn model_alias(&self) -> &str {
        &self.model_alias
    }

    /// Validated model context capacity.
    #[must_use]
    pub const fn max_context_tokens(&self) -> TokenCount {
        self.max_context_tokens
    }
}

/// Exact-tokenized chat client for a pinned llama.cpp loopback server.
pub struct LlamaCppChatClient {
    client: TokenFactoryClient,
    profile: LlamaCppExpectedProfile,
}

impl std::fmt::Debug for LlamaCppChatClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlamaCppChatClient")
            .field("profile", &self.profile)
            .field("client", &"[REDACTED]")
            .finish()
    }
}

impl LlamaCppChatClient {
    /// Connect to a literal loopback server and validate its pinned build/model/template profile.
    /// # Errors
    /// Returns a typed model error if the local endpoint is unavailable or mismatches the profile.
    pub async fn connect(
        base: BaseUrl,
        profile: LlamaCppExpectedProfile,
        config: ClientConfig,
    ) -> Result<Self, ModelError> {
        if base.0.scheme() != "http" || base.0.path() != "/v1/" {
            return Err(ModelError::from(TfError::Configuration));
        }
        let client =
            TokenFactoryClient::anonymous_loopback(base, config).map_err(ModelError::from)?;
        client.validate_llama_cpp_profile(&profile).await?;
        Ok(Self { client, profile })
    }

    /// The build, alias, and context verified against the live server at construction.
    #[must_use]
    pub const fn profile(&self) -> &LlamaCppExpectedProfile {
        &self.profile
    }
}

impl ChatModel for LlamaCppChatClient {
    type Stream = ProviderStream;

    async fn stream(&self, req: ChatRequest) -> Result<Self::Stream, ModelError> {
        if req.tools.len() > self.client.inner.config.max_tool_calls
            || chat_request_input_bound(&req)? > self.client.inner.config.max_request_bytes
        {
            return Err(ModelError::from(TfError::TooLarge));
        }
        let request_body = llama_chat_request_json(
            &req,
            self.client.inner.config.reasoning_replay,
            &self.profile.model_alias,
        )?;
        self.client
            .start_stream_with_body(req, StreamProtocol::ChatCompletions, request_body)
            .await
    }
}

impl PromptTokenizer for LlamaCppChatClient {
    async fn count_prompt(&self, request: &ChatRequest) -> Result<PromptTokenCount, ModelError> {
        self.client.count_llama_prompt(request, &self.profile).await
    }
}

impl VllmChatClient {
    /// Wrap a configured OpenAI-compatible client and opt into its same-origin `/tokenize` route.
    #[must_use]
    pub fn new(client: TokenFactoryClient) -> Self {
        Self { client }
    }
}

impl std::fmt::Debug for VllmChatClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VllmChatClient")
            .field("client", &"[REDACTED]")
            .finish()
    }
}

impl ChatModel for VllmChatClient {
    type Stream = ProviderStream;

    async fn stream(&self, req: ChatRequest) -> Result<Self::Stream, ModelError> {
        self.client.stream(req).await
    }
}

impl PromptTokenizer for VllmChatClient {
    async fn count_prompt(&self, request: &ChatRequest) -> Result<PromptTokenCount, ModelError> {
        self.client.count_prompt(request).await
    }
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
        if !base.is_literal_loopback_ip() {
            return Err(TfError::Configuration);
        }
        Self::build(base, Credentials::AnonymousLoopback, config)
    }

    /// Attach explicit local request admission and usage reconciliation.
    #[must_use]
    pub fn with_rate_limit(mut self, policy: RateLimitPolicy) -> Self {
        self.rate_limit = Some(Arc::new(policy));
        self
    }

    /// Start a streaming request against the OpenAI Responses endpoint.
    ///
    /// This uses the documented common text, reasoning, function-call, terminal, and usage event
    /// subset. Token Factory or a local endpoint may not implement this API; compatibility must
    /// be verified against that provider before relying on it.
    /// # Errors
    /// Returns the SDK model error for invalid requests, HTTP errors, or malformed streams.
    pub async fn stream_responses(&self, req: ChatRequest) -> Result<ProviderStream, ModelError> {
        self.start_stream(req, StreamProtocol::Responses).await
    }

    async fn count_prompt(&self, request: &ChatRequest) -> Result<PromptTokenCount, ModelError> {
        if request.tools.len() > self.inner.config.max_tool_calls {
            return Err(ModelError::from(TfError::TooLarge));
        }
        if chat_request_input_bound(request)? > self.inner.config.max_request_bytes {
            return Err(ModelError::from(TfError::TooLarge));
        }
        let mut body = chat_request_json(request, self.inner.config.reasoning_replay)?;
        let body_object = body.as_object_mut().ok_or(TfError::Protocol)?;
        drop(body_object.remove("stream"));
        drop(body_object.remove("n"));
        drop(body_object.remove("max_tokens"));
        body_object.insert("add_generation_prompt".to_owned(), json!(true));
        body_object.insert("continue_final_message".to_owned(), json!(false));
        body_object.insert("add_special_tokens".to_owned(), json!(false));
        body_object.insert("return_token_strs".to_owned(), json!(false));
        if serde_json::to_vec(&body).map_err(|_| TfError::Json)?.len()
            > self.inner.config.max_request_bytes
        {
            return Err(ModelError::from(TfError::TooLarge));
        }
        let url = self
            .inner
            .base
            .origin_endpoint("tokenize")
            .map_err(ModelError::from)?;
        let response = self
            .send_with_429_retry(Method::POST, url, Some(body))
            .await
            .map_err(ModelError::from)?;
        let bytes = read_bounded(
            response,
            self.inner.config.max_json_response_bytes,
            self.inner.config.stream_idle_timeout,
        )
        .await
        .map_err(ModelError::from)?;
        let response: TokenizeResponseWire =
            serde_json::from_slice(&bytes).map_err(|_| ModelError::from(TfError::Protocol))?;
        let token_length = u64::try_from(response.tokens.len())
            .map_err(|_| ModelError::from(TfError::Protocol))?;
        if response.count == 0 || response.max_model_len == 0 || response.count != token_length {
            return Err(ModelError::from(TfError::Protocol));
        }
        let input_tokens = u32::try_from(response.count)
            .map(TokenCount::new)
            .map_err(|_| ModelError::from(TfError::Protocol))?;
        let max_context_tokens = u32::try_from(response.max_model_len)
            .map(TokenCount::new)
            .map_err(|_| ModelError::from(TfError::Protocol))?;
        for token in response.tokens {
            let _token_id =
                u32::try_from(token).map_err(|_| ModelError::from(TfError::Protocol))?;
        }
        Ok(PromptTokenCount {
            input_tokens,
            max_context_tokens,
        })
    }

    async fn start_stream(
        &self,
        req: ChatRequest,
        protocol: StreamProtocol,
    ) -> Result<ProviderStream, ModelError> {
        if req.tools.len() > self.inner.config.max_tool_calls {
            return Err(ModelError::from(TfError::TooLarge));
        }
        if chat_request_input_bound(&req)? > self.inner.config.max_request_bytes {
            return Err(ModelError::from(TfError::TooLarge));
        }
        let request_body = match protocol {
            StreamProtocol::ChatCompletions => {
                chat_request_json(&req, self.inner.config.reasoning_replay)
            }
            StreamProtocol::Responses => responses_request_json(&req),
        }
        .map_err(ModelError::from)?;
        self.start_stream_with_body(req, protocol, request_body)
            .await
    }

    async fn start_stream_with_body(
        &self,
        req: ChatRequest,
        protocol: StreamProtocol,
        request_body: Value,
    ) -> Result<ProviderStream, ModelError> {
        let stream_deadline = tokio::time::Instant::now()
            .checked_add(self.inner.config.request_timeout)
            .ok_or_else(|| ModelError::from(TfError::Configuration))?;
        if req.tools.len() > self.inner.config.max_tool_calls {
            return Err(ModelError::from(TfError::TooLarge));
        }
        if chat_request_input_bound(&req)? > self.inner.config.max_request_bytes {
            return Err(ModelError::from(TfError::TooLarge));
        }
        if serde_json::to_vec(&request_body)
            .map_err(|_| TfError::Json)
            .map_err(ModelError::from)?
            .len()
            > self.inner.config.max_request_bytes
        {
            return Err(ModelError::from(TfError::TooLarge));
        }
        let endpoint = match protocol {
            StreamProtocol::ChatCompletions => "chat/completions",
            StreamProtocol::Responses => "responses",
        };
        let url = self
            .inner
            .base
            .endpoint(endpoint)
            .map_err(ModelError::from)?;
        let mut permit = self
            .acquire_rate_limit(&req)
            .await
            .map_err(ModelError::from)?;
        if let Some(admission) = permit.as_mut() {
            tokio::time::timeout(self.inner.config.request_timeout, async {
                loop {
                    match admission.mark_dispatched() {
                        Ok(()) => return Ok(()),
                        Err(RateLimitError::Busy) => {
                            tokio::time::sleep(Duration::from_millis(1)).await;
                        }
                        Err(error) => return Err(error),
                    }
                }
            })
            .await
            .map_err(|_| ModelError::from(TfError::Timeout))?
            .map_err(rate_limit_error)?;
        }
        let response = self
            .send_with_429_retry(Method::POST, url, Some(request_body))
            .await
            .map_err(ModelError::from)?;
        let body: BodyStream = Box::pin(response.bytes_stream());
        let parser = SseParser::new(
            self.inner.config.max_sse_frame_bytes,
            self.inner.config.max_events_per_chunk,
        )
        .map_err(ModelError::from)?;
        Ok(ProviderStream {
            body,
            stream_deadline,
            parser: Some(parser),
            pending: VecDeque::new(),
            config: self.inner.config,
            protocol,
            permit,
            raw_bytes: 0,
            text_bytes: 0,
            tool_bytes: 0,
            tools: BTreeMap::new(),
            finish_reason: None,
            saw_done: false,
            terminal: false,
            eof_flushed: false,
        })
    }

    async fn validate_llama_cpp_profile(
        &self,
        expected: &LlamaCppExpectedProfile,
    ) -> Result<(), ModelError> {
        let props_url = self
            .inner
            .base
            .origin_endpoint("props")
            .map_err(ModelError::from)?;
        let props_response = self
            .send_with_429_retry(Method::GET, props_url, None)
            .await
            .map_err(ModelError::from)?;
        let props_bytes = read_bounded(
            props_response,
            self.inner.config.max_json_response_bytes,
            self.inner.config.stream_idle_timeout,
        )
        .await
        .map_err(ModelError::from)?;
        let props: LlamaPropsWire = serde_json::from_slice(&props_bytes)
            .map_err(|_| ModelError::from(TfError::Protocol))?;
        if props.build_info != expected.build_info
            || props.model_alias != expected.model_alias
            || props.default_generation_settings.n_ctx != expected.max_context_tokens.get()
            || !props.chat_template_caps.supports_tools
            || !props.chat_template_caps.supports_tool_calls
        {
            return Err(ModelError::from(TfError::Protocol));
        }

        let models_url = self
            .inner
            .base
            .endpoint("models")
            .map_err(ModelError::from)?;
        let models_response = self
            .send_with_429_retry(Method::GET, models_url, None)
            .await
            .map_err(ModelError::from)?;
        let models_bytes = read_bounded(
            models_response,
            self.inner.config.max_json_response_bytes,
            self.inner.config.stream_idle_timeout,
        )
        .await
        .map_err(ModelError::from)?;
        let models: LlamaModelListWire = serde_json::from_slice(&models_bytes)
            .map_err(|_| ModelError::from(TfError::Protocol))?;
        if models.data.len() != 1 {
            return Err(ModelError::from(TfError::Protocol));
        }
        let model = models
            .data
            .first()
            .ok_or_else(|| ModelError::from(TfError::Protocol))?;
        if model.id != expected.model_alias
            || model.meta.n_ctx != expected.max_context_tokens.get()
            || model.architecture.input_modalities.as_slice() != ["text"]
            || model.architecture.output_modalities.as_slice() != ["text"]
        {
            return Err(ModelError::from(TfError::Protocol));
        }
        Ok(())
    }

    async fn count_llama_prompt(
        &self,
        request: &ChatRequest,
        profile: &LlamaCppExpectedProfile,
    ) -> Result<PromptTokenCount, ModelError> {
        if request.tools.len() > self.inner.config.max_tool_calls
            || chat_request_input_bound(request)? > self.inner.config.max_request_bytes
        {
            return Err(ModelError::from(TfError::TooLarge));
        }
        let body = llama_chat_request_json(
            request,
            self.inner.config.reasoning_replay,
            &profile.model_alias,
        )?;
        if serde_json::to_vec(&body)
            .map_err(|_| TfError::Json)
            .map_err(ModelError::from)?
            .len()
            > self.inner.config.max_request_bytes
        {
            return Err(ModelError::from(TfError::TooLarge));
        }
        let url = self
            .inner
            .base
            .endpoint("chat/completions/input_tokens")
            .map_err(ModelError::from)?;
        let response = self
            .send_with_429_retry(Method::POST, url, Some(body))
            .await
            .map_err(ModelError::from)?;
        let bytes = read_bounded(
            response,
            self.inner.config.max_json_response_bytes,
            self.inner.config.stream_idle_timeout,
        )
        .await
        .map_err(ModelError::from)?;
        let wire: LlamaInputTokensWire =
            serde_json::from_slice(&bytes).map_err(|_| ModelError::from(TfError::Protocol))?;
        if wire.object != "response.input_tokens" || wire.input_tokens == 0 {
            return Err(ModelError::from(TfError::Protocol));
        }
        Ok(PromptTokenCount {
            input_tokens: TokenCount::new(wire.input_tokens),
            max_context_tokens: profile.max_context_tokens,
        })
    }

    async fn acquire_rate_limit(
        &self,
        request: &ChatRequest,
    ) -> Result<Option<Permit<MonotonicClock>>, TfError> {
        let Some(policy) = &self.rate_limit else {
            return Ok(None);
        };
        let prompt = policy.estimator.estimate_prompt_tokens(request)?;
        let max_output = request.max_output_tokens.ok_or(TfError::Configuration)?;
        let estimate = Estimate { prompt, max_output };
        let permit = tokio::time::timeout(
            self.inner.config.request_timeout,
            policy.limiter.acquire(estimate, policy.priority),
        )
        .await
        .map_err(|_| TfError::Timeout)?
        .map_err(rate_limit_error)?;
        Ok(Some(permit))
    }

    fn build(
        base: BaseUrl,
        credentials: Credentials,
        config: ClientConfig,
    ) -> Result<Self, TfError> {
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
        Ok(Self {
            inner: Arc::new(ClientInner {
                http,
                base,
                credentials,
                config,
            }),
            rate_limit: None,
        })
    }

    /// Fetch available model identifiers from the OpenAI-compatible models endpoint.
    ///
    /// Model context sizes, modalities, and availability are intentionally omitted because they
    /// are not guaranteed by this endpoint and the recorded live catalog is not bundled here.
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, TfError> {
        let url = self.inner.base.endpoint("models")?;
        let response = self.send_with_429_retry(Method::GET, url, None).await?;
        let bytes = read_bounded(
            response,
            self.inner.config.max_json_response_bytes,
            self.inner.config.stream_idle_timeout,
        )
        .await?;
        let result: ModelListWire = serde_json::from_slice(&bytes).map_err(|_| TfError::Json)?;
        let mut seen = BTreeSet::new();
        let mut models = Vec::with_capacity(result.data.len());
        for item in result.data {
            if item.id.is_empty()
                || item.id.chars().any(char::is_control)
                || !seen.insert(item.id.clone())
            {
                return Err(TfError::Protocol);
            }
            models.push(ModelInfo { id: item.id });
        }
        Ok(models)
    }

    /// Request a batch of embeddings. The response's actual vector dimension is returned and
    /// checked for consistency; no dimension is assumed from a model name.
    pub async fn embeddings(
        &self,
        model: &ModelId,
        batch: &[&str],
        config: &EmbeddingConfig,
    ) -> Result<EmbeddingBatch, TfError> {
        config.validate()?;
        if batch.is_empty()
            || batch.len() > config.max_batch_items
            || batch.iter().any(|text| text.len() > config.max_input_bytes)
        {
            return Err(TfError::Configuration);
        }
        let input_bound = batch
            .iter()
            .try_fold(128usize, |total, text| {
                total.checked_add(text.len()).and_then(|n| n.checked_add(8))
            })
            .ok_or(TfError::TooLarge)?;
        if input_bound > self.inner.config.max_request_bytes {
            return Err(TfError::TooLarge);
        }
        let body = json!({"model": model.as_str(), "input": batch});
        if serde_json::to_vec(&body).map_err(|_| TfError::Json)?.len()
            > self.inner.config.max_request_bytes
        {
            return Err(TfError::TooLarge);
        }
        let response = self
            .send_with_429_retry(
                Method::POST,
                self.inner.base.endpoint("embeddings")?,
                Some(body),
            )
            .await?;
        let bytes = read_bounded(
            response,
            self.inner.config.max_json_response_bytes,
            self.inner.config.stream_idle_timeout,
        )
        .await?;
        let parsed: EmbeddingResponseWire =
            serde_json::from_slice(&bytes).map_err(|_| TfError::Json)?;
        if parsed.data.len() != batch.len() {
            return Err(TfError::Protocol);
        }
        let mut vectors = BTreeMap::new();
        for item in parsed.data {
            if item.index >= batch.len() || vectors.insert(item.index, item.embedding).is_some() {
                return Err(TfError::Protocol);
            }
        }
        let mut ordered = Vec::with_capacity(batch.len());
        for expected_index in 0..batch.len() {
            let Some(vector) = vectors.remove(&expected_index) else {
                return Err(TfError::Protocol);
            };
            if vector.is_empty()
                || vector.len() > config.max_dimensions
                || vector.iter().any(|value| !value.is_finite())
            {
                return Err(TfError::Protocol);
            }
            ordered.push(vector);
        }
        let dimension = ordered.first().map(Vec::len).ok_or(TfError::Protocol)?;
        if ordered.iter().any(|vector| vector.len() != dimension) {
            return Err(TfError::Protocol);
        }
        Ok(EmbeddingBatch {
            vectors: ordered,
            dimension,
        })
    }

    /// Create an SDK embedding adapter that checks each real response against an explicitly
    /// configured dimension observed or otherwise verified by the composition root.
    /// # Errors
    /// Returns Configuration for a zero or unsupported expected dimension.
    pub fn embedding_provider(
        &self,
        model: ModelId,
        config: EmbeddingConfig,
        expected_dimension: usize,
    ) -> Result<EmbeddingClient, TfError> {
        config.validate()?;
        if expected_dimension == 0 || expected_dimension > config.max_dimensions {
            return Err(TfError::Configuration);
        }
        Ok(EmbeddingClient {
            client: self.clone(),
            model,
            config,
            expected_dimension,
        })
    }

    async fn send_with_429_retry(
        &self,
        method: Method,
        url: Url,
        body: Option<Value>,
    ) -> Result<Response, TfError> {
        let mut retries = 0_u8;
        let start = tokio::time::Instant::now();
        let Some(deadline) = start.checked_add(self.inner.config.request_timeout) else {
            return Err(TfError::Configuration);
        };
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(TfError::Timeout);
            }
            let mut request = self.inner.http.request(method.clone(), url.clone());
            if let Credentials::Bearer(key) = &self.inner.credentials {
                request = request.bearer_auth(key.0.expose_secret());
            }
            if let Some(json_body) = body.as_ref() {
                request = request.json(json_body);
            }
            let response = match tokio::time::timeout(remaining, request.send()).await {
                Err(_) => return Err(TfError::Timeout),
                Ok(Err(_)) => return Err(TfError::Transport),
                Ok(Ok(response)) => response,
            };
            let status = response.status();
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(parse_retry_after);
            if status == StatusCode::TOO_MANY_REQUESTS
                && let Some(policy) = &self.rate_limit
            {
                policy
                    .limiter
                    .on_response(status.as_u16(), retry_after)
                    .map_err(rate_limit_error)?;
            }
            let retry_limit = if status == StatusCode::TOO_MANY_REQUESTS {
                self.inner.config.max_429_retries
            } else if status.is_server_error() {
                self.inner.config.max_5xx_retries
            } else {
                0
            };
            if retries < retry_limit {
                let delay = retry_after.map_or_else(
                    || retry_delay(None, retries, self.inner.config),
                    |value| value.min(self.inner.config.max_retry_delay),
                );
                if delay >= deadline.saturating_duration_since(tokio::time::Instant::now()) {
                    return Err(status_error(response));
                }
                retries = retries.saturating_add(1);
                tokio::time::sleep(delay).await;
                continue;
            }
            if !status.is_success() {
                return Err(status_error(response));
            }
            return Ok(response);
        }
    }
}

impl std::fmt::Debug for TokenFactoryClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenFactoryClient")
            .field("base", &self.inner.base)
            .field("credential", &self.inner.credentials)
            .finish()
    }
}

impl ChatModel for TokenFactoryClient {
    type Stream = ProviderStream;

    async fn stream(&self, req: ChatRequest) -> Result<Self::Stream, ModelError> {
        self.start_stream(req, StreamProtocol::ChatCompletions)
            .await
    }
}

/// One model id from the provider's models endpoint. No unverified capabilities are attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    /// Provider model identifier.
    pub id: String,
}

#[derive(Deserialize)]
struct ModelListWire {
    data: Vec<ModelListEntryWire>,
}

#[derive(Deserialize)]
struct TokenizeResponseWire {
    count: u64,
    max_model_len: u64,
    tokens: Vec<u64>,
}

#[derive(Deserialize)]
struct LlamaPropsWire {
    build_info: String,
    model_alias: String,
    default_generation_settings: LlamaGenerationSettingsWire,
    chat_template_caps: LlamaChatTemplateCapsWire,
}

#[derive(Deserialize)]
struct LlamaGenerationSettingsWire {
    n_ctx: u32,
}

#[derive(Deserialize)]
struct LlamaChatTemplateCapsWire {
    supports_tools: bool,
    supports_tool_calls: bool,
}

#[derive(Deserialize)]
struct LlamaModelListWire {
    data: Vec<LlamaModelWire>,
}

#[derive(Deserialize)]
struct LlamaModelWire {
    id: String,
    meta: LlamaModelMetaWire,
    architecture: LlamaModelArchitectureWire,
}

#[derive(Deserialize)]
struct LlamaModelMetaWire {
    n_ctx: u32,
}

#[derive(Deserialize)]
struct LlamaModelArchitectureWire {
    input_modalities: Vec<String>,
    output_modalities: Vec<String>,
}

#[derive(Deserialize)]
struct LlamaInputTokensWire {
    input_tokens: u32,
    object: String,
}

#[derive(Deserialize)]
struct ModelListEntryWire {
    id: String,
}

#[derive(Deserialize)]
struct EmbeddingResponseWire {
    data: Vec<EmbeddingItemWire>,
}
#[derive(Deserialize)]
struct EmbeddingItemWire {
    index: usize,
    embedding: Vec<f32>,
}

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
        f.debug_struct("EmbeddingClient")
            .field("model", &self.model)
            .field("expected_dimension", &self.expected_dimension)
            .finish()
    }
}

impl EmbeddingClient {
    /// Fetch a batch and return the dimension observed in the provider response.
    pub async fn embeddings(&self, batch: &[&str]) -> Result<EmbeddingBatch, TfError> {
        self.client
            .embeddings(&self.model, batch, &self.config)
            .await
    }
}

impl EmbeddingProvider for EmbeddingClient {
    fn dimension(&self) -> usize {
        self.expected_dimension
    }

    async fn embed(&self, batch: &[&str]) -> Result<Vec<Vec<f32>>, ModelError> {
        let result = self.embeddings(batch).await.map_err(ModelError::from)?;
        if result.dimension != self.expected_dimension {
            return Err(ModelError::Protocol(
                "embedding dimension mismatch".to_owned(),
            ));
        }
        Ok(result.vectors)
    }
}

type BodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

/// Pull-based SDK stream for one completion.
pub struct ProviderStream {
    body: BodyStream,
    stream_deadline: tokio::time::Instant,
    parser: Option<SseParser>,
    pending: VecDeque<Result<ChatChunk, ModelError>>,
    config: ClientConfig,
    protocol: StreamProtocol,
    permit: Option<Permit<MonotonicClock>>,
    raw_bytes: usize,
    text_bytes: usize,
    tool_bytes: usize,
    tools: BTreeMap<u32, ToolCallState>,
    finish_reason: Option<FinishReason>,
    saw_done: bool,
    terminal: bool,
    eof_flushed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamProtocol {
    ChatCompletions,
    Responses,
}

impl ChatChunkSource for ProviderStream {
    async fn next_chunk(&mut self) -> Option<Result<ChatChunk, ModelError>> {
        loop {
            if let Some(chunk) = self.pending.pop_front() {
                return Some(chunk);
            }
            if self.terminal {
                return None;
            }
            let remaining = self
                .stream_deadline
                .saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return self.fail(TfError::Timeout);
            }
            let next = tokio::time::timeout(
                remaining.min(self.config.stream_idle_timeout),
                self.body.next(),
            )
            .await;
            match next {
                Err(_) => return self.fail(TfError::Timeout),
                Ok(Some(Err(error))) => {
                    return self.fail(if error.is_timeout() {
                        TfError::Timeout
                    } else {
                        TfError::Transport
                    });
                }
                Ok(Some(Ok(bytes))) => {
                    self.raw_bytes = match self.raw_bytes.checked_add(bytes.len()) {
                        Some(total) if total <= self.config.max_chat_stream_bytes => total,
                        _ => return self.fail(TfError::TooLarge),
                    };
                    if let Err(error) = self.route_bytes(&bytes) {
                        return self.fail(error);
                    }
                }
                Ok(None) => {
                    if !self.eof_flushed {
                        if let Err(error) = self.finish_parser() {
                            return self.fail(error);
                        }
                        self.eof_flushed = true;
                    }
                    if !self.saw_done || self.finish_reason.is_none() {
                        return self.fail(TfError::StreamTruncated);
                    }
                    if let Err(error) = self.validate_tool_calls() {
                        return self.fail(error);
                    }
                    self.terminal = true;
                    let Some(reason) = self.finish_reason else {
                        return self.fail(TfError::StreamTruncated);
                    };
                    return Some(Ok(ChatChunk::Finished(reason)));
                }
            }
        }
    }
}

impl ProviderStream {
    fn route_bytes(&mut self, bytes: &[u8]) -> Result<(), TfError> {
        let mut parser = self.parser.take().ok_or(TfError::Protocol)?;
        let result = parser.feed_with(bytes, |event| self.consume_event(event));
        self.parser = Some(parser);
        result
    }

    fn finish_parser(&mut self) -> Result<(), TfError> {
        let mut parser = self.parser.take().ok_or(TfError::Protocol)?;
        let result = parser.finish_with(|event| self.consume_event(event));
        self.parser = Some(parser);
        result
    }

    fn fail(&mut self, error: TfError) -> Option<Result<ChatChunk, ModelError>> {
        self.terminal = true;
        self.pending.clear();
        Some(Err(ModelError::from(error)))
    }

    fn consume_event(&mut self, event: SseEventRef<'_>) -> Result<(), TfError> {
        if self.saw_done {
            return Err(TfError::Protocol);
        }
        match event {
            SseEventRef::Done => {
                if self.protocol == StreamProtocol::Responses {
                    return Err(TfError::Protocol);
                }
                if self.finish_reason.is_none() {
                    return Err(TfError::StreamTruncated);
                }
                self.validate_tool_calls()?;
                self.saw_done = true;
            }
            SseEventRef::Data(data) => {
                if self.protocol == StreamProtocol::Responses {
                    return self.consume_responses_event(data);
                }
                let chunk: ChatCompletionChunk<'_> =
                    serde_json::from_str(data).map_err(|_| TfError::Json)?;
                if chunk.choices.len() > 1 {
                    return Err(TfError::Protocol);
                }
                if let Some(usage) = chunk.usage {
                    let usage: Usage = usage.into();
                    self.reconcile_usage(usage)?;
                    self.pending.push_back(Ok(ChatChunk::Usage(usage)));
                }
                for choice in chunk.choices {
                    if choice.index != 0 {
                        return Err(TfError::Protocol);
                    }
                    // OpenRouter repeats `"content": ""` on the usage chunk after the finish
                    // reason; empty strings carry no output, so only real deltas count here.
                    let reasoning = choice.delta.reasoning_content.or(choice.delta.reasoning);
                    let has_delta = choice.delta.content.as_ref().is_some_and(|c| !c.is_empty())
                        || reasoning.as_ref().is_some_and(|r| !r.is_empty())
                        || !choice.delta.tool_calls.is_empty();
                    if self.finish_reason.is_some() && has_delta {
                        return Err(TfError::Protocol);
                    }
                    if let Some(content) = choice.delta.content {
                        self.add_text_bytes(content.len())?;
                        if !content.is_empty() {
                            self.pending
                                .push_back(Ok(ChatChunk::Text(content.into_owned())));
                        }
                    }
                    if let Some(reasoning) = reasoning {
                        self.add_text_bytes(reasoning.len())?;
                        if !reasoning.is_empty() {
                            self.pending
                                .push_back(Ok(ChatChunk::Reasoning(reasoning.into_owned())));
                        }
                    }
                    for delta in choice.delta.tool_calls {
                        self.add_tool_delta(delta)?;
                    }
                    if let Some(reason) = choice.finish_reason {
                        let mapped = parse_finish_reason(reason.as_ref());
                        if self
                            .finish_reason
                            .is_some_and(|existing| existing != mapped)
                        {
                            return Err(TfError::Protocol);
                        }
                        self.finish_reason = Some(mapped);
                    }
                }
            }
        }
        Ok(())
    }

    fn consume_responses_event(&mut self, data: &str) -> Result<(), TfError> {
        let value: Value = serde_json::from_str(data).map_err(|_| TfError::Json)?;
        let event_type = required_string(&value, "type")?;
        match event_type {
            "response.created"
            | "response.queued"
            | "response.in_progress"
            | "response.content_part.added"
            | "response.content_part.done"
            | "response.output_text.done"
            | "response.reasoning_text.done"
            | "response.reasoning_summary_text.done"
            | "response.output_item.done" => Ok(()),
            "response.output_item.added" => self.register_response_output_item(&value),
            "response.output_text.delta" => {
                let text = required_string(&value, "delta")?.to_owned();
                self.add_text_bytes(text.len())?;
                if !text.is_empty() {
                    self.pending.push_back(Ok(ChatChunk::Text(text)));
                }
                Ok(())
            }
            "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                let reasoning = required_string(&value, "delta")?.to_owned();
                self.add_text_bytes(reasoning.len())?;
                if !reasoning.is_empty() {
                    self.pending.push_back(Ok(ChatChunk::Reasoning(reasoning)));
                }
                Ok(())
            }
            "response.function_call_arguments.delta" => self.consume_response_tool_delta(&value),
            "response.function_call_arguments.done" => self.finish_response_tool_call(&value),
            "response.completed" => self.finish_response(&value, FinishReason::Stop, "completed"),
            "response.incomplete" => {
                let response = value.get("response").ok_or(TfError::Protocol)?;
                let reason = response
                    .get("incomplete_details")
                    .and_then(|details| details.get("reason"))
                    .and_then(Value::as_str);
                let finish = match reason {
                    Some("max_output_tokens") => FinishReason::Length,
                    _ => FinishReason::Other,
                };
                self.finish_response(&value, finish, "incomplete")
            }
            "response.failed" | "error" => Err(TfError::Protocol),
            _ => Err(TfError::Protocol),
        }
    }

    fn register_response_output_item(&mut self, value: &Value) -> Result<(), TfError> {
        let index = required_u32(value, "output_index")?;
        let item = value.get("item").ok_or(TfError::Protocol)?;
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            return Ok(());
        }
        let call_id = required_string(item, "call_id")?.to_owned();
        let name = required_string(item, "name")?.to_owned();
        if call_id.is_empty()
            || name.is_empty()
            || !name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            return Err(TfError::Protocol);
        }
        if !self.tools.contains_key(&index) && self.tools.len() >= self.config.max_tool_calls {
            return Err(TfError::TooLarge);
        }
        let state = self.tools.entry(index).or_default();
        set_consistent(&mut state.id, Some(call_id))?;
        set_consistent(&mut state.name, Some(name))?;
        Ok(())
    }

    fn consume_response_tool_delta(&mut self, value: &Value) -> Result<(), TfError> {
        let index = required_u32(value, "output_index")?;
        let call_id = required_string(value, "call_id")?;
        let delta = required_string(value, "delta")?;
        self.add_response_tool_arguments(index, call_id, delta)
    }

    fn add_response_tool_arguments(
        &mut self,
        index: u32,
        call_id: &str,
        delta: &str,
    ) -> Result<(), TfError> {
        let state = self.tools.get_mut(&index).ok_or(TfError::StreamTruncated)?;
        if state.id.as_deref() != Some(call_id) {
            return Err(TfError::Protocol);
        }
        let new_total = self
            .tool_bytes
            .checked_add(delta.len())
            .ok_or(TfError::TooLarge)?;
        if new_total > self.config.max_tool_argument_bytes {
            return Err(TfError::TooLarge);
        }
        let first_fragment = state.arguments.is_empty();
        state.arguments.push_str(delta);
        self.tool_bytes = new_total;
        self.pending.push_back(Ok(ChatChunk::ToolCallDelta {
            index,
            id: if first_fragment {
                state.id.clone()
            } else {
                None
            },
            name: if first_fragment {
                state.name.clone()
            } else {
                None
            },
            arguments_fragment: delta.to_owned(),
        }));
        Ok(())
    }

    fn finish_response_tool_call(&mut self, value: &Value) -> Result<(), TfError> {
        let index = required_u32(value, "output_index")?;
        let call_id = required_string(value, "call_id")?;
        let name = required_string(value, "name")?;
        let arguments = required_string(value, "arguments")?;
        let state = self.tools.get_mut(&index).ok_or(TfError::StreamTruncated)?;
        set_consistent(&mut state.id, Some(call_id.to_owned()))?;
        set_consistent(&mut state.name, Some(name.to_owned()))?;
        if state.arguments.is_empty() && !arguments.is_empty() {
            let delta_bytes = arguments.len();
            let new_total = self
                .tool_bytes
                .checked_add(delta_bytes)
                .ok_or(TfError::TooLarge)?;
            if new_total > self.config.max_tool_argument_bytes {
                return Err(TfError::TooLarge);
            }
            state.arguments.push_str(arguments);
            self.tool_bytes = new_total;
            self.pending.push_back(Ok(ChatChunk::ToolCallDelta {
                index,
                id: state.id.clone(),
                name: state.name.clone(),
                arguments_fragment: arguments.to_owned(),
            }));
        } else if state.arguments != arguments {
            return Err(TfError::Protocol);
        }
        Ok(())
    }

    fn finish_response(
        &mut self,
        value: &Value,
        finish_reason: FinishReason,
        expected_status: &str,
    ) -> Result<(), TfError> {
        let response = value.get("response").ok_or(TfError::Protocol)?;
        if required_string(response, "status")? != expected_status {
            return Err(TfError::Protocol);
        }
        if let Some(usage) = response.get("usage").filter(|usage| !usage.is_null()) {
            let usage: ResponsesUsageWire =
                serde_json::from_value(usage.clone()).map_err(|_| TfError::Json)?;
            let usage: Usage = usage.into();
            self.reconcile_usage(usage)?;
            self.pending.push_back(Ok(ChatChunk::Usage(usage)));
        }
        self.validate_tool_calls()?;
        let reason = if finish_reason == FinishReason::Stop && !self.tools.is_empty() {
            FinishReason::ToolCalls
        } else {
            finish_reason
        };
        if self
            .finish_reason
            .is_some_and(|existing| existing != reason)
        {
            return Err(TfError::Protocol);
        }
        self.finish_reason = Some(reason);
        self.saw_done = true;
        Ok(())
    }

    fn add_text_bytes(&mut self, bytes: usize) -> Result<(), TfError> {
        self.text_bytes = self
            .text_bytes
            .checked_add(bytes)
            .ok_or(TfError::TooLarge)?;
        if self.text_bytes > self.config.max_text_bytes {
            return Err(TfError::TooLarge);
        }
        Ok(())
    }

    fn reconcile_usage(&mut self, usage: Usage) -> Result<(), TfError> {
        let Some(permit) = self.permit.take() else {
            return Ok(());
        };
        let actual = u64::from(usage.prompt_tokens.get())
            .checked_add(u64::from(usage.completion_tokens.get()))
            .ok_or(TfError::RateLimit)?;
        permit.reconcile(actual).map_err(rate_limit_error)?;
        Ok(())
    }

    fn add_tool_delta(&mut self, delta: ToolCallDeltaWire<'_>) -> Result<(), TfError> {
        if !self.tools.contains_key(&delta.index) && self.tools.len() >= self.config.max_tool_calls
        {
            return Err(TfError::TooLarge);
        }
        let id = delta.id.map(Cow::into_owned);
        let name = delta.function.name.map(Cow::into_owned);
        let arguments = delta.function.arguments.unwrap_or_default().into_owned();
        self.tool_bytes = self
            .tool_bytes
            .checked_add(arguments.len())
            .ok_or(TfError::TooLarge)?;
        if self.tool_bytes > self.config.max_tool_argument_bytes {
            return Err(TfError::TooLarge);
        }
        let state = self.tools.entry(delta.index).or_default();
        set_consistent(&mut state.id, id.clone())?;
        set_consistent(&mut state.name, name.clone())?;
        state.arguments.push_str(&arguments);
        self.pending.push_back(Ok(ChatChunk::ToolCallDelta {
            index: delta.index,
            id,
            name,
            arguments_fragment: arguments,
        }));
        Ok(())
    }

    fn validate_tool_calls(&self) -> Result<(), TfError> {
        let mut seen_ids = BTreeSet::new();
        for call in self.tools.values() {
            let id = call
                .id
                .as_deref()
                .filter(|id| !id.is_empty())
                .ok_or(TfError::StreamTruncated)?;
            let name = call
                .name
                .as_deref()
                .filter(|name| !name.is_empty())
                .ok_or(TfError::StreamTruncated)?;
            if !seen_ids.insert(id)
                || !name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
            {
                return Err(TfError::Protocol);
            }
            let args: Value =
                serde_json::from_str(&call.arguments).map_err(|_| TfError::StreamTruncated)?;
            if !args.is_object() {
                return Err(TfError::StreamTruncated);
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct ToolCallState {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

fn set_consistent(target: &mut Option<String>, update: Option<String>) -> Result<(), TfError> {
    if let Some(update) = update {
        if let Some(existing) = target.as_ref() {
            if existing != &update {
                return Err(TfError::Protocol);
            }
        } else {
            *target = Some(update);
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "'de: 'a"))]
struct ChatCompletionChunk<'a> {
    #[serde(default)]
    choices: Vec<ChoiceWire<'a>>,
    usage: Option<UsageWire>,
}
#[derive(Deserialize)]
#[serde(bound(deserialize = "'de: 'a"))]
struct ChoiceWire<'a> {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    delta: DeltaWire<'a>,
    finish_reason: Option<Cow<'a, str>>,
}
#[derive(Deserialize, Default)]
#[serde(bound(deserialize = "'de: 'a"))]
struct DeltaWire<'a> {
    content: Option<Cow<'a, str>>,
    /// vLLM / Token Factory reasoning field.
    reasoning_content: Option<Cow<'a, str>>,
    /// OpenRouter reasoning field; used only when `reasoning_content` is absent.
    reasoning: Option<Cow<'a, str>>,
    #[serde(default)]
    tool_calls: Vec<ToolCallDeltaWire<'a>>,
}
#[derive(Deserialize)]
#[serde(bound(deserialize = "'de: 'a"))]
struct ToolCallDeltaWire<'a> {
    index: u32,
    id: Option<Cow<'a, str>>,
    function: FunctionDeltaWire<'a>,
}
#[derive(Deserialize)]
#[serde(bound(deserialize = "'de: 'a"))]
struct FunctionDeltaWire<'a> {
    name: Option<Cow<'a, str>>,
    arguments: Option<Cow<'a, str>>,
}
#[derive(Deserialize)]
struct UsageWire {
    prompt_tokens: u32,
    completion_tokens: u32,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokenDetailsWire>,
}
#[derive(Deserialize)]
struct PromptTokenDetailsWire {
    cached_tokens: Option<u32>,
}

impl From<UsageWire> for Usage {
    fn from(value: UsageWire) -> Self {
        let cached = value
            .prompt_tokens_details
            .and_then(|details| details.cached_tokens)
            .map_or_else(|| TokenCount::new(0), TokenCount::new);
        Self {
            prompt_tokens: TokenCount::new(value.prompt_tokens),
            completion_tokens: TokenCount::new(value.completion_tokens),
            cached_tokens: cached,
        }
    }
}

#[derive(Deserialize)]
struct ResponsesUsageWire {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(default)]
    input_tokens_details: Option<ResponsesInputDetailsWire>,
}

#[derive(Deserialize)]
struct ResponsesInputDetailsWire {
    cached_tokens: Option<u32>,
}

impl From<ResponsesUsageWire> for Usage {
    fn from(value: ResponsesUsageWire) -> Self {
        let cached = value
            .input_tokens_details
            .and_then(|details| details.cached_tokens)
            .map_or_else(|| TokenCount::new(0), TokenCount::new);
        Self {
            prompt_tokens: TokenCount::new(value.input_tokens),
            completion_tokens: TokenCount::new(value.output_tokens),
            cached_tokens: cached,
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

/// Conservative lower-cost bound used before constructing the duplicated wire JSON tree.
/// JSON escaping and field names can increase the final size, so the serialized-size check remains
/// authoritative after construction.
pub(crate) fn chat_request_input_bound(req: &ChatRequest) -> Result<usize, TfError> {
    let mut total = req
        .model
        .as_str()
        .len()
        .checked_add(128)
        .ok_or(TfError::TooLarge)?;
    for message in &req.messages {
        total = total
            .checked_add(message.content.len())
            .and_then(|n| n.checked_add(64))
            .ok_or(TfError::TooLarge)?;
        if let Some(call_id) = &message.tool_call_id {
            total = total
                .checked_add(call_id.len())
                .and_then(|n| n.checked_add(16))
                .ok_or(TfError::TooLarge)?;
        }
        if let Some(reasoning) = &message.reasoning {
            total = total
                .checked_add(reasoning.len())
                .and_then(|n| n.checked_add(24))
                .ok_or(TfError::TooLarge)?;
        }
        for call in &message.tool_calls {
            total = total
                .checked_add(call.id.len())
                .and_then(|n| n.checked_add(call.name.len()))
                .and_then(|n| n.checked_add(call.arguments_json.len()))
                .and_then(|n| n.checked_add(96))
                .ok_or(TfError::TooLarge)?;
        }
    }
    for tool in &req.tools {
        total = total
            .checked_add(tool.name.len())
            .and_then(|n| n.checked_add(tool.description.len()))
            .and_then(|n| n.checked_add(tool.input_schema_json.len()))
            .and_then(|n| n.checked_add(96))
            .ok_or(TfError::TooLarge)?;
    }
    Ok(total)
}

fn chat_request_json(
    req: &ChatRequest,
    reasoning_replay: ReasoningReplay,
) -> Result<Value, TfError> {
    if req.messages.is_empty() {
        return Err(TfError::BadRequest);
    }
    validate_history(&req.messages)?;
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|message| {
            let mut wire = serde_json::Map::new();
            let role = match message.role {
                ChatRole::System => "system",
                ChatRole::User => "user",
                ChatRole::Assistant => "assistant",
                ChatRole::Tool => "tool",
            };
            wire.insert("role".to_owned(), json!(role));
            wire.insert("content".to_owned(), json!(message.content));
            if let Some(call_id) = &message.tool_call_id {
                wire.insert("tool_call_id".to_owned(), json!(call_id));
            }
            if !message.tool_calls.is_empty() {
                let calls: Vec<Value> = message
                    .tool_calls
                    .iter()
                    .map(tool_call_message_json)
                    .collect();
                wire.insert("tool_calls".to_owned(), Value::Array(calls));
            }
            if reasoning_replay == ReasoningReplay::ReasoningContent
                && let Some(reasoning) = &message.reasoning
            {
                wire.insert("reasoning_content".to_owned(), json!(reasoning));
            }
            Value::Object(wire)
        })
        .collect();
    let mut body = serde_json::Map::new();
    body.insert("model".to_owned(), json!(req.model.as_str()));
    body.insert("messages".to_owned(), Value::Array(messages));
    body.insert("stream".to_owned(), json!(true));
    body.insert("n".to_owned(), json!(1));
    if let Some(max_tokens) = req.max_output_tokens {
        body.insert("max_tokens".to_owned(), json!(max_tokens.get()));
    }
    if !req.tools.is_empty() {
        let mut names = BTreeSet::new();
        if req
            .tools
            .iter()
            .any(|tool| !names.insert(tool.name.as_ref()))
        {
            return Err(TfError::BadRequest);
        }
        let tools = req
            .tools
            .iter()
            .map(tool_json)
            .collect::<Result<Vec<_>, _>>()?;
        body.insert("tools".to_owned(), Value::Array(tools));
    }
    // The SDK carries effort, but the provider-specific reasoning control is unverified (S0-5).
    // Do not invent a reasoning_effort or budget field.
    let _requested_effort = req.effort;
    Ok(Value::Object(body))
}

fn llama_chat_request_json(
    req: &ChatRequest,
    reasoning_replay: ReasoningReplay,
    model_alias: &str,
) -> Result<Value, ModelError> {
    let mut body = chat_request_json(req, reasoning_replay).map_err(ModelError::from)?;
    let body_object = body
        .as_object_mut()
        .ok_or_else(|| ModelError::from(TfError::Protocol))?;
    body_object.insert("model".to_owned(), json!(model_alias));
    body_object.insert("stream_options".to_owned(), json!({"include_usage": true}));
    Ok(body)
}

fn responses_request_json(req: &ChatRequest) -> Result<Value, TfError> {
    if req.messages.is_empty() {
        return Err(TfError::BadRequest);
    }
    validate_history(&req.messages)?;
    let mut input = Vec::new();
    for message in &req.messages {
        match message.role {
            ChatRole::Tool => {
                let call_id = message.tool_call_id.as_deref().ok_or(TfError::BadRequest)?;
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": message.content,
                }));
            }
            role => {
                let role = match role {
                    ChatRole::System => "system",
                    ChatRole::User => "user",
                    ChatRole::Assistant => "assistant",
                    ChatRole::Tool => return Err(TfError::BadRequest),
                };
                input.push(json!({
                    "type": "message",
                    "role": role,
                    "content": message.content,
                }));
                for call in &message.tool_calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": call.arguments_json,
                    }));
                }
            }
        }
    }
    let mut body = serde_json::Map::new();
    body.insert("model".to_owned(), json!(req.model.as_str()));
    body.insert("input".to_owned(), Value::Array(input));
    body.insert("stream".to_owned(), json!(true));
    if let Some(max_tokens) = req.max_output_tokens {
        body.insert("max_output_tokens".to_owned(), json!(max_tokens.get()));
    }
    if !req.tools.is_empty() {
        let mut names = BTreeSet::new();
        let mut tools = Vec::with_capacity(req.tools.len());
        for tool in &req.tools {
            if !names.insert(tool.name.as_ref())
                || tool.name.is_empty()
                || !tool
                    .name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
            {
                return Err(TfError::BadRequest);
            }
            let parameters: Value =
                serde_json::from_str(&tool.input_schema_json).map_err(|_| TfError::BadRequest)?;
            if !parameters.is_object() {
                return Err(TfError::BadRequest);
            }
            tools.push(json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": parameters,
                "strict": false,
            }));
        }
        body.insert("tools".to_owned(), Value::Array(tools));
        body.insert("tool_choice".to_owned(), json!("auto"));
    }
    // The provider-specific reasoning controls and replay format remain unverified (S0-5).
    Ok(Value::Object(body))
}

fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, TfError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(TfError::Protocol)
}

fn required_u32(value: &Value, key: &str) -> Result<u32, TfError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|number| u32::try_from(number).ok())
        .ok_or(TfError::Protocol)
}

fn validate_history(messages: &[polaris_sdk::registry::ChatMessage]) -> Result<(), TfError> {
    let mut known_calls = BTreeSet::new();
    let mut outstanding_calls = BTreeSet::new();
    for message in messages {
        if message.reasoning.is_some() && message.role != ChatRole::Assistant {
            return Err(TfError::BadRequest);
        }
        if !outstanding_calls.is_empty() && message.role != ChatRole::Tool {
            return Err(TfError::BadRequest);
        }
        match message.role {
            ChatRole::Assistant => {
                if message.tool_call_id.is_some() {
                    return Err(TfError::BadRequest);
                }
                for call in &message.tool_calls {
                    if call.id.is_empty()
                        || call.name.is_empty()
                        || !known_calls.insert(call.id.clone())
                        || !outstanding_calls.insert(call.id.clone())
                    {
                        return Err(TfError::BadRequest);
                    }
                    let args: Value = serde_json::from_str(&call.arguments_json)
                        .map_err(|_| TfError::BadRequest)?;
                    if !args.is_object() {
                        return Err(TfError::BadRequest);
                    }
                }
            }
            ChatRole::Tool => {
                if !message.tool_calls.is_empty() {
                    return Err(TfError::BadRequest);
                }
                let Some(call_id) = message.tool_call_id.as_ref() else {
                    return Err(TfError::BadRequest);
                };
                if !outstanding_calls.remove(call_id) {
                    return Err(TfError::BadRequest);
                }
            }
            ChatRole::System | ChatRole::User => {
                if !message.tool_calls.is_empty() || message.tool_call_id.is_some() {
                    return Err(TfError::BadRequest);
                }
            }
        }
    }
    if !outstanding_calls.is_empty() {
        return Err(TfError::BadRequest);
    }
    Ok(())
}

fn tool_call_message_json(call: &ChatToolCall) -> Value {
    json!({"id": call.id, "type": "function", "function": {"name": call.name, "arguments": call.arguments_json}})
}

fn tool_json(tool: &ToolSpec) -> Result<Value, TfError> {
    let parameters: Value =
        serde_json::from_str(&tool.input_schema_json).map_err(|_| TfError::BadRequest)?;
    if tool.name.is_empty()
        || !tool
            .name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        || !parameters.is_object()
    {
        return Err(TfError::BadRequest);
    }
    Ok(json!({"type": "function", "function": {
        "name": tool.name, "description": tool.description, "parameters": parameters
    }}))
}

async fn read_bounded(
    response: Response,
    maximum: usize,
    idle_timeout: Duration,
) -> Result<Bytes, TfError> {
    let mut stream = Box::pin(response.bytes_stream());
    let mut buffer = BytesMut::new();
    loop {
        match tokio::time::timeout(idle_timeout, stream.next()).await {
            Err(_) => return Err(TfError::Timeout),
            Ok(None) => return Ok(buffer.freeze()),
            Ok(Some(Err(_))) => return Err(TfError::Transport),
            Ok(Some(Ok(bytes))) => {
                let length = buffer
                    .len()
                    .checked_add(bytes.len())
                    .ok_or(TfError::TooLarge)?;
                if length > maximum {
                    return Err(TfError::TooLarge);
                }
                buffer.extend_from_slice(&bytes);
            }
        }
    }
}

fn status_error(response: Response) -> TfError {
    let status = response.status();
    let retry_after_ms = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(parse_retry_after)
        .and_then(|delay| u64::try_from(delay.as_millis()).ok());
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => TfError::Unauthorized,
        StatusCode::BAD_REQUEST
        | StatusCode::PAYLOAD_TOO_LARGE
        | StatusCode::UNPROCESSABLE_ENTITY => TfError::BadRequest,
        _ => TfError::Http {
            status: status.as_u16(),
            retry_after_ms,
        },
    }
}

fn rate_limit_error(error: RateLimitError) -> TfError {
    match error {
        RateLimitError::ExceedsBurst => TfError::BadRequest,
        RateLimitError::Cancelled => TfError::Timeout,
        RateLimitError::InvalidQuota
        | RateLimitError::Accounting
        | RateLimitError::Undispatched
        | RateLimitError::Busy => TfError::RateLimit,
    }
}

fn retry_delay(
    header: Option<&reqwest::header::HeaderValue>,
    retry: u8,
    config: ClientConfig,
) -> Duration {
    let server = header.and_then(parse_retry_after);
    if let Some(delay) = server {
        return delay.min(config.max_retry_delay);
    }

    let shift = u32::from(retry);
    let multiplier = if shift < u32::BITS {
        1_u32 << shift
    } else {
        u32::MAX
    };
    let ceiling = config
        .retry_base_delay
        .saturating_mul(multiplier)
        .min(config.max_retry_delay);
    let ceiling_ms = ceiling
        .as_secs()
        .saturating_mul(1_000)
        .saturating_add(u64::from(ceiling.subsec_millis()));
    let jitter_ms = rand::rng().random_range(0..=ceiling_ms);
    Duration::from_millis(jitter_ms)
}

fn parse_retry_after(header: &reqwest::header::HeaderValue) -> Option<Duration> {
    let value = header.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return seconds.checked_mul(1_000).map(Duration::from_millis);
    }
    let timestamp = jiff::fmt::rfc2822::parse(value).ok()?.timestamp();
    Duration::try_from(timestamp.duration_since(jiff::Timestamp::now())).ok()
}

#[cfg(test)]
mod tests {
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
                && let Some(header_end) =
                    request.windows(4).position(|window| window == b"\r\n\r\n")
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
        let estimator =
            |_request: &ChatRequest| Ok::<TokenCount, super::TfError>(TokenCount::new(24));
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
        let (base, server) =
            fixture_server(vec![(429, vec![("Retry-After", "1")], String::new())])?;
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
        assert!(chunks.iter().any(|chunk| matches!(chunk, ChatChunk::Usage(_))));
        Ok(())
    }

    #[tokio::test]
    async fn non_empty_content_after_finish_is_still_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
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
        assert!(failed, "real content after the finish reason stays a protocol error");
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
                    r#"{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}"#
                        .into(),
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
            return Err(
                "embedding vectors were not validated and reordered by response index".into(),
            );
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
                input_schema_json: r#"{"type":"object","properties":{"q":{"type":"string"}}}"#
                    .into(),
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
            return Err(
                "Responses fixture did not receive the expected OpenAI-style request".into(),
            );
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
}
