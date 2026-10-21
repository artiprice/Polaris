//! Protocol Version Negotiation (descriptor: specs/descriptors/proto_protocol_version_negotiation.json).

use std::error::Error;

use polaris_proto::{PROTOCOL_VERSION, ProtoError, ProtocolVersion, negotiate, pb};
use polaris_sdk::error::{ErrorKind, HasKind};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

const fn v(major: u16, minor: u16) -> ProtocolVersion {
    ProtocolVersion::new(major, minor)
}

#[test]
fn same_major_negotiates_to_min_minor() -> TestResult {
    assert_eq!(negotiate(v(1, 3), v(1, 5))?, v(1, 3));
    assert_eq!(negotiate(v(1, 5), v(1, 3))?, v(1, 3));
    assert_eq!(negotiate(v(1, 0), v(1, 0))?, v(1, 0));
    assert_eq!(negotiate(v(7, 2), v(7, 2))?, v(7, 2));
    Ok(())
}

#[test]
fn different_major_is_rejected() {
    let local = v(1, 0);
    let remote = v(2, 0);
    assert_eq!(
        negotiate(local, remote),
        Err(ProtoError::VersionMismatch { local, remote })
    );
    assert_eq!(
        negotiate(remote, local),
        Err(ProtoError::VersionMismatch {
            local: remote,
            remote: local
        })
    );
}

#[test]
fn zero_major_is_rejected() {
    assert!(matches!(
        negotiate(v(0, 1), v(0, 1)),
        Err(ProtoError::VersionMismatch { .. })
    ));
    assert!(matches!(
        negotiate(v(1, 0), v(0, 0)),
        Err(ProtoError::VersionMismatch { .. })
    ));
    assert!(matches!(
        negotiate(v(0, 0), v(1, 0)),
        Err(ProtoError::VersionMismatch { .. })
    ));
}

#[test]
fn negotiation_is_symmetric_on_success() -> TestResult {
    let a = v(3, 9);
    let b = v(3, 4);
    assert_eq!(negotiate(a, b)?, negotiate(b, a)?);
    Ok(())
}

#[test]
fn guest_spike_handshake_still_holds() -> TestResult {
    // Spike S0-12: `polaris-guest handshake <n>` accepted n == 1 (exit 0) and refused n == 2 (exit 3).
    // The single integer of the spike is the major version of this protocol.
    assert_eq!(PROTOCOL_VERSION.major, 1);
    let accepted = negotiate(PROTOCOL_VERSION, v(1, 0))?;
    assert_eq!(accepted.major, 1);
    let refused = negotiate(PROTOCOL_VERSION, v(2, 0));
    assert_eq!(
        refused,
        Err(ProtoError::VersionMismatch {
            local: PROTOCOL_VERSION,
            remote: v(2, 0)
        })
    );
    assert_eq!(
        refused.map_err(|e| e.kind()),
        Err(ErrorKind::Permanent),
        "a version mismatch is not retryable"
    );
    Ok(())
}

#[test]
fn wire_version_out_of_range_is_error() -> TestResult {
    let ok = ProtocolVersion::try_from(pb::ProtocolVersion { major: 1, minor: 2 })?;
    assert_eq!(ok, v(1, 2));
    let big_major = ProtocolVersion::try_from(pb::ProtocolVersion {
        major: u32::from(u16::MAX) + 1,
        minor: 0,
    });
    assert!(matches!(big_major, Err(ProtoError::OutOfRange { .. })));
    let big_minor = ProtocolVersion::try_from(pb::ProtocolVersion {
        major: 1,
        minor: u32::MAX,
    });
    assert!(matches!(big_minor, Err(ProtoError::OutOfRange { .. })));
    let back = pb::ProtocolVersion::from(v(u16::MAX, u16::MAX));
    assert_eq!(back.major, u32::from(u16::MAX));
    assert_eq!(back.minor, u32::from(u16::MAX));
    Ok(())
}

#[test]
fn display_is_major_dot_minor() {
    assert_eq!(v(1, 0).to_string(), "1.0");
    assert_eq!(v(12, 34).to_string(), "12.34");
}

proptest! {
    #[test]
    fn prop_negotiation_matches_the_rule(
        lmaj in 0u16..4, lmin in any::<u16>(), rmaj in 0u16..4, rmin in any::<u16>()
    ) {
        let local = v(lmaj, lmin);
        let remote = v(rmaj, rmin);
        let result = negotiate(local, remote);
        if lmaj == rmaj && lmaj != 0 {
            prop_assert_eq!(result, Ok(v(lmaj, lmin.min(rmin))));
        } else {
            prop_assert_eq!(result, Err(ProtoError::VersionMismatch { local, remote }));
        }
    }
}
