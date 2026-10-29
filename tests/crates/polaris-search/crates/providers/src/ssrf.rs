// Unit tests of crates/polaris-search/crates/providers/src/ssrf.rs (`tests`); compiled into that module via #[path].
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
