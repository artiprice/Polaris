use polaris_ratelimit::{Quota, ShedPolicy};
use polaris_sdk::registry::{RedactStats, SearchProvider, SearchQuery, WebSearch};
use polaris_search_providers::{
    FetchPolicy, FetchedTrust, Fetcher, ProviderRuntime, Searxng, SearxngConfig,
};
use secrecy::SecretString;
use std::{error::Error, net::SocketAddr, num::NonZeroU32, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use url::Url;

#[derive(Debug)]
struct IdentityRedactor;

impl polaris_sdk::registry::Redactor for IdentityRedactor {
    fn redact_into(&self, input: &[u8], output: &mut Vec<u8>) -> RedactStats {
        let text = String::from_utf8_lossy(input);
        let redacted = text.replace("private-query", "masked-query");
        output.extend_from_slice(redacted.as_bytes());
        RedactStats {
            bytes_in: input.len(),
            replacements: u32::from(redacted != text),
        }
    }
}

#[derive(Debug)]
struct BrokenRedactor;

impl polaris_sdk::registry::Redactor for BrokenRedactor {
    fn redact_into(&self, input: &[u8], _output: &mut Vec<u8>) -> RedactStats {
        RedactStats {
            bytes_in: input.len().saturating_sub(1),
            replacements: 0,
        }
    }
}

fn runtime() -> Result<ProviderRuntime, Box<dyn Error>> {
    let quota = Quota {
        rpm: NonZeroU32::new(60).ok_or("invalid test quota")?,
        tpm: NonZeroU32::new(60).ok_or("invalid test quota")?,
        burst_requests: 4,
        burst_tokens: 4,
    };
    ProviderRuntime::new(quota, 4, ShedPolicy::default()).map_err(Into::into)
}

async fn one_response(
    body: &'static str,
    content_type: &'static str,
) -> Result<(Url, tokio::task::JoinHandle<Result<String, std::io::Error>>), Box<dyn Error>> {
    fixture_response("200 OK", body, content_type).await
}

async fn fixture_response(
    status: &'static str,
    body: &'static str,
    content_type: &'static str,
) -> Result<(Url, tokio::task::JoinHandle<Result<String, std::io::Error>>), Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address: SocketAddr = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let serve = async {
            let (mut stream, _) = listener.accept().await?;
            let mut request_bytes = Vec::new();
            while !request_bytes.ends_with(b"\r\n\r\n") {
                request_bytes.push(stream.read_u8().await?);
            }
            let headers = String::from_utf8_lossy(request_bytes.as_slice());
            let parsed_content_length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            });
            let mut content_length = 0;
            if let Some(length) = parsed_content_length {
                content_length = length;
            }
            let mut request_body = vec![0_u8; content_length];
            stream.read_exact(request_body.as_mut_slice()).await?;
            request_bytes.extend_from_slice(request_body.as_slice());
            let request = String::from_utf8_lossy(request_bytes.as_slice()).into_owned();
            let retry_header = if status.starts_with("429") {
                "Retry-After: 3\r\n"
            } else {
                ""
            };
            let redirect_header = if status.starts_with("302") {
                "Location: http://127.0.0.1:30303/private\r\n"
            } else {
                ""
            };
            let response = format!(
                "HTTP/1.1 {status}\r\n{retry_header}{redirect_header}Content-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await?;
            Ok::<String, std::io::Error>(request)
        };
        match tokio::time::timeout(Duration::from_secs(3), serve).await {
            Ok(result) => result,
            Err(_) => Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "fixture server timed out",
            )),
        }
    });
    let url = Url::parse(&format!("http://{address}/"))?;
    Ok((url, server))
}

#[tokio::test]
async fn searxng_maps_results_and_redacts_query() -> Result<(), Box<dyn Error>> {
    let body = r#"{"results":[{"url":"https://example.org/page","title":"Example","content":"A snippet"}]}"#;
    let (base_url, server) = one_response(body, "application/json").await?;
    let provider = Searxng::new(
        SearxngConfig {
            base_url,
            engines: vec!["general".to_owned()],
            timeout: Duration::from_secs(2),
            max_response_bytes: 16_384,
            allow_http_loopback: true,
        },
        runtime()?,
        Arc::new(IdentityRedactor),
    )?;
    let query = SearchQuery {
        text: "private-query".to_owned(),
        max_results: 3,
        site: None,
        recency_days: None,
    };
    let hits = provider.search(&query).await?;
    let request = server.await??;
    if hits.len() != 1 || hits.first().map(|hit| hit.provider) != Some(SearchProvider::Searxng) {
        return Err("SearXNG hit mapping did not preserve the single fixture result".into());
    }
    if !request.contains("q=masked-query")
        || request.contains("private-query")
        || !request.contains("format=json")
    {
        return Err("SearXNG query was not redacted before the JSON API request".into());
    }
    Ok(())
}

