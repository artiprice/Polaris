//! Provider selection chain and chat models for endpoints without a token-counting route.
//!
//! Hosted OpenAI-compatible providers (Token Factory, OpenRouter) and local servers such as
//! Ollama do not expose a trusted chat-template tokenizer. [`HostedChatModel`] therefore reports a
//! conservative upper bound: every byte-level BPE or byte-fallback token covers at least one UTF-8
//! byte, so the bounded serialized request size can only over-count. Prompts are trimmed earlier
//! than strictly necessary, never sent over the configured context capacity on our side.

use std::time::Duration;

use polaris_config::{PolarisConfig, ProviderName, SecretRef};
use polaris_sdk::{
    effort::Effort,
    ids::{ModelId, TokenCount},
    registry::{ChatModel, ChatRequest, ModelError, PromptTokenCount, PromptTokenizer},
    secrets::SecretSource,
};

use crate::{
    ApiKey, BaseUrl, ClientConfig, CredentialError, ProviderStream, ReasoningReplay, TfError,
    TokenFactoryClient, client::chat_request_input_bound,
};

/// Bounded timeouts and sizes for interactive chat against a hosted provider.
#[must_use]
pub const fn hosted_client_config() -> ClientConfig {
    ClientConfig {
        request_timeout: Duration::from_secs(300),
        connect_timeout: Duration::from_secs(15),
        stream_idle_timeout: Duration::from_secs(120),
        max_sse_frame_bytes: 256 * 1024,
        max_events_per_chunk: 256,
        max_chat_stream_bytes: 64 * 1024 * 1024,
        max_text_bytes: 32 * 1024 * 1024,
        max_tool_argument_bytes: 8 * 1024 * 1024,
        max_tool_calls: 128,
        max_json_response_bytes: 16 * 1024 * 1024,
        max_request_bytes: 16 * 1024 * 1024,
        max_429_retries: 3,
        max_5xx_retries: 2,
        retry_base_delay: Duration::from_millis(500),
        max_retry_delay: Duration::from_secs(20),
        reasoning_replay: ReasoningReplay::Disabled,
    }
}

/// Short-deadline policy used only to probe a local server's model list.
const fn probe_client_config() -> ClientConfig {
    let mut config = hosted_client_config();
    config.request_timeout = Duration::from_secs(3);
    config.connect_timeout = Duration::from_secs(2);
    config.stream_idle_timeout = Duration::from_secs(3);
    config.max_429_retries = 0;
    config.max_5xx_retries = 0;
    config
}

/// One selected model on an OpenAI-compatible endpoint, counted with a conservative bound.
pub struct HostedChatModel {
    client: TokenFactoryClient,
    model: ModelId,
    max_context_tokens: TokenCount,
}

impl std::fmt::Debug for HostedChatModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostedChatModel")
            .field("model", &self.model)
            .field("max_context_tokens", &self.max_context_tokens)
            .field("client", &"[REDACTED]")
            .finish()
    }
}

impl HostedChatModel {
    /// Bind one model id and its context capacity to a configured client.
    /// # Errors
    /// Returns Configuration when the context capacity is zero.
    pub fn new(
        client: TokenFactoryClient,
        model: ModelId,
        max_context_tokens: TokenCount,
    ) -> Result<Self, TfError> {
        if max_context_tokens.get() == 0 {
            return Err(TfError::Configuration);
        }
        Ok(Self {
            client,
            model,
            max_context_tokens,
        })
    }

    /// The model every request must name.
    #[must_use]
    pub const fn model(&self) -> &ModelId {
        &self.model
    }

    /// The configured context capacity.
    #[must_use]
    pub const fn max_context_tokens(&self) -> TokenCount {
        self.max_context_tokens
    }
}

impl ChatModel for HostedChatModel {
    type Stream = ProviderStream;

    async fn stream(&self, req: ChatRequest) -> Result<Self::Stream, ModelError> {
        if req.model != self.model {
            return Err(ModelError::BadRequest(
                "request names a model other than the selected one".to_owned(),
            ));
        }
        self.client.stream(req).await
    }
}

impl PromptTokenizer for HostedChatModel {
    async fn count_prompt(&self, request: &ChatRequest) -> Result<PromptTokenCount, ModelError> {
        let bound = chat_request_input_bound(request).map_err(ModelError::from)?;
        let input_tokens = u32::try_from(bound).unwrap_or(u32::MAX);
        Ok(PromptTokenCount {
            input_tokens: TokenCount::new(input_tokens),
            max_context_tokens: self.max_context_tokens,
        })
    }
}

/// Why a provider in `provider.order` was not selected. Never carries secret material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// No credential reference is configured.
    NoCredentialReference,
    /// The configured reference holds no value.
    CredentialMissing,
    /// The credential source failed or returned an unusable value.
    CredentialUnavailable,
    /// The provider needs a setting that is unset (for example `local.model`).
    NotConfigured,
    /// The configured base URL or capacity is invalid for this provider.
    InvalidConfiguration,
    /// The local server did not answer its model list in time.
    Unreachable,
    /// The local server answered but does not serve the configured model.
    ModelNotServed,
}

impl SkipReason {
    /// Stable human-readable explanation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoCredentialReference => "no API key reference configured",
            Self::CredentialMissing => "API key is not set",
            Self::CredentialUnavailable => "API key could not be read",
            Self::NotConfigured => "no model configured",
            Self::InvalidConfiguration => "invalid provider configuration",
            Self::Unreachable => "server is not reachable",
            Self::ModelNotServed => "server does not serve the configured model",
        }
    }
}

