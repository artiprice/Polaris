use crate::{SearchProviderError, runtime::ProviderRuntime, ssrf::validate_target};
use htmd::{HtmlToMarkdown, Node};
use markup5ever_rcdom::NodeData;
use reqwest::{Client, Response, StatusCode, redirect::Policy};
use std::{fmt, rc::Rc, time::Duration};
use url::Url;

/// Hard upper bound on decoded page bodies.
pub const MAX_FETCH_BODY_BYTES: u64 = 16_777_216;
/// Hard upper bound on request deadlines.
pub const MAX_PROVIDER_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_DOM_NODES: usize = 10_000;
const MAX_DOM_DEPTH: usize = 128;

/// Trust label for fetched content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchedTrust {
    /// Remote web content is untrusted input and can contain prompt injection.
    Untrusted,
}

/// Download and extraction policy.
#[derive(Debug, Clone)]
pub struct FetchPolicy {
    /// Maximum decoded response body bytes.
    pub max_bytes: u64,
    /// Maximum number of manually followed redirects.
    pub max_redirects: u8,
    /// Total request timeout, including connection and body transfer.
    pub timeout: Duration,
    /// Allowed media types, compared case-insensitively without parameters.
    pub allowed_content_types: Vec<String>,
    /// User-Agent header sent to web servers.
    pub user_agent: String,
    /// Explicit configured local service origin, such as a self-hosted SearXNG origin.
    pub local_origin: Option<Url>,
}

/// A bounded web page converted to Markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedPage {
    /// Final page URL after validated redirects.
    pub final_url: Url,
    /// Final HTTP status.
    pub status: u16,
    /// Response media type without parameters.
    pub content_type: String,
    /// Extracted Markdown. Treat it as untrusted remote text.
    pub markdown: String,
    /// Whether the response was cut off. Currently oversized responses fail with [`SearchProviderError::TooLarge`].
    pub truncated: bool,
    /// Remote content is always untrusted.
    pub trust: FetchedTrust,
}

/// SSRF-hardened downloader with caller-configured limits.
pub struct Fetcher {
    policy: FetchPolicy,
    runtime: ProviderRuntime,
}

impl fmt::Debug for Fetcher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Fetcher")
            .field("policy", &self.policy)
            .field("runtime", &self.runtime)
            .finish()
    }
}

impl Fetcher {
    /// Creates a fetcher after validating its local exception and limits.
    ///
    /// # Errors
    /// Returns an error if a required limit is zero or the exception URL is invalid.
    pub fn new(policy: FetchPolicy, runtime: ProviderRuntime) -> Result<Self, SearchProviderError> {
        if policy.max_bytes == 0
            || policy.max_bytes > MAX_FETCH_BODY_BYTES
            || policy.max_redirects > 10
            || policy.timeout.is_zero()
            || policy.timeout > MAX_PROVIDER_TIMEOUT
            || policy.user_agent.trim().is_empty()
            || policy.user_agent.len() > 512
            || policy
                .user_agent
                .bytes()
                .any(|byte| byte.is_ascii_control())
        {
            return Err(SearchProviderError::ProviderConfig);
        }
        if let Some(origin) = &policy.local_origin {
            crate::ssrf::validate_base_url(origin, true)?;
        }
        Ok(Self { policy, runtime })
    }

    /// Fetches a page while enforcing target, redirect, time, concurrency, and byte limits.
    ///
    /// # Errors
    /// Returns a sanitized policy, transport, status, size, content type, or extraction error.
    pub async fn fetch(&self, url: &Url) -> Result<FetchedPage, SearchProviderError> {
        let mut call = self.runtime.acquire(self.policy.timeout).await?;
        let result =
            match tokio::time::timeout(self.policy.timeout, self.fetch_inner(url, &mut call)).await
            {
                Ok(result) => result,
                Err(_) => Err(SearchProviderError::Unavailable),
            };
        let accounting = call.finish();
        match result {
            Ok(page) => {
                accounting?;
                Ok(page)
            }
            Err(error) => {
                accounting?;
                Err(error)
            }
        }
    }