#[tokio::test]
async fn tavily_maps_results_and_keeps_key_out_of_debug_output() -> Result<(), Box<dyn Error>> {
    let body = r#"{"results":[{"url":"https://example.org/page","title":"Example","content":"A snippet"}]}"#;
    let (base_url, server) = one_response(body, "application/json").await?;
    let config = polaris_search_providers::TavilyConfig {
        base_url,
        api_key: SecretString::from("synthetic-canary-key"),
        timeout: Duration::from_secs(2),
        max_response_bytes: 16_384,
        allow_http_loopback: true,
    };
    if format!("{config:?}").contains("synthetic-canary-key") {
        return Err("Tavily configuration debug output exposed its credential".into());
    }
    let provider =
        polaris_search_providers::Tavily::new(config, runtime()?, Arc::new(IdentityRedactor))?;
    let query = SearchQuery {
        text: "private-query".to_owned(),
        max_results: 3,
        site: None,
        recency_days: None,
    };
    let hits = provider.search(&query).await?;
    let request = server.await??;
    if hits.len() != 1 || hits.first().map(|hit| hit.provider) != Some(SearchProvider::Tavily) {
        return Err("Tavily hit mapping did not preserve the single fixture result".into());
    }
    if !request
        .to_ascii_lowercase()
        .contains("authorization: bearer synthetic-canary-key")
        || !request.contains("masked-query")
        || request.contains("private-query")
        || format!("{provider:?}").contains("synthetic-canary-key")
    {
        return Err("Tavily credential or unredacted query handling was incorrect".into());
    }
    Ok(())
}

#[tokio::test]
async fn tavily_maps_rate_limit_without_exposing_response_body() -> Result<(), Box<dyn Error>> {
    let (base_url, server) = fixture_response(
        "429 Too Many Requests",
        "private provider error body",
        "application/json",
    )
    .await?;
    let provider = polaris_search_providers::Tavily::new(
        polaris_search_providers::TavilyConfig {
            base_url,
            api_key: SecretString::from("synthetic-canary-key"),
            timeout: Duration::from_secs(2),
            max_response_bytes: 16_384,
            allow_http_loopback: true,
        },
        runtime()?,
        Arc::new(IdentityRedactor),
    )?;
    let query = SearchQuery {
        text: "fixture query".to_owned(),
        max_results: 1,
        site: None,
        recency_days: None,
    };
    let result = provider.search(&query).await;
    server.await??;
    if !matches!(
        result,
        Err(polaris_sdk::registry::SearchError::RateLimited {
            retry_after_ms: Some(3_000)
        })
    ) {
        return Err("Tavily 429 response was not mapped to a rate-limit error".into());
    }
    if format!("{result:?}").contains("private provider error body") {
        return Err("Tavily diagnostics exposed an upstream response body".into());
    }
    Ok(())
}

#[tokio::test]
async fn fetch_rejects_private_destination_without_explicit_origin() -> Result<(), Box<dyn Error>> {
    let page_url = Url::parse("http://127.0.0.1:30303/private")?;
    let fetcher = Fetcher::new(
        FetchPolicy {
            max_bytes: 16_384,
            max_redirects: 2,
            timeout: Duration::from_secs(2),
            allowed_content_types: vec!["text/html".to_owned()],
            user_agent: "PolarisSearchTest/1".to_owned(),
            local_origin: None,
        },
        runtime()?,
    )?;
    if fetcher.fetch(&page_url).await.is_ok() {
        return Err("fetcher unexpectedly allowed a private destination".into());
    }
    Ok(())
}

