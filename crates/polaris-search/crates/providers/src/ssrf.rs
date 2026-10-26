use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use tokio::net::lookup_host;
use url::Url;

use crate::{BlockReason, SearchProviderError};

const MAX_TARGET_URL_BYTES: usize = 8_192;

/// URL with DNS answers validated and ready to be pinned into reqwest.
#[derive(Debug, Clone)]
pub(crate) struct ValidatedTarget {
    pub(crate) url: Url,
    pub(crate) addresses: Vec<SocketAddr>,
    pub(crate) host: String,
}

/// Resolves a destination, rejects unsafe address classes, and returns the checked addresses.
pub(crate) async fn validate_target(
    url: &Url,
    local_origin: Option<&Url>,
) -> Result<ValidatedTarget, SearchProviderError> {
    if url.as_str().len() > MAX_TARGET_URL_BYTES {
        return Err(SearchProviderError::Blocked(BlockReason::UrlTooLong));
    }
    if !matches!(url.scheme(), "http" | "https") {
        return Err(SearchProviderError::Blocked(BlockReason::Scheme));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SearchProviderError::Blocked(BlockReason::UserInfo));
    }
    let host = url
        .host_str()
        .ok_or(SearchProviderError::ProviderConfig)?
        .to_owned();
    let port = url
        .port_or_known_default()
        .ok_or(SearchProviderError::ProviderConfig)?;
    let is_local_exception = local_origin.is_some_and(|origin| same_origin(url, origin));
    if local_origin.is_some_and(|origin| !same_origin(url, origin)) && is_local_address_literal(url)
    {
        return Err(SearchProviderError::Blocked(
            BlockReason::LocalOriginMismatch,
        ));
    }
    let addresses = match url.host() {
        Some(url::Host::Ipv4(address)) => vec![SocketAddr::new(IpAddr::V4(address), port)],
        Some(url::Host::Ipv6(address)) => vec![SocketAddr::new(IpAddr::V6(address), port)],
        Some(url::Host::Domain(domain)) => lookup_host((domain, port))
            .await
            .map_err(|_| SearchProviderError::Blocked(BlockReason::UnresolvedHost))?
            .collect::<Vec<_>>(),
        None => return Err(SearchProviderError::Blocked(BlockReason::UnresolvedHost)),
    };
    if addresses.is_empty() {
        return Err(SearchProviderError::Blocked(BlockReason::UnresolvedHost));
    }
    if addresses.iter().any(|address| {
        if is_local_exception {
            !is_allowed_local(address.ip())
        } else {
            !is_public_address(address.ip())
        }
    }) {
        return Err(SearchProviderError::Blocked(BlockReason::NonPublicAddress));
    }
    Ok(ValidatedTarget {
        url: url.clone(),
        addresses,
        host,
    })
}

pub(crate) fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

pub(crate) fn validate_base_url(
    url: &Url,
    allow_http_loopback: bool,
) -> Result<(), SearchProviderError> {
    if url.as_str().len() > 2_048 {
        return Err(SearchProviderError::ProviderConfig);
    }
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(SearchProviderError::ProviderConfig);
    }
    if url.scheme() == "http" && !(allow_http_loopback && is_loopback_host(url)) {
        return Err(SearchProviderError::ProviderConfig);
    }
    Ok(())
}

fn is_local_address_literal(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(address)) => !is_public_address(IpAddr::V4(address)),
        Some(url::Host::Ipv6(address)) => !is_public_address(IpAddr::V6(address)),
        Some(url::Host::Domain(domain)) => {
            domain.eq_ignore_ascii_case("localhost")
                || domain.to_ascii_lowercase().ends_with(".localhost")
        }
        None => false,
    }
}

fn is_loopback_host(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(domain)) => {
            domain.eq_ignore_ascii_case("localhost")
                || domain.to_ascii_lowercase().ends_with(".localhost")
        }
        None => false,
    }
}

fn is_allowed_local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ipv4) => {
            ipv4.is_loopback()
                || (ipv4.is_private()
                    && !(ipv4.octets().first() == Some(&169) && ipv4.octets().get(1) == Some(&254)))
        }
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                || ipv6.is_unique_local()
                || ipv6.to_ipv4_mapped().is_some_and(|mapped| {
                    mapped.is_loopback()
                        || (mapped.is_private()
                            && !(mapped.octets().first() == Some(&169)
                                && mapped.octets().get(1) == Some(&254)))
                })
        }
    }
}

fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ipv4) => is_public_ipv4(ipv4),
        IpAddr::V6(ipv6) => is_public_ipv6(ipv6),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let octets = address.octets();
    !(address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_unspecified()
        || address.is_multicast()
        || octets.first() == Some(&0)
        || (octets.first() == Some(&100)
            && octets
                .get(1)
                .is_some_and(|second| (64..=127).contains(second)))
        || (octets.first() == Some(&192) && octets.get(1) == Some(&0) && octets.get(2) == Some(&0))
        || (octets.first() == Some(&192) && octets.get(1) == Some(&0) && octets.get(2) == Some(&2))
        || (octets.first() == Some(&198)
            && octets
                .get(1)
                .is_some_and(|second| (18..=19).contains(second)))
        || (octets.first() == Some(&198)
            && octets.get(1) == Some(&51)
            && octets.get(2) == Some(&100))
        || (octets.first() == Some(&203)
            && octets.get(1) == Some(&0)
            && octets.get(2) == Some(&113))
        || (octets.first() == Some(&192)
            && octets.get(1) == Some(&88)
            && octets.get(2) == Some(&99))
        || octets.first().is_some_and(|first| *first >= 240))
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let segments = address.segments();
    let Some(first) = segments.first().copied() else {
        return false;
    };
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || address.is_unique_local()
        || address.is_unicast_link_local()
        || first & 0xe000 != 0x2000
        || (first == 0x2001
            && (segments.get(1) == Some(&0x0000)
                || segments.get(1) == Some(&0x0010)
                || segments.get(1) == Some(&0x0db8)))
        || first == 0x2002)
}

#[cfg(test)]
mod tests {
    use super::{MAX_TARGET_URL_BYTES, is_public_ipv4, is_public_ipv6, validate_target};
    use std::{
        error::Error,
        net::{Ipv4Addr, Ipv6Addr},
    };
    use url::Url;

    #[test]
    fn rejects_non_public_ipv4_ranges_and_allows_global_unicast() -> Result<(), Box<dyn Error>> {
        let blocked = [
            Ipv4Addr::new(127, 0, 0, 1),
            Ipv4Addr::new(10, 0, 0, 1),
            Ipv4Addr::new(169, 254, 169, 254),
            Ipv4Addr::new(100, 64, 0, 1),
            Ipv4Addr::new(192, 0, 2, 1),
            Ipv4Addr::new(224, 0, 0, 1),
        ];
        if blocked.iter().any(|address| is_public_ipv4(*address)) {
            return Err("SSRF guard accepted a non-public IPv4 range".into());
        }
        if !is_public_ipv4(Ipv4Addr::new(1, 1, 1, 1)) {
            return Err("SSRF guard rejected a global IPv4 address".into());
        }
        Ok(())
    }

    #[test]
    fn rejects_non_public_ipv6_and_ipv4_mapped_private() -> Result<(), Box<dyn Error>> {
        let blocked = [
            Ipv6Addr::LOCALHOST,
            Ipv6Addr::UNSPECIFIED,
            Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1),
            Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 1),
            Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x0a00, 0x0001),
            Ipv6Addr::new(0x2002, 0x0a00, 0, 0, 0, 0, 0, 1),
        ];
        if blocked.iter().any(|address| is_public_ipv6(*address)) {
            return Err("SSRF guard accepted a non-public IPv6 range".into());
        }
        if !is_public_ipv6(Ipv6Addr::new(0x2606, 0x4700, 0, 0, 0, 0, 0, 0x1111)) {
            return Err("SSRF guard rejected a global IPv6 address".into());
        }
        Ok(())
    }

    #[tokio::test]
    async fn rejects_overlong_target_before_dns_lookup() -> Result<(), Box<dyn Error>> {
        let path = "a".repeat(MAX_TARGET_URL_BYTES);
        let url = Url::parse(format!("https://example.org/{path}").as_str())?;
        if !matches!(
            validate_target(&url, None).await,
            Err(crate::SearchProviderError::Blocked(
                crate::BlockReason::UrlTooLong
            ))
        ) {
            return Err("SSRF guard accepted a URL above its local byte limit".into());
        }
        Ok(())
    }
}
