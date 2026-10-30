//! Protocol version and the handshake rule shared by the kernel, worker and guest services.
//!
//! The version has a `major` and a `minor` part. Peers with different majors cannot talk; peers
//! with the same major speak the lower minor (minor bumps only add optional fields). Major `0` is
//! never valid: it is what an unset protobuf field decodes to.
//!
//! Compatibility with spike S0-12: the spike guest compared one integer (`protocol_version`, host
//! `1` accepted, host `2` refused with exit code 3). That integer is the major version here;
//! [`PROTOCOL_VERSION`] keeps major `1`.

use core::fmt;

use crate::error::ProtoError;
use crate::pb;

/// The wire-protocol version this build speaks.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::new(1, 3);

/// A wire-protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProtocolVersion {
    /// Incompatible changes bump the major version.
    pub major: u16,
    /// Backward-compatible additions bump the minor version.
    pub minor: u16,
}

impl ProtocolVersion {
    /// A version from its two parts.
    #[must_use]
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl From<ProtocolVersion> for pb::ProtocolVersion {
    fn from(version: ProtocolVersion) -> Self {
        Self {
            major: u32::from(version.major),
            minor: u32::from(version.minor),
        }
    }
}

impl TryFrom<pb::ProtocolVersion> for ProtocolVersion {
    type Error = ProtoError;

    fn try_from(wire: pb::ProtocolVersion) -> Result<Self, Self::Error> {
        let major = u16::try_from(wire.major).map_err(|_| ProtoError::OutOfRange {
            field: "protocol_version.major",
        })?;
        let minor = u16::try_from(wire.minor).map_err(|_| ProtoError::OutOfRange {
            field: "protocol_version.minor",
        })?;
        Ok(Self { major, minor })
    }
}

/// Agrees on the version two peers will speak.
///
/// # Errors
/// [`ProtoError::VersionMismatch`] when the major versions differ or either major is `0`.
pub const fn negotiate(
    local: ProtocolVersion,
    remote: ProtocolVersion,
) -> Result<ProtocolVersion, ProtoError> {
    if local.major == remote.major && local.major != 0 {
        let minor = if local.minor < remote.minor {
            local.minor
        } else {
            remote.minor
        };
        Ok(ProtocolVersion::new(local.major, minor))
    } else {
        Err(ProtoError::VersionMismatch { local, remote })
    }
}