#[tokio::test]
async fn fetch_converts_html_strips_hidden_text_and_marks_untrusted() -> Result<(), Box<dyn Error>>
{
    let html = "<html><head><title>hidden head</title></head><body><script>hidden script</script><div hidden>hidden attribute</div><span style=\"display: none\">hidden style</span><p>Visible text</p></body></html>";
    let (page_url, server) = one_response(html, "text/html; charset=utf-8").await?;
    let fetcher = Fetcher::new(
        FetchPolicy {
            max_bytes: 16_384,
            max_redirects: 2,
            timeout: Duration::from_secs(2),
            allowed_content_types: vec!["text/html".to_owned()],
            user_agent: "PolarisSearchTest/1".to_owned(),
            local_origin: Some(page_url.clone()),
        },
        runtime()?,
    )?;
    let page = fetcher.fetch(&page_url).await?;
    server.await??;
    if page.trust != FetchedTrust::Untrusted || !page.markdown.contains("Visible text") {
        return Err("fetch result did not retain visible untrusted page content".into());
    }
    if page.markdown.contains("hidden") {
        return Err("fetch result retained hidden or active HTML content".into());
    }
    Ok(())
}

#[tokio::test]
async fn fetch_enforces_body_byte_limit() -> Result<(), Box<dyn Error>> {
    let body = "<html><body>content longer than configured limit</body></html>";
    let (page_url, server) = one_response(body, "text/html").await?;
    let fetcher = Fetcher::new(
        FetchPolicy {
            max_bytes: 10,
            max_redirects: 1,
            timeout: Duration::from_secs(2),
            allowed_content_types: vec!["text/html".to_owned()],
            user_agent: "PolarisSearchTest/1".to_owned(),
            local_origin: Some(page_url.clone()),
        },
        runtime()?,
    )?;
    let result = fetcher.fetch(&page_url).await;
    server.await??;
    if !matches!(
        result,
        Err(polaris_search_providers::SearchProviderError::TooLarge)
    ) {
        return Err("fetcher did not reject a response above its byte cap".into());
    }
    Ok(())
}

#[tokio::test]
async fn fetch_rejects_control_characters_in_user_agent() -> Result<(), Box<dyn Error>> {
    let page_url = Url::parse("http://127.0.0.1:30303/")?;
    let result = Fetcher::new(
        FetchPolicy {
            max_bytes: 16_384,
            max_redirects: 1,
            timeout: Duration::from_secs(2),
            allowed_content_types: vec!["text/html".to_owned()],
            user_agent: "PolarisSearchTest/1\r\nX-Injected: yes".to_owned(),
            local_origin: Some(page_url),
        },
        runtime()?,
    );
    if result.is_ok() {
        return Err("fetcher accepted control characters in its user-agent".into());
    }
    Ok(())
}

#[tokio::test]
async fn fetch_revalidates_redirect_target_before_request() -> Result<(), Box<dyn Error>> {
    let (page_url, server) = fixture_response("302 Found", "", "text/html").await?;
    let fetcher = Fetcher::new(
        FetchPolicy {
            max_bytes: 16_384,
            max_redirects: 2,
            timeout: Duration::from_secs(2),
            allowed_content_types: vec!["text/html".to_owned()],
            user_agent: "PolarisSearchTest/1".to_owned(),
            local_origin: Some(page_url.clone()),
        },
        runtime()?,
    )?;
    let result = fetcher.fetch(&page_url).await;
    server.await??;
    if !matches!(
        result,
        Err(polaris_search_providers::SearchProviderError::Blocked(_))
    ) {
        return Err("fetcher followed a redirect to a private destination".into());
    }
    Ok(())
}

#[tokio::test]
async fn search_fails_closed_when_redactor_accounting_is_invalid() -> Result<(), Box<dyn Error>> {
    let base_url = Url::parse("http://127.0.0.1:30303/")?;
    let provider = Searxng::new(
        SearxngConfig {
            base_url,
            engines: vec!["general".to_owned()],
            timeout: Duration::from_secs(2),
            max_response_bytes: 16_384,
            allow_http_loopback: true,
        },
        runtime()?,
        Arc::new(BrokenRedactor),
    )?;
    let query = SearchQuery {
        text: "private-query".to_owned(),
        max_results: 1,
        site: None,
        recency_days: None,
    };
    if provider.search(&query).await.is_ok() {
        return Err("search provider egressed after redactor accounting failed".into());
    }
    Ok(())
}
