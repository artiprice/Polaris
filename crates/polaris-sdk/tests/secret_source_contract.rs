//! Descriptor: specs/descriptors/secret_source_registry.json.
use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::secrets::{EnvName, KeyringLocation, SecretLocation, SecretSourceError};

#[test]
fn environment_name_requires_bounded_uppercase_identifier() {
    assert!(EnvName::new("NEBIUS_API_KEY").is_ok());
    assert!(EnvName::new("").is_err());
    assert!(EnvName::new("lowercase").is_err());
    assert!(EnvName::new(&"A".repeat(65)).is_err());
}

#[test]
fn keyring_location_rejects_empty_or_controlled_parts() {
    assert!(KeyringLocation::new("polaris", "NEBIUS_API_KEY").is_ok());
    assert!(KeyringLocation::new("", "account").is_err());
    assert!(KeyringLocation::new("polaris", "account\nvalue").is_err());
}

#[test]
fn validated_locations_contain_identifiers_only() -> Result<(), polaris_sdk::error::ParseError> {
    let location = SecretLocation::Environment(EnvName::new("TAVILY_API_KEY")?);
    let text = format!("{location:?}");
    assert!(text.contains("TAVILY_API_KEY"));
    assert!(!text.contains("synthetic-secret-canary"));
    Ok(())
}

#[test]
fn source_error_display_is_sanitized() {
    for error in [
        SecretSourceError::Unavailable,
        SecretSourceError::PermissionDenied,
        SecretSourceError::Unsupported,
        SecretSourceError::InvalidLocation,
        SecretSourceError::TooLarge,
        SecretSourceError::Failed,
    ] {
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains("credential secret"));
        assert!(!rendered.contains("provider supplied details"));
    }
}

#[test]
fn has_kind_maps_transient_and_permanent_failures() {
    assert_eq!(SecretSourceError::Unavailable.kind(), ErrorKind::Transient);
    assert_eq!(SecretSourceError::Unsupported.kind(), ErrorKind::Permanent);
    assert_eq!(
        SecretSourceError::InvalidLocation.kind(),
        ErrorKind::Permanent
    );
}
