//! Validated local policy for VM disk, TTL, and network ingress.

use std::net::IpAddr;
use std::time::Duration;

use thiserror::Error;

const MAX_OPERATOR_IPS: usize = 256;

/// Errors returned while validating VM or ingress policy.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SpecError {
    /// Disk size must be positive.
    #[error("disk size must be positive")]
    ZeroDisk,
    /// Hard lifetime must be positive.
    #[error("hard TTL must be positive")]
    ZeroTtl,
    /// Idle timeout must be positive.
    #[error("idle timeout must be positive")]
    ZeroIdleTimeout,
    /// Idle timeout cannot exceed hard lifetime.
    #[error("idle timeout exceeds hard TTL")]
    IdleExceedsTtl,
    /// At least one exact operator IP address is required.
    #[error("ingress allowlist cannot be empty")]
    EmptyIngressAllowlist,
    /// mTLS must be enabled for every remote gateway.
    #[error("mTLS is required")]
    MtlsRequired,
    /// An operator address is unspecified, multicast, or IPv4 limited broadcast.
    #[error("operator IP address is not a valid unicast address")]
    InvalidIngressAddress,
    /// The operator allowlist exceeds the fixed maximum of 256 addresses.
    #[error("operator IP allowlist exceeds 256 addresses")]
    TooManyIngressAddresses,
}

/// Exact IP address allowlist with mandatory mutual TLS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngressPolicy {
    operator_ips: Vec<IpAddr>,
}

/// VM policy values that passed local validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedVmSpec {
    disk_gib: u32,
    hard_ttl: Duration,
    idle_timeout: Duration,
    ingress: IngressPolicy,
}

impl IngressPolicy {
    /// Create a policy for at most 256 exact unicast IP addresses and require mTLS.
    ///
    /// IPv4-mapped IPv6 addresses are normalized to IPv4, and canonical duplicates
    /// are removed while preserving the first occurrence.
    /// IPv4 limited broadcast is rejected. The caller must also reject any
    /// subnet-directed broadcast using its network prefix, which a host IP alone
    /// cannot identify.
    pub fn new(operator_ips: Vec<IpAddr>, mtls_required: bool) -> Result<Self, SpecError> {
        if operator_ips.is_empty() {
            return Err(SpecError::EmptyIngressAllowlist);
        }
        if operator_ips.len() > MAX_OPERATOR_IPS {
            return Err(SpecError::TooManyIngressAddresses);
        }
        if !mtls_required {
            return Err(SpecError::MtlsRequired);
        }
        let mut canonical_ips = Vec::with_capacity(operator_ips.len());
        for address in operator_ips {
            let canonical = canonicalize_ip(address);
            if !is_valid_operator_ip(canonical) {
                return Err(SpecError::InvalidIngressAddress);
            }
            if !canonical_ips.contains(&canonical) {
                canonical_ips.push(canonical);
            }
        }
        Ok(Self {
            operator_ips: canonical_ips,
        })
    }

    /// Return whether an exact operator address is allowed.
    #[must_use]
    pub fn allows(&self, address: IpAddr) -> bool {
        self.operator_ips.contains(&canonicalize_ip(address))
    }

    /// Report that mutual TLS is a construction invariant of this policy.
    #[must_use]
    pub const fn mtls_required(&self) -> bool {
        true
    }

    /// Borrow the exact allowed operator addresses.
    #[must_use]
    pub fn operator_ips(&self) -> &[IpAddr] {
        &self.operator_ips
    }
}

fn canonicalize_ip(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V4(address) => IpAddr::V4(address),
        IpAddr::V6(address) => match address.to_ipv4_mapped() {
            Some(mapped) => IpAddr::V4(mapped),
            None => IpAddr::V6(address),
        },
    }
}

fn is_valid_operator_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            !address.is_unspecified() && !address.is_broadcast() && !address.is_multicast()
        }
        IpAddr::V6(address) => !address.is_unspecified() && !address.is_multicast(),
    }
}

impl ValidatedVmSpec {
    /// Validate the local cloud VM lifecycle and ingress policy.
    pub fn new(
        disk_gib: u32,
        hard_ttl: Duration,
        idle_timeout: Duration,
        ingress: IngressPolicy,
    ) -> Result<Self, SpecError> {
        if disk_gib == 0 {
            return Err(SpecError::ZeroDisk);
        }
        if hard_ttl.is_zero() {
            return Err(SpecError::ZeroTtl);
        }
        if idle_timeout.is_zero() {
            return Err(SpecError::ZeroIdleTimeout);
        }
        if idle_timeout > hard_ttl {
            return Err(SpecError::IdleExceedsTtl);
        }
        Ok(Self {
            disk_gib,
            hard_ttl,
            idle_timeout,
            ingress,
        })
    }

    /// Return the validated boot disk size.
    #[must_use]
    pub const fn disk_gib(&self) -> u32 {
        self.disk_gib
    }

    /// Return the validated hard lifetime.
    #[must_use]
    pub const fn hard_ttl(&self) -> Duration {
        self.hard_ttl
    }

    /// Return the validated idle timeout.
    #[must_use]
    pub const fn idle_timeout(&self) -> Duration {
        self.idle_timeout
    }

    /// Borrow the validated ingress policy.
    #[must_use]
    pub const fn ingress(&self) -> &IngressPolicy {
        &self.ingress
    }
}
