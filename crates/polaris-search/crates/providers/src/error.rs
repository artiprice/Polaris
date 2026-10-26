use std::time::Duration;

/// Reason a requested network destination was blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// Only HTTP and HTTPS destinations are supported.
    Scheme,
    /// The URL contains username or password components.
    UserInfo,
    /// DNS or an IP literal resolved to a non-public address.
    NonPublicAddress,
    /// A local exception did not match the configured service origin.
    LocalOriginMismatch,
    /// The request URL exceeded the local length bound.
    UrlTooLong,
    /// The hostname did not resolve to any addresses.
    UnresolvedHost,
    /// The server exceeded the configured redirect count.
    RedirectLimit,
}

/// Sanitized errors from provider requests and page fetching.
#[derive(Debug, thiserror::Error)]
pub enum SearchProviderError {
    /// A transport or timeout failure. Details never include a URL, query, or credential.
    #[error("remote service unavailable")]
    Unavailable,
    /// A remote service returned an unsuccessful status.
    #[error("remote service returned HTTP status {0}")]
    Status(u16),
    /// A provider rejected the request due to rate limiting.
    #[error("remote service rate limited the request")]
    RateLimited {
        /// Parsed Retry-After delay from delta-seconds or HTTP-date syntax.
        retry_after: Option<Duration>,
    },
    /// A URL was rejected by the destination policy.
    #[error("destination blocked by network policy: {0:?}")]
    Blocked(BlockReason),
    /// The response exceeded a configured decoded byte limit.
    #[error("response exceeded the configured byte limit")]
    TooLarge,
    /// The response media type was not allowed.
    #[error("unsupported response content type")]
    UnsupportedContent,
    /// A response did not match the provider's documented JSON contract.
    #[error("remote service returned an invalid response")]
    Decode,
    /// Local configuration is incomplete or invalid.
    #[error("invalid search provider configuration")]
    ProviderConfig,
    /// Redaction did not satisfy the SDK input accounting contract.
    #[error("query redaction failed closed")]
    Redaction,
    /// Rate limit admission or accounting failed.
    #[error("request admission failed")]
    RateLimit,
    /// HTML conversion failed.
    #[error("HTML extraction failed")]
    Extraction,
}
