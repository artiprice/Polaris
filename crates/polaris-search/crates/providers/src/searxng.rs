use crate::{
    SearchProviderError, fetch::retry_after, runtime::ProviderRuntime, ssrf::validate_base_url,
};
use polaris_sdk::registry::{
    Redactor, SearchError, SearchHit, SearchProvider, SearchQuery, WebSearch,
};
use reqwest::{Client, StatusCode, header, redirect::Policy};
use serde::Deserialize;
use std::{fmt, sync::Arc, time::Duration};
use url::Url;

const MAX_PROVIDER_RESPONSE_BYTES: u64 = 2_097_152;

/// SearXNG adapter configuration.
#[derive(Debug, Clone)]
pub struct SearxngConfig {
    /// Base URL of the trusted SearXNG instance.
    pub base_url: Url,
    /// Configured engine names allowed for requests.
    pub engines: Vec<String>,
    /// Request deadline.
    pub timeout: Duration,
    /// Maximum decoded JSON response size.
    pub max_response_bytes: u64,
    /// Permit plain HTTP only when the base host is an explicit loopback address/name.
    pub allow_http_loopback: bool,
}

/// SearXNG JSON search provider.
pub struct Searxng {
    endpoint: Url,
    base_url: Url,
    engines: String,
    timeout: Duration,
    max_response_bytes: u64,
    runtime: ProviderRuntime,
    redactor: Arc<dyn Redactor>,
}

impl fmt::Debug for Searxng {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Searxng")
            .field("endpoint", &self.endpoint)
            .field("engines", &self.engines)
            .field("timeout", &self.timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("runtime", &self.runtime)
            .finish_non_exhaustive()
    }
}

impl Searxng {
    /// Creates a SearXNG provider using an injected redactor and request runtime.
    ///
    /// # Errors
    /// Returns an error if the base URL, engine list, timeout, or response limit is invalid.
    pub fn new(
        config: SearxngConfig,
        runtime: ProviderRuntime,
        redactor: Arc<dyn Redactor>,
    ) -> Result<Self, SearchProviderError> {
        validate_base_url(&config.base_url, config.allow_http_loopback)?;
        if config.engines.is_empty()
            || config.timeout.is_zero()
            || config.timeout > crate::MAX_PROVIDER_TIMEOUT
            || config.max_response_bytes == 0
            || config.max_response_bytes > MAX_PROVIDER_RESPONSE_BYTES
            || config.engines.len() > 16
            || config.engines.iter().any(|engine| {
                engine.is_empty()
                    || engine.len() > 128
                    || !engine
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            })
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
        let engines = config.engines.join(",");
        Ok(Self {
            endpoint,
            base_url,
            engines,
            timeout: config.timeout,
            max_response_bytes: config.max_response_bytes,
            runtime,
            redactor,
        })
    }
}

