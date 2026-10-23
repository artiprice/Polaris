//! Descriptor: specs/descriptors/secret_source_registry.json.
use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::{
    backends::{OneOf2, OneOf3},
    secrets::{EnvName, KeyringLocation, SecretLocation, SecretSource, SecretSourceError},
};
use secrecy::{ExposeSecret, SecretString};

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

#[derive(Debug)]
struct FixtureSecretSource(&'static str);

impl SecretSource for FixtureSecretSource {
    fn load(&self, _location: &SecretLocation) -> Result<Option<SecretString>, SecretSourceError> {
        Ok(Some(SecretString::from(self.0)))
    }

    fn store(
        &self,
        _location: &SecretLocation,
        value: &SecretString,
    ) -> Result<(), SecretSourceError> {
        if value.expose_secret() == self.0 {
            Ok(())
        } else {
            Err(SecretSourceError::Failed)
        }
    }

    fn delete(&self, _location: &SecretLocation) -> Result<bool, SecretSourceError> {
        Ok(true)
    }
}

fn fixture_location() -> Result<SecretLocation, polaris_sdk::error::ParseError> {
    Ok(SecretLocation::Environment(EnvName::new("FIXTURE_SECRET")?))
}

#[test]
fn one_of_two_forwards_all_secret_source_operations() -> Result<(), Box<dyn std::error::Error>> {
    let source: OneOf2<FixtureSecretSource, FixtureSecretSource> =
        OneOf2::B(FixtureSecretSource("variant-b"));
    let location = fixture_location()?;
    let value = source
        .load(&location)?
        .ok_or("selected source returned no value")?;
    assert_eq!(value.expose_secret(), "variant-b");
    source.store(&location, &value)?;
    assert!(source.delete(&location)?);
    Ok(())
}

#[test]
fn one_of_three_forwards_all_secret_source_operations() -> Result<(), Box<dyn std::error::Error>> {
    let source: OneOf3<FixtureSecretSource, FixtureSecretSource, FixtureSecretSource> =
        OneOf3::C(FixtureSecretSource("variant-c"));
    let location = fixture_location()?;
    let value = source
        .load(&location)?
        .ok_or("selected source returned no value")?;
    assert_eq!(value.expose_secret(), "variant-c");
    source.store(&location, &value)?;
    assert!(source.delete(&location)?);
    Ok(())
}
