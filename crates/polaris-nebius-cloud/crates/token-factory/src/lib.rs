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
//! - Keep stream allocations bounded by configured limits; zero-allocation performance is not claimed and is unmeasured.
//!
//! ## API
//!
//! - `struct TokenFactoryClient`
//! - `impl ChatModel`
//! - `impl EmbeddingProvider`
//!
//! ## Internal dependencies
//!
//! `polaris-sdk`, `polaris-config`, `polaris-log`, `polaris-ratelimit`, `polaris-telemetry`
//!
//! ## External dependencies
//!
//! `reqwest`, `futures`, `serde`, `serde_json`, `bytes`, `tokio`, `secrecy`, `url`

mod client;
mod error;
mod sse;

pub use client::{
    ApiKey, BaseUrl, ClientConfig, EmbeddingBatch, EmbeddingClient, EmbeddingConfig, ModelInfo,
    ProviderStream, ReasoningReplay, TokenFactoryClient,
};
pub use error::TfError;
pub use sse::{SseEvent, SseParser};