/// A provider that was considered and skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkippedProvider {
    /// The skipped provider.
    pub provider: ProviderName,
    /// Why it was skipped.
    pub reason: SkipReason,
}

/// Credentials resolved for the hosted providers in `provider.order`.
///
/// Resolution reads the secret source synchronously; run it on a blocking task.
#[derive(Debug, Default)]
pub struct ProviderCredentials {
    token_factory: Option<Result<ApiKey, SkipReason>>,
    openrouter: Option<Result<ApiKey, SkipReason>>,
}

impl ProviderCredentials {
    /// Resolve the API keys of hosted providers listed in `provider.order`.
    #[must_use]
    pub fn resolve(config: &PolarisConfig, source: &(impl SecretSource + ?Sized)) -> Self {
        let mut credentials = Self::default();
        for provider in &config.provider.order {
            match provider {
                ProviderName::TokenFactory => {
                    credentials.token_factory =
                        Some(resolve_key(config.nebius.api_key.as_ref(), source));
                }
                ProviderName::OpenRouter => {
                    credentials.openrouter =
                        Some(resolve_key(config.openrouter.api_key.as_ref(), source));
                }
                ProviderName::Local => {}
            }
        }
        credentials
    }
}

fn resolve_key(
    reference: Option<&SecretRef>,
    source: &(impl SecretSource + ?Sized),
) -> Result<ApiKey, SkipReason> {
    let reference = reference.ok_or(SkipReason::NoCredentialReference)?;
    let location = reference
        .location()
        .map_err(|_| SkipReason::InvalidConfiguration)?;
    ApiKey::from_source(source, &location).map_err(|error| match error {
        CredentialError::Missing => SkipReason::CredentialMissing,
        CredentialError::Source(_) | CredentialError::Invalid => SkipReason::CredentialUnavailable,
    })
}

/// The provider and model serving this session.
#[derive(Debug)]
pub struct SelectedProvider {
    /// The chosen provider.
    pub provider: ProviderName,
    /// The model bound to it for the requested effort.
    pub model: HostedChatModel,
}

/// Result of walking `provider.order`: the first usable provider, plus every skip reason.
#[derive(Debug, Default)]
pub struct ProviderChoice {
    /// The selected provider, or `None` when the session must run offline.
    pub selected: Option<SelectedProvider>,
    /// Providers considered before the selection (or all of them when offline).
    pub skipped: Vec<SkippedProvider>,
}

/// Walk `provider.order` and select the first usable provider for `effort`.
///
/// Hosted providers are usable when their credential resolved; they are not contacted here, so a
/// later request can still fail (for example on a revoked key). The local provider is usable when
/// `local.model` is set and its server lists that model. The selection is made once per session:
/// a frame is bound to one model identity, so there is no silent mid-run provider switch.
pub async fn select_provider(
    config: &PolarisConfig,
    effort: Effort,
    credentials: ProviderCredentials,
) -> ProviderChoice {
    let ProviderCredentials {
        mut token_factory,
        mut openrouter,
    } = credentials;
    let mut choice = ProviderChoice::default();
    for &provider in &config.provider.order {
        let attempt = match provider {
            ProviderName::TokenFactory => hosted(
                &config.nebius.base_url,
                token_factory.take(),
                config.nebius.effort.model(effort),
                config.nebius.context_tokens,
            ),
            ProviderName::OpenRouter => hosted(
                &config.openrouter.base_url,
                openrouter.take(),
                config.openrouter.effort.model(effort),
                config.openrouter.context_tokens,
            ),
            ProviderName::Local => local(config).await,
        };
        match attempt {
            Ok(model) => {
                choice.selected = Some(SelectedProvider { provider, model });
                return choice;
            }
            Err(reason) => choice.skipped.push(SkippedProvider { provider, reason }),
        }
    }
    choice
}

fn hosted(
    base_url: &str,
    key: Option<Result<ApiKey, SkipReason>>,
    model: &ModelId,
    context_tokens: u32,
) -> Result<HostedChatModel, SkipReason> {
    let key = key.ok_or(SkipReason::NoCredentialReference)??;
    let base = BaseUrl::parse(base_url).map_err(|_| SkipReason::InvalidConfiguration)?;
    let client = TokenFactoryClient::new(base, key, hosted_client_config())
        .map_err(|_| SkipReason::InvalidConfiguration)?;
    HostedChatModel::new(client, model.clone(), TokenCount::new(context_tokens))
        .map_err(|_| SkipReason::InvalidConfiguration)
}

async fn local(config: &PolarisConfig) -> Result<HostedChatModel, SkipReason> {
    let model = config
        .local
        .model
        .as_ref()
        .ok_or(SkipReason::NotConfigured)?;
    let base = BaseUrl::parse(&config.local.base_url).map_err(|_| SkipReason::InvalidConfiguration)?;
    let probe = TokenFactoryClient::anonymous_loopback(base.clone(), probe_client_config())
        .map_err(|_| SkipReason::InvalidConfiguration)?;
    let served = probe
        .list_models()
        .await
        .map_err(|_| SkipReason::Unreachable)?;
    if !served.iter().any(|info| info.id == model.as_str()) {
        return Err(SkipReason::ModelNotServed);
    }
    let client = TokenFactoryClient::anonymous_loopback(base, hosted_client_config())
        .map_err(|_| SkipReason::InvalidConfiguration)?;
    HostedChatModel::new(
        client,
        model.clone(),
        TokenCount::new(config.local.context_tokens),
    )
    .map_err(|_| SkipReason::InvalidConfiguration)
}
