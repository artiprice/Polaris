//! # polaris-nebius-cloud-token-factory
//!
//! OpenAI-compatible Token Factory HTTP adapter for streaming chat, embeddings, and model listing.
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L2 — adapters & infrastructure |
//! | Kind | lib |
//! | Phase | P3 |
//! | Parent facade | `polaris-nebius-cloud` |
//! | Status | implemented adapter; provider-specific live contract remains unverified |
//!
//! ## Responsibilities
//!
//! - Implement the SDK chat and embedding traits over an explicitly configured OpenAI-compatible base URL.
//! - Parse bounded SSE frames incrementally and preserve text, isolated reasoning, tool-call, finish, and usage chunks.
//! - Keep credentials opaque and redact transport diagnostics; bound requests, responses, retries, and deadlines.
//! - Return only provider-reported model identifiers and embedding dimensions; this crate does not assert
//!   model capabilities or telemetry/rate-limit reconciliation that the endpoint has not verified.
//! - Route borrowed SSE frames without allocations after parser initialization; JSON decoding and
//!   owned SDK output can allocate, and those costs are not claimed allocation-free.
//!
//! ## API
//!
//! - `struct TokenFactoryClient`
//! - `impl ChatModel`
//! - `impl EmbeddingProvider`
//! - Files and fine-tuning (`upload_file`, `create_fine_tuning_job`, `fine_tuning_job`,
//!   `list_fine_tuning_jobs`) per the OpenAI-compatible Token Factory API reference; not run live.
//!
//! ## Internal dependencies
//!
//! `polaris-sdk`, `polaris-config`, `polaris-log`, `polaris-ratelimit`, `polaris-telemetry`
//!
//! ## External dependencies
//!
//! Runtime: `reqwest`, `futures`, `serde`, `serde_json`, `bytes`, `tokio`, `secrecy`, `url`, `rand`, `jiff`.
//! Tests: `proptest` and `eventsource-stream` (differential SSE oracle only).

mod client;
mod error;
mod hosted;
mod sse;

pub use hosted::{
    HostedChatModel, ProviderChoice, ProviderCredentials, SelectedProvider, SkipReason,
    SkippedProvider, hosted_client_config, provider_model, select_provider,
};

pub use client::{
    ApiKey, BaseUrl, ClientConfig, EmbeddingBatch, EmbeddingClient, EmbeddingConfig, FineTuningJob,
    FineTuningRequest, LlamaCppChatClient, LlamaCppExpectedProfile, ModelInfo,
    PromptTokenEstimator, ProviderFile, ProviderStream, RateLimitPolicy, ReasoningReplay,
    TokenFactoryClient, VllmChatClient,
};
pub use error::{CredentialError, TfError};
pub use sse::{SseEvent, SseEventRef, SseParser};