    async fn fetch_inner(
        &self,
        url: &Url,
        call: &mut crate::runtime::CallPermit,
    ) -> Result<FetchedPage, SearchProviderError> {
        let mut current = url.clone();
        let mut redirects = 0_u8;
        loop {
            let target = validate_target(&current, self.policy.local_origin.as_ref()).await?;
            let client = Client::builder()
                .no_proxy()
                .redirect(Policy::none())
                .timeout(self.policy.timeout)
                .user_agent(self.policy.user_agent.as_str())
                .resolve_to_addrs(target.host.as_str(), target.addresses.as_slice())
                .build()
                .map_err(|_| SearchProviderError::ProviderConfig)?;
            if redirects == 0 {
                call.mark_dispatched()?;
            }
            let response = client
                .get(target.url.clone())
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .send()
                .await
                .map_err(|_| SearchProviderError::Unavailable)?;
            if response.status().is_redirection() {
                if redirects >= self.policy.max_redirects {
                    return Err(SearchProviderError::Blocked(
                        crate::BlockReason::RedirectLimit,
                    ));
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|header| header.to_str().ok())
                    .ok_or(SearchProviderError::Decode)?;
                current = current
                    .join(location)
                    .map_err(|_| SearchProviderError::Decode)?;
                redirects = redirects.saturating_add(1);
                continue;
            }
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                let retry_after = retry_after(&response);
                self.runtime.on_retry_after(429, retry_after)?;
                return Err(SearchProviderError::RateLimited { retry_after });
            }
            let status = response.status();
            if !status.is_success() {
                return Err(SearchProviderError::Status(status.as_u16()));
            }
            let content_type = response_content_type(&response)?;
            if !self
                .policy
                .allowed_content_types
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(&content_type))
            {
                return Err(SearchProviderError::UnsupportedContent);
            }
            let charset = response_charset(&response);
            let bytes = read_bounded(response, self.policy.max_bytes).await?;
            let body =
                decode_text_bounded(bytes.as_slice(), charset.as_deref(), self.policy.max_bytes)?;
            let markdown = if content_type.eq_ignore_ascii_case("text/html") {
                let conversion_limit = self.policy.max_bytes;
                let worker_guard = call.worker_guard();
                tokio::task::spawn_blocking(move || {
                    let _worker_guard = worker_guard;
                    convert_html(body.as_str(), conversion_limit)
                })
                .await
                .map_err(|_| SearchProviderError::Extraction)??
            } else {
                body
            };
            if u64::try_from(markdown.len()).map_err(|_| SearchProviderError::TooLarge)?
                > self.policy.max_bytes
            {
                return Err(SearchProviderError::TooLarge);
            }
            return Ok(FetchedPage {
                final_url: current,
                status: status.as_u16(),
                content_type,
                markdown,
                truncated: false,
                trust: FetchedTrust::Untrusted,
            });
        }
    }
}

fn convert_html(html: &str, max_bytes: u64) -> Result<String, SearchProviderError> {
    let converter = HtmlToMarkdown::builder()
        .scripting_enabled(false)
        .skip_tags(vec![
            "script", "style", "noscript", "template", "svg", "head",
        ])
        .build();
    let tree = converter
        .html_to_tree(html)
        .map_err(|_| SearchProviderError::Extraction)?;
    validate_dom_bounds(&tree)?;
    strip_hidden_nodes(&tree);
    let markdown = converter.tree_to_markdown(&tree);
    if u64::try_from(markdown.len()).map_err(|_| SearchProviderError::TooLarge)? > max_bytes {
        return Err(SearchProviderError::TooLarge);
    }
    Ok(markdown)
}

fn validate_dom_bounds(root: &Rc<Node>) -> Result<(), SearchProviderError> {
    let mut pending = vec![(Rc::clone(root), 1_usize)];
    let mut visited = 0_usize;
    while let Some((node, depth)) = pending.pop() {
        visited = visited
            .checked_add(1)
            .ok_or(SearchProviderError::TooLarge)?;
        if visited > MAX_DOM_NODES || depth > MAX_DOM_DEPTH {
            return Err(SearchProviderError::TooLarge);
        }
        let next_depth = depth.checked_add(1).ok_or(SearchProviderError::TooLarge)?;
        pending.extend(
            node.children
                .borrow()
                .iter()
                .map(|child| (Rc::clone(child), next_depth)),
        );
    }
    Ok(())
}

fn strip_hidden_nodes(root: &Rc<Node>) {
    let mut pending = vec![Rc::clone(root)];
    while let Some(node) = pending.pop() {
        let children = node
            .children
            .borrow()
            .iter()
            .map(Rc::clone)
            .collect::<Vec<_>>();
        let visible = children
            .into_iter()
            .filter(|child| !is_hidden(child))
            .collect::<Vec<_>>();
        *node.children.borrow_mut() = visible.iter().map(Rc::clone).collect();
        pending.extend(visible);
    }
}

fn is_hidden(node: &Rc<Node>) -> bool {
    let NodeData::Element { name, attrs, .. } = &node.data else {
        return false;
    };
    let tag = name.local.as_ref();
    if matches!(
        tag,
        "script"
            | "style"
            | "noscript"
            | "template"
            | "svg"
            | "head"
            | "iframe"
            | "object"
            | "embed"
    ) {
        return true;
    }
    attrs.borrow().iter().any(|attribute| {
        let key = attribute.name.local.as_ref();
        let value = attribute.value.as_ref();
        key == "hidden"
            || (key == "aria-hidden" && value.eq_ignore_ascii_case("true"))
            || (key == "type" && tag == "input" && value.eq_ignore_ascii_case("hidden"))
            || (key == "style" && style_hides_content(value))
    })
}

