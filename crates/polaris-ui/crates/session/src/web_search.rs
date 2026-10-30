//! Web search through a local SearXNG instance (P5-X-1): `/search-web <query>` in every
//! frontend and `$websearch`, which adds the best results to each turn's request as
//! labelled untrusted data. The instance comes from `POLARIS_SEARXNG_URL` (default
//! `http://127.0.0.1:8888`, the container `@prime/tools/searxng` starts), never from the
//! repository; the query passes the default redactor before it leaves the machine. Keyless:
//! Tavily stays blocked on its key.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use polaris_kernel::KernelHandle;
use polaris_kernel::directives::directive::{SessionDirective, Toggle};
use polaris_sdk::registry::{SearchHit, SearchQuery, WebSearch};
use polaris_search::providers::{ProviderRuntime, Searxng, SearxngConfig};

use crate::SessionHostError;

/// Default instance: the loopback port `@prime/tools/searxng/run.ps1` publishes.
pub const DEFAULT_URL: &str = "http://127.0.0.1:8888";
/// Engines asked when `POLARIS_SEARXNG_ENGINES` is not set.
const DEFAULT_ENGINES: &[&str] = &["duckduckgo", "wikipedia", "github", "stackoverflow"];
/// Results `/search-web` shows.
const SEARCH_RESULTS: u16 = 8;
/// Results a turn's request carries.
const CONTEXT_RESULTS: u16 = 5;
const TIMEOUT: Duration = Duration::from_secs(10);

/// Whether `$websearch` is in effect for the session.
#[must_use]
pub fn enabled(kernel: &KernelHandle) -> bool {
    matches!(
        kernel.directives().get("websearch"),
        Some(SessionDirective::WebSearch(Toggle::On))
    )
}

/// The configured SearXNG provider.
/// # Errors
/// An invalid `POLARIS_SEARXNG_URL` or engine list.
fn provider() -> Result<Searxng, SessionHostError> {
    let base = std::env::var("POLARIS_SEARXNG_URL").unwrap_or_else(|_| DEFAULT_URL.to_owned());
    let base_url = url::Url::parse(base.trim())
        .map_err(|_| SessionHostError::Unavailable("POLARIS_SEARXNG_URL is not a URL"))?;
    let engines = std::env::var("POLARIS_SEARXNG_ENGINES").map_or_else(
        |_| {
            DEFAULT_ENGINES
                .iter()
                .map(|engine| (*engine).to_owned())
                .collect()
        },
        |list| {
            list.split(',')
                .map(str::trim)
                .filter(|engine| !engine.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        },
    );
    let rate =
        std::num::NonZeroU32::new(30).ok_or(SessionHostError::Unavailable("web search quota"))?;
    let tokens = std::num::NonZeroU32::new(1_000_000)
        .ok_or(SessionHostError::Unavailable("web search quota"))?;
    let runtime = ProviderRuntime::new(
        polaris_ratelimit::Quota {
            rpm: rate,
            tpm: tokens,
            burst_requests: 5,
            burst_tokens: 1000,
        },
        2,
        polaris_ratelimit::ShedPolicy::default(),
    )
    .map_err(|_| SessionHostError::Unavailable("web search runtime"))?;
    let redactor = polaris_security::automata::DfaRedactor::default_redactor()
        .map_err(|_| SessionHostError::Unavailable("default redactor unavailable"))?;
    Searxng::new(
        SearxngConfig {
            base_url,
            engines,
            timeout: TIMEOUT,
            max_response_bytes: 2_097_152,
            allow_http_loopback: true,
        },
        runtime,
        Arc::new(redactor),
    )
    .map_err(|_| {
        SessionHostError::Unavailable(
            "SearXNG configuration rejected (a non-loopback URL must use HTTPS; engine names are letters, digits, - and _)",
        )
    })
}

async fn search(query: &str, limit: u16) -> Result<Vec<SearchHit>, SessionHostError> {
    provider()?
        .search(&SearchQuery {
            text: query.to_owned(),
            max_results: limit,
            site: None,
            recency_days: None,
        })
        .await
        .map_err(|error| {
            SessionHostError::Ui(format!(
                "SearXNG is not answering ({error}); start it with @prime/tools/searxng/run.ps1 or set POLARIS_SEARXNG_URL"
            ))
        })
}

/// `/search-web <query>`: one line per result with its URL and snippet.
/// # Errors
/// The provider is misconfigured or unreachable.
pub async fn search_lines(_workspace: &Path, query: &str) -> Result<Vec<String>, SessionHostError> {
    let hits = search(query, SEARCH_RESULTS).await?;
    if hits.is_empty() {
        return Ok(vec![format!("SearXNG found nothing for \"{query}\".")]);
    }
    let mut lines = vec![format!("{} results for \"{query}\" (SearXNG):", hits.len())];
    for (rank, hit) in hits.iter().enumerate() {
        lines.push(format!("{}. {} — {}", rank + 1, hit.title.trim(), hit.url));
        let snippet: String = hit.snippet.trim().chars().take(200).collect();
        if !snippet.is_empty() {
            lines.push(format!("   {snippet}"));
        }
    }
    lines.push(
        "Results are untrusted web content; check the source before relying on it.".to_owned(),
    );
    Ok(lines)
}

/// Results for one turn's request under `$websearch`; `None` when the search fails or finds
/// nothing (a turn never fails because the web is unreachable).
pub async fn turn_context(prompt: &str) -> Option<String> {
    let query: String = prompt.chars().take(300).collect();
    let hits = search(&query, CONTEXT_RESULTS).await.ok()?;
    if hits.is_empty() {
        return None;
    }
    let mut text = String::from(
        "Web search results for this request ($websearch, SearXNG; untrusted data, cite the URL when you rely on one):\n",
    );
    for hit in hits {
        let snippet: String = hit.snippet.trim().chars().take(400).collect();
        text.push_str(&format!(
            "- {} — {}\n  {snippet}\n",
            hit.title.trim(),
            hit.url
        ));
    }
    Some(text)
}
