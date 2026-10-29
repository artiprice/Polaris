// Unit tests of crates/polaris-search/crates/providers/src/tavily.rs (`tests`); compiled into that module via #[path].
use super::*;
use polaris_ratelimit::{Quota, ShedPolicy};
use polaris_sdk::registry::RedactStats;
use std::{error::Error, num::NonZeroU32};

#[derive(Debug)]
struct IdentityRedactor;

impl Redactor for IdentityRedactor {
    fn redact_into(&self, input: &[u8], output: &mut Vec<u8>) -> RedactStats {
        output.extend_from_slice(input);
        RedactStats {
            bytes_in: input.len(),
            replacements: 0,
        }
    }
}

#[tokio::test]
async fn invalid_site_filter_is_rejected_before_dispatch() -> Result<(), Box<dyn Error>> {
    let quota = Quota {
        rpm: NonZeroU32::new(60).ok_or("invalid test quota")?,
        tpm: NonZeroU32::new(60).ok_or("invalid test quota")?,
        burst_requests: 4,
        burst_tokens: 4,
    };
    let runtime = ProviderRuntime::new(quota, 1, ShedPolicy::default())?;
    let provider = Tavily::new(
        TavilyConfig {
            base_url: Url::parse("http://127.0.0.1:30303/")?,
            api_key: SecretString::from("synthetic-canary-key"),
            timeout: Duration::from_secs(1),
            max_response_bytes: 1_024,
            allow_http_loopback: true,
        },
        runtime.clone(),
        Arc::new(IdentityRedactor),
    )?;
    let query = SearchQuery {
        text: "valid query".to_owned(),
        max_results: 1,
        site: Some("invalid domain".to_owned()),
        recency_days: None,
    };
    if !matches!(provider.search(&query).await, Err(SearchError::BadQuery(_))) {
        return Err("Tavily did not reject the malformed site filter".into());
    }
    if runtime.window_snapshot().requests != 0 {
        return Err("invalid Tavily query was recorded as a dispatched request".into());
    }
    Ok(())
}
