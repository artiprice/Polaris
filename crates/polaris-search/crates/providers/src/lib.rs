#![forbid(unsafe_code)]
//! # Polaris search providers
//!
//! Production Tavily and SearXNG adapters plus an SSRF-hardened, bounded page fetcher. Search
//! results retain their remote provenance, and fetched Markdown is always [`FetchedTrust::Untrusted`].
//!
//! Search requests apply the injected SDK [`Redactor`] before network egress. Each adapter uses a
//! caller-configured [`ProviderRuntime`] for request admission and concurrency. The fetcher resolves
//! and validates every destination, pins the accepted DNS answers into a dedicated reqwest client,
//! disables implicit redirects and proxies, then repeats validation for each redirect hop.

mod error;
mod fetch;
mod runtime;
mod searxng;
mod ssrf;
mod tavily;

pub use error::{BlockReason, SearchProviderError};
pub use fetch::{
    FetchPolicy, FetchedPage, FetchedTrust, Fetcher, MAX_FETCH_BODY_BYTES, MAX_PROVIDER_TIMEOUT,
};
pub use runtime::ProviderRuntime;
pub use searxng::{Searxng, SearxngConfig};
pub use tavily::{Tavily, TavilyConfig};
