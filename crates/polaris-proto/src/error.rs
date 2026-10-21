//! The error type of every fallible operation in this crate.

use polaris_sdk::error::{ErrorKind, HasKind, ParseError};

use crate::version::ProtocolVersion;

/// Why a wire message was rejected or could not be converted.
///
/// Every variant is permanent: the same bytes fail the same way when retried.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtoError {
    /// A required field or `oneof` body is absent.
    #[error("missing required field `{0}`")]
    MissingField(&'static str),
    /// An enum number that is `*_UNSPECIFIED` or not part of the schema.
    #[error("unknown enum value {value} in `{field}`")]
    UnknownVariant {
        /// Dotted path of the field.
        field: &'static str,
        /// The number found on the wire.
        value: i32,
    },
    /// The peers do not speak a compatible protocol (different or zero major version).
    #[error("incompatible protocol versions: local {local}, remote {remote}")]
    VersionMismatch {
        /// The version of this process.
        local: ProtocolVersion,
        /// The version announced by the peer.
        remote: ProtocolVersion,
    },
    /// A message or chunk is longer than the configured limit.
    #[error("message of {len} bytes exceeds the limit of {limit} bytes")]
    Oversize {
        /// Length of the rejected message in bytes.
        len: usize,
        /// The limit that applied, in bytes.
        limit: usize,
    },
    /// A fixed-size field (a `Cid` digest) has the wrong length.
    #[error("`{field}` must be {expected} bytes, got {actual}")]
    InvalidLength {
        /// Dotted path of the field.
        field: &'static str,
        /// Required length in bytes.
        expected: usize,
        /// Length found on the wire in bytes.
        actual: usize,
    },
    /// A number that does not fit the sdk type, or a timestamp outside the supported range.
    #[error("`{field}` is out of range")]
    OutOfRange {
        /// Dotted path of the field.
        field: &'static str,
    },
    /// An sdk validator rejected a value (label, id, probability, schema version).
    #[error(transparent)]
    Invalid(#[from] ParseError),
    /// The bytes are not a valid protobuf encoding of the expected message.
    #[error("protobuf decode failed: {0}")]
    Decode(#[from] prost::DecodeError),
}

impl HasKind for ProtoError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::MissingField(_)
            | Self::UnknownVariant { .. }
            | Self::VersionMismatch { .. }
            | Self::Oversize { .. }
            | Self::InvalidLength { .. }
            | Self::OutOfRange { .. }
            | Self::Invalid(_)
            | Self::Decode(_) => ErrorKind::Permanent,
        }
    }
}
