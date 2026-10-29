use crate::{
    SearchProviderError,
    fetch::retry_after,
    runtime::ProviderRuntime,
    searxng::{read_json_body, redact_query, search_error},
    ssrf::validate_base_url,
};
use polaris_sdk::registry::{
    Redactor, SearchError, SearchHit, SearchProvider, SearchQuery, WebSearch,
};
use reqwest::{Client, StatusCode, header, redirect::Policy};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde_json::json;
use std::{fmt, sync::Arc, time::Duration};
use url::Url;

const MAX_PROVIDER_RESPONSE_BYTES: u64 = 2_097_152;

/// Tavily adapter configuration.
#[derive(Debug)]
pub struct TavilyConfig {
    /// Tavily API base URL, normally `https://api.tavily.com/`.
    pub base_url: Url,
    /// Credential injected by the composition root. No environment lookup is performed here.
    pub api_key: SecretString,
    /// Request deadline.
    pub timeout: Duration,
    /// Maximum decoded JSON response size.
    pub max_response_bytes: u64,
    /// Permit plain HTTP only for an explicit loopback fixture endpoint.
    pub allow_http_loopback: bool,
}

/// Tavily Web Search adapter.
pub struct Tavily {
    endpoint: Url,
    local_origin: Option<Url>,
    api_key: SecretString,
    timeout: Duration,
    max_response_bytes: u64,
    runtime: ProviderRuntime,
    redactor: Arc<dyn Redactor>,
}

impl fmt::Debug for Tavily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Tavily")
            .field("endpoint", &self.endpoint)
            .field("timeout", &self.timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("runtime", &self.runtime)
            .finish_non_exhaustive()
    }
}

impl Tavily {
    /// Creates a Tavily provider with an injected secret, SDK redactor, and request runtime.
    ///
    /// # Errors
    /// Returns an error if the URL, credential, timeout, or response limit is invalid.
    pub fn new(
        config: TavilyConfig,
        runtime: ProviderRuntime,
        redactor: Arc<dyn Redactor>,
    ) -> Result<Self, SearchProviderError> {
        validate_base_url(&config.base_url, config.allow_http_loopback)?;
        if config.api_key.expose_secret().trim().is_empty()
            || config.api_key.expose_secret().len() > 4_096
            || config.timeout.is_zero()
            || config.timeout > crate::MAX_PROVIDER_TIMEOUT
            || config.max_response_bytes == 0
            || config.max_response_bytes > MAX_PROVIDER_RESPONSE_BYTES
        {
            return Err(SearchProviderError::ProviderConfig);
        }
        let mut base_url = config.base_url.clone();
        if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(path.as_str());
        }
        let endpoint = base_url
            .join("search")
            .map_err(|_| SearchProviderError::ProviderConfig)?;
        let local_origin = if config.allow_http_loopback && base_url.scheme() == "http" {
            Some(base_url)
        } else {
            None
        };
        Ok(Self {
            endpoint,
            local_origin,
            api_key: config.api_key,
            timeout: config.timeout,
            max_response_bytes: config.max_response_bytes,
            runtime,
            redactor,
        })
    }
}

impl WebSearch for Tavily {
    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>, SearchError> {
        if query.text.trim().is_empty() || query.max_results == 0 {
            return Err(SearchError::BadQuery(
                "empty query or result limit".to_owned(),
            ));
        }
        let redacted =
            redact_query(self.redactor.as_ref(), query.text.as_str()).map_err(search_error)?;
        let mut admission = self
            .runtime
            .acquire(self.timeout)
            .await
            .map_err(search_error)?;
        let target = tokio::time::timeout(
            self.timeout,
            crate::ssrf::validate_target(&self.endpoint, self.local_origin.as_ref()),
        )
        .await
        .map_err(|_| search_error(SearchProviderError::Unavailable))?
        .map_err(search_error)?;
        let client = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .timeout(self.timeout)
            .resolve_to_addrs(target.host.as_str(), target.addresses.as_slice())
            .build()
            .map_err(|_| search_error(SearchProviderError::Unavailable))?;
        let mut body = json!({
            "query": redacted,
            "max_results": usize::from(query.max_results).min(20),
            "search_depth": "basic",
            "include_answer": false,
            "include_raw_content": false
        });
        if let Some(site) = &query.site {
            let site = redact_query(self.redactor.as_ref(), site.as_str()).map_err(search_error)?;
            if !valid_domain(site.as_str()) {
                admission.finish().map_err(search_error)?;
                return Err(SearchError::BadQuery("invalid site filter".to_owned()));
            }
            body["include_domains"] = json!([site]);
        }
        if let Some(days) = query.recency_days {
            let range = match days {
                0..=1 => "day",
                2..=7 => "week",
                8..=31 => "month",
                _ => "year",
            };
            body["time_range"] = json!(range);
        }
        admission.mark_dispatched().await.map_err(search_error)?;
        let response = match client
            .post(target.url)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT_ENCODING, "identity")
            .bearer_auth(self.api_key.expose_secret())
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(_) => {
                admission.finish().map_err(search_error)?;
                return Err(SearchError::Unavailable("Tavily request failed".to_owned()));
            }
        };
        if response.status().is_redirection() {
            admission.finish().map_err(search_error)?;
            return Err(SearchError::Protocol(
                "unexpected provider redirect".to_owned(),
            ));
        }
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            let retry = retry_after(&response);
            self.runtime
                .on_retry_after(429, retry)
                .map_err(search_error)?;
            admission.finish().map_err(search_error)?;
            return Err(search_error(SearchProviderError::RateLimited {
                retry_after: retry,
            }));
        }
        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            admission.finish().map_err(search_error)?;
            return Err(SearchError::Unauthorized);
        }
        if !response.status().is_success() {
            let status = response.status().as_u16();
            admission.finish().map_err(search_error)?;
            return Err(search_error(SearchProviderError::Status(status)));
        }
        let bytes = match read_json_body(response, self.max_response_bytes).await {
            Ok(bytes) => bytes,
            Err(error) => {
                admission.finish().map_err(search_error)?;
                return Err(search_error(error));
            }
        };
        let result: TavilyResponse = match serde_json::from_slice(&bytes) {
            Ok(result) => result,
            Err(_) => {
                admission.finish().map_err(search_error)?;
                return Err(SearchError::Protocol(
                    "invalid provider response".to_owned(),
                ));
            }
        };
        admission.finish().map_err(search_error)?;
        let limit = usize::from(query.max_results).min(20);
        Ok(result
            .results
            .into_iter()
            .take(limit)
            .filter_map(|hit| {
                result_url(hit.url.as_str()).map(|url| SearchHit {
                    url,
                    title: hit.title,
                    snippet: hit.content,
                    provider: SearchProvider::Tavily,
                })
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct TavilyResponse {
    #[serde(default)]
    results: Vec<TavilyHit>,
}

#[derive(Debug, Deserialize)]
struct TavilyHit {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    content: String,
}

fn valid_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && label
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_alphanumeric())
                && label
                    .bytes()
                    .last()
                    .is_some_and(|byte| byte.is_ascii_alphanumeric())
        })
}

fn result_url(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url.to_string())
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-search/crates/providers/src/tavily.rs"]
mod tests;
