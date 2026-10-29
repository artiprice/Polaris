//! Real credential resolution seam; fixture sources perform no host credential access.
use polaris_nebius_cloud_token_factory::{ApiKey, CredentialError};
use polaris_sdk::secrets::{EnvName, SecretLocation, SecretSource, SecretSourceError};
use secrecy::SecretString;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Source {
    value: Result<Option<&'static str>, SecretSourceError>,
    reads: AtomicUsize,
}
impl SecretSource for Source {
    fn load(&self, _location: &SecretLocation) -> Result<Option<SecretString>, SecretSourceError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.value.map(|value| value.map(SecretString::from))
    }
    fn store(
        &self,
        _location: &SecretLocation,
        _value: &SecretString,
    ) -> Result<(), SecretSourceError> {
        Err(SecretSourceError::Unsupported)
    }
    fn delete(&self, _location: &SecretLocation) -> Result<bool, SecretSourceError> {
        Err(SecretSourceError::Unsupported)
    }
}
fn location() -> Result<SecretLocation, Box<dyn std::error::Error>> {
    Ok(SecretLocation::Environment(EnvName::new(
        "SYNTHETIC_API_KEY",
    )?))
}

#[test]
fn provider_value_is_resolved_once_and_debug_is_redacted() -> Result<(), Box<dyn std::error::Error>>
{
    let source = Source {
        value: Ok(Some("synthetic-provider-secret")),
        reads: AtomicUsize::new(0),
    };
    let key = ApiKey::from_source(&source, &location()?)?;
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    assert!(!format!("{key:?}").contains("synthetic-provider-secret"));
    Ok(())
}

#[test]
fn missing_and_failed_credentials_remain_distinct() -> Result<(), Box<dyn std::error::Error>> {
    let missing = Source {
        value: Ok(None),
        reads: AtomicUsize::new(0),
    };
    assert!(matches!(
        ApiKey::from_source(&missing, &location()?),
        Err(CredentialError::Missing)
    ));
    let failed = Source {
        value: Err(SecretSourceError::PermissionDenied),
        reads: AtomicUsize::new(0),
    };
    assert!(matches!(
        ApiKey::from_source(&failed, &location()?),
        Err(CredentialError::Source(SecretSourceError::PermissionDenied))
    ));
    Ok(())
}

#[test]
fn malformed_provider_credentials_fail_without_leaking_values()
-> Result<(), Box<dyn std::error::Error>> {
    for value in ["", "synthetic\nheader-injection", "synthetic\0credential"] {
        let source = Source {
            value: Ok(Some(value)),
            reads: AtomicUsize::new(0),
        };
        let result = ApiKey::from_source(&source, &location()?);
        assert!(matches!(result, Err(CredentialError::Invalid)));
        if let Err(error) = result {
            assert!(!error.to_string().contains("synthetic"));
        }
    }
    Ok(())
}
