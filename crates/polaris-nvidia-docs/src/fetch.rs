//! Incremental download of documentation pages: only NVIDIA documentation hosts over HTTPS,
//! conditional requests from the last ETag / Last-Modified, a bounded body, and a pause between
//! requests to the same host so a refresh never hammers a docs site.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::NvidiaError;
use crate::docs::{DocChunk, chunk_markdown, html_to_markdown};
use crate::profile::DocTopic;

/// Hosts documentation is fetched from; their published docs may be indexed for local lookup.
pub const DOC_HOSTS: &[&str] = &[
    "docs.nvidia.com",
    "developer.nvidia.com",
    "nvidia.github.io",
    "raw.githubusercontent.com",
];
/// Largest page body accepted, in bytes.
pub const MAX_PAGE_BYTES: usize = 8 * 1024 * 1024;

/// What the server said about a page the last time it was fetched.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Validators {
    /// `ETag` response header.
    pub etag: Option<String>,
    /// `Last-Modified` response header.
    pub last_modified: Option<String>,
}

/// The result of one conditional fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchOutcome {
    /// New content and its validators.
    Updated {
        /// Page body as text.
        body: String,
        /// Whether the body is HTML (else Markdown or plain text).
        html: bool,
        /// Validators for the next refresh.
        validators: Validators,
    },
    /// The page has not changed since `validators` were recorded.
    NotModified,
}

/// Fetches pages politely and remembers their validators.
#[derive(Debug)]
pub struct DocFetcher {
    client: reqwest::Client,
    spacing: Duration,
    allow_loopback: bool,
    last_request: BTreeMap<String, Instant>,
    validators: BTreeMap<String, Validators>,
}

impl DocFetcher {
    /// A fetcher that waits `spacing` between requests to one host.
    /// # Errors
    /// The HTTP client could not be built.
    pub fn new(spacing: Duration) -> Result<Self, NvidiaError> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("polaris-nvidia-docs/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .map_err(|_| NvidiaError::Fetch("client".to_owned()))?;
        Ok(Self {
            client,
            spacing,
            allow_loopback: false,
            last_request: BTreeMap::new(),
            validators: BTreeMap::new(),
        })
    }

    /// Also allow plain-HTTP loopback servers (for tests and local mirrors).
    #[must_use]
    pub const fn allowing_loopback(mut self) -> Self {
        self.allow_loopback = true;
        self
    }

    /// Validators recorded so far, by URL (persist them with the seed).
    #[must_use]
    pub const fn validators(&self) -> &BTreeMap<String, Validators> {
        &self.validators
    }

    /// Start from validators saved by an earlier refresh.
    pub fn remember(&mut self, validators: BTreeMap<String, Validators>) {
        self.validators = validators;
    }

    /// Fetch `url`, conditionally when it was fetched before.
    /// # Errors
    /// A disallowed URL, a network or HTTP error, or an oversized body.
    pub async fn fetch(&mut self, url: &str) -> Result<FetchOutcome, NvidiaError> {
        let parsed = url::Url::parse(url).map_err(|_| NvidiaError::Malformed("URL"))?;
        let host = parsed.host_str().unwrap_or_default().to_owned();
        let loopback = matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]");
        let allowed = (parsed.scheme() == "https" && DOC_HOSTS.contains(&host.as_str()))
            || (self.allow_loopback && loopback && parsed.scheme() == "http");
        if !allowed {
            return Err(NvidiaError::Fetch(format!("{host} is not a documentation host")));
        }
        if let Some(last) = self.last_request.get(&host) {
            tokio::time::sleep_until(*last + self.spacing).await;
        }
        self.last_request.insert(host, Instant::now());
        let mut request = self.client.get(parsed);
        if let Some(previous) = self.validators.get(url) {
            if let Some(etag) = &previous.etag {
                request = request.header(reqwest::header::IF_NONE_MATCH, etag);
            }
            if let Some(modified) = &previous.last_modified {
                request = request.header(reqwest::header::IF_MODIFIED_SINCE, modified);
            }
        }
        let response = request
            .send()
            .await
            .map_err(|error| NvidiaError::Fetch(error.without_url().to_string()))?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(FetchOutcome::NotModified);
        }
        if !response.status().is_success() {
            return Err(NvidiaError::Fetch(format!("HTTP {}", response.status().as_u16())));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_PAGE_BYTES as u64)
        {
            return Err(NvidiaError::TooLarge("documentation page"));
        }
        let header = |name: reqwest::header::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        let validators = Validators {
            etag: header(reqwest::header::ETAG),
            last_modified: header(reqwest::header::LAST_MODIFIED),
        };
        let html = header(reqwest::header::CONTENT_TYPE)
            .is_some_and(|kind| kind.to_ascii_lowercase().contains("html"));
        let mut body = Vec::new();
        let mut stream = response;
        while let Some(chunk) = stream
            .chunk()
            .await
            .map_err(|error| NvidiaError::Fetch(error.without_url().to_string()))?
        {
            if body.len().saturating_add(chunk.len()) > MAX_PAGE_BYTES {
                return Err(NvidiaError::TooLarge("documentation page"));
            }
            body.extend_from_slice(&chunk);
        }
        let body = String::from_utf8(body).map_err(|_| NvidiaError::Malformed("page encoding"))?;
        self.validators.insert(url.to_owned(), validators.clone());
        Ok(FetchOutcome::Updated {
            body,
            html,
            validators,
        })
    }

    /// Fetch `url` and split it into chunks of `topic`; `None` when it has not changed.
    /// # Errors
    /// As [`Self::fetch`], or a page that cannot be converted.
    pub async fn ingest(
        &mut self,
        url: &str,
        topic: DocTopic,
        max_chars: usize,
    ) -> Result<Option<Vec<DocChunk>>, NvidiaError> {
        match self.fetch(url).await? {
            FetchOutcome::NotModified => Ok(None),
            FetchOutcome::Updated { body, html, .. } => {
                let markdown = if html { html_to_markdown(&body)? } else { body };
                chunk_markdown(&markdown, topic, url, max_chars).map(Some)
            }
        }
    }
}