impl WebSearch for Searxng {
    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>, SearchError> {
        if query.text.trim().is_empty() || query.max_results == 0 {
            return Err(SearchError::BadQuery(
                "empty query or result limit".to_owned(),
            ));
        }
        let mut raw_query = query.text.clone();
        if let Some(site) = &query.site {
            if site.is_empty() || site.bytes().any(|byte| byte.is_ascii_whitespace()) {
                return Err(SearchError::BadQuery("invalid site filter".to_owned()));
            }
            raw_query.push_str(" site:");
            raw_query.push_str(site);
        }
        let redacted =
            redact_query(self.redactor.as_ref(), raw_query.as_str()).map_err(search_error)?;
        let mut admission = self
            .runtime
            .acquire(self.timeout)
            .await
            .map_err(search_error)?;
        let target = tokio::time::timeout(
            self.timeout,
            crate::ssrf::validate_target(&self.endpoint, Some(&self.base_url)),
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
        admission.mark_dispatched().await.map_err(search_error)?;
        let mut parameters = vec![
            ("q", redacted.as_str()),
            ("format", "json"),
            ("engines", self.engines.as_str()),
        ];
        if let Some(days) = query.recency_days {
            let range = match days {
                0..=1 => "day",
                2..=7 => "week",
                8..=31 => "month",
                _ => "year",
            };
            parameters.push(("time_range", range));
        }
        let request = client
            .get(target.url)
            .query(&parameters)
            .header(header::ACCEPT_ENCODING, "identity");
        let response = match request.send().await {
            Ok(response) => response,
            Err(_) => {
                admission.finish().map_err(search_error)?;
                return Err(SearchError::Unavailable(
                    "SearXNG request failed".to_owned(),
                ));
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
        let result: SearxResponse = match serde_json::from_slice(&bytes) {
            Ok(result) => result,
            Err(_) => {
                admission.finish().map_err(search_error)?;
                return Err(SearchError::Protocol(
                    "invalid provider response".to_owned(),
                ));
            }
        };
        admission.finish().map_err(search_error)?;
        let limit = usize::from(query.max_results);
        Ok(result
            .results
            .into_iter()
            .take(limit)
            .filter_map(|hit| {
                result_url(hit.url.as_str()).map(|url| SearchHit {
                    url,
                    title: hit.title,
                    snippet: hit.content,
                    provider: SearchProvider::Searxng,
                })
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct SearxResponse {
    #[serde(default)]
    results: Vec<SearxHit>,
}

#[derive(Debug, Deserialize)]
struct SearxHit {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    content: String,
}

pub(crate) fn redact_query(
    redactor: &dyn Redactor,
    input: &str,
) -> Result<String, SearchProviderError> {
    if input.len() > 16_384 {
        return Err(SearchProviderError::ProviderConfig);
    }
    let mut output = Vec::with_capacity(input.len());
    let stats = redactor.redact_into(input.as_bytes(), &mut output);
    if stats.bytes_in != input.len() || output.is_empty() || output.len() > 65_536 {
        return Err(SearchProviderError::Redaction);
    }
    String::from_utf8(output).map_err(|_| SearchProviderError::Redaction)
}

pub(crate) async fn read_json_body(
    mut response: reqwest::Response,
    max_bytes: u64,
) -> Result<Vec<u8>, SearchProviderError> {
    if response
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|encoding| !encoding.eq_ignore_ascii_case("identity"))
    {
        return Err(SearchProviderError::UnsupportedContent);
    }
    if !response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(';').next().is_some_and(|media_type| {
                media_type.trim().eq_ignore_ascii_case("application/json")
            })
        })
    {
        return Err(SearchProviderError::UnsupportedContent);
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(SearchProviderError::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| SearchProviderError::Unavailable)?
    {
        let chunk_len = u64::try_from(chunk.len()).map_err(|_| SearchProviderError::TooLarge)?;
        let body_len = u64::try_from(bytes.len())
            .ok()
            .and_then(|length| length.checked_add(chunk_len))
            .ok_or(SearchProviderError::TooLarge)?;
        if body_len > max_bytes {
            return Err(SearchProviderError::TooLarge);
        }
        bytes.extend_from_slice(chunk.as_ref());
    }
    Ok(bytes)
}

pub(crate) fn search_error(error: SearchProviderError) -> SearchError {
    match error {
        SearchProviderError::Unavailable => {
            SearchError::Unavailable("search provider unavailable".to_owned())
        }
        SearchProviderError::Status(_) => {
            SearchError::Protocol("search provider returned an error".to_owned())
        }
        SearchProviderError::RateLimited { retry_after } => SearchError::RateLimited {
            retry_after_ms: retry_after.and_then(|delay| delay.as_millis().try_into().ok()),
        },
        SearchProviderError::Blocked(_) | SearchProviderError::ProviderConfig => {
            SearchError::Protocol("search provider configuration or policy error".to_owned())
        }
        SearchProviderError::TooLarge
        | SearchProviderError::UnsupportedContent
        | SearchProviderError::Decode
        | SearchProviderError::Redaction
        | SearchProviderError::RateLimit
        | SearchProviderError::Extraction => {
            SearchError::Protocol("search provider returned an invalid response".to_owned())
        }
    }
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