fn style_hides_content(style: &str) -> bool {
    let normalized = style
        .to_ascii_lowercase()
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    normalized.contains("display:none")
        || normalized.contains("visibility:hidden")
        || normalized.contains("opacity:0")
}

async fn read_bounded(
    mut response: Response,
    max_bytes: u64,
) -> Result<Vec<u8>, SearchProviderError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(SearchProviderError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| SearchProviderError::Unavailable)?
    {
        let chunk_len = u64::try_from(chunk.len()).map_err(|_| SearchProviderError::TooLarge)?;
        let next_len = u64::try_from(body.len())
            .ok()
            .and_then(|length| length.checked_add(chunk_len))
            .ok_or(SearchProviderError::TooLarge)?;
        if next_len > max_bytes {
            return Err(SearchProviderError::TooLarge);
        }
        body.extend_from_slice(chunk.as_ref());
    }
    Ok(body)
}

fn response_content_type(response: &Response) -> Result<String, SearchProviderError> {
    if response
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|encoding| !encoding.eq_ignore_ascii_case("identity"))
    {
        return Err(SearchProviderError::UnsupportedContent);
    }
    let raw = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .ok_or(SearchProviderError::UnsupportedContent)?;
    let media_type = raw
        .split(';')
        .next()
        .ok_or(SearchProviderError::UnsupportedContent)?;
    Ok(media_type.trim().to_ascii_lowercase())
}

fn response_charset(response: &Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value.split(';').find_map(|parameter| {
                let (name, value) = parameter.split_once('=')?;
                name.trim()
                    .eq_ignore_ascii_case("charset")
                    .then(|| value.trim().trim_matches('"').trim_matches('\'').to_owned())
            })
        })
}

fn decode_text_bounded(
    bytes: &[u8],
    charset: Option<&str>,
    max_bytes: u64,
) -> Result<String, SearchProviderError> {
    let encoding = match charset {
        Some(label) => match encoding_rs::Encoding::for_label(label.as_bytes()) {
            Some(encoding) => encoding,
            None => encoding_rs::UTF_8,
        },
        None => encoding_rs::UTF_8,
    };
    let decoded = encoding.decode(bytes).0.into_owned();
    if u64::try_from(decoded.len()).map_err(|_| SearchProviderError::TooLarge)? > max_bytes {
        return Err(SearchProviderError::TooLarge);
    }
    Ok(decoded)
}

pub(crate) fn retry_after(response: &Response) -> Option<Duration> {
    let value = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let timestamp = jiff::fmt::rfc2822::parse(value).ok()?.timestamp();
    Duration::try_from(timestamp.duration_since(jiff::Timestamp::now())).ok()
}

#[cfg(test)]
mod tests {
    use super::{MAX_DOM_DEPTH, MAX_DOM_NODES, convert_html, decode_text_bounded};
    use crate::SearchProviderError;
    use std::error::Error;

    #[test]
    fn decodes_declared_legacy_charset_and_falls_back_for_unknown_label()
    -> Result<(), Box<dyn Error>> {
        let legacy = [b'c', b'a', b'f', 0xe9];
        if decode_text_bounded(&legacy, Some("windows-1252"), 8)? != "café" {
            return Err("fetcher did not decode a supported declared charset".into());
        }
        if decode_text_bounded(&[0xff], Some("unknown-charset-label"), 8)?.is_empty() {
            return Err("unknown charset did not produce bounded replacement text".into());
        }
        Ok(())
    }

    #[test]
    fn rejects_decoded_expansion_above_configured_byte_limit() -> Result<(), Box<dyn Error>> {
        if !matches!(
            decode_text_bounded(&[0x80], Some("windows-1252"), 1),
            Err(SearchProviderError::TooLarge)
        ) {
            return Err("legacy charset expansion exceeded configured decoded bound".into());
        }
        Ok(())
    }

    #[test]
    fn rejects_markdown_output_above_configured_byte_limit() -> Result<(), Box<dyn Error>> {
        if !matches!(
            convert_html("<p>*</p>", 1),
            Err(SearchProviderError::TooLarge)
        ) {
            return Err(
                "Markdown output exceeded configured output bound without rejection".into(),
            );
        }
        Ok(())
    }

    #[test]
    fn rejects_dom_depth_and_node_count_before_markdown_walk() -> Result<(), Box<dyn Error>> {
        let nested = format!(
            "{}x{}",
            "<div>".repeat(MAX_DOM_DEPTH),
            "</div>".repeat(MAX_DOM_DEPTH)
        );
        if !matches!(
            convert_html(nested.as_str(), u64::MAX),
            Err(SearchProviderError::TooLarge)
        ) {
            return Err("deep DOM passed the recursive Markdown conversion gate".into());
        }
        let many_nodes = "<span>x</span>".repeat(MAX_DOM_NODES);
        if !matches!(
            convert_html(many_nodes.as_str(), u64::MAX),
            Err(SearchProviderError::TooLarge)
        ) {
            return Err("oversized DOM passed the Markdown node-count gate".into());
        }
        Ok(())
    }
}
