use polaris_sdk::secrets::{KeyringLocation, SecretLocation, SecretSource, SecretSourceError};
use polaris_security_secrets::{SecretVault, VaultLimits};
use secrecy::SecretString;
use std::sync::atomic::{AtomicUsize, Ordering};
struct Source {
    key: Option<SecretString>,
    writes: AtomicUsize,
}
impl SecretSource for Source {
    fn load(&self, _: &SecretLocation) -> Result<Option<SecretString>, SecretSourceError> {
        Ok(self.key.clone())
    }
    fn store(&self, _: &SecretLocation, _: &SecretString) -> Result<(), SecretSourceError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Err(SecretSourceError::Unsupported)
    }
    fn delete(&self, _: &SecretLocation) -> Result<bool, SecretSourceError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Err(SecretSourceError::Unsupported)
    }
}
fn source(value: Option<&str>) -> Source {
    Source {
        key: value.map(SecretString::from),
        writes: AtomicUsize::new(0),
    }
}
type TestResult = Result<(), Box<dyn std::error::Error>>;
#[test]
fn explicit_source_key_recovers_ciphertext_in_a_new_vault() -> TestResult {
    let location =
        SecretLocation::Keyring(KeyringLocation::new("polaris.synthetic", "snapshot.key")?);
    let source = source(Some(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    ));
    let first = SecretVault::with_seal_key_from_source(&location, &source, VaultLimits::new(0, 0))?;
    let blob = first.seal_bytes(b"synthetic recoverable secret")?;
    drop(first);
    let second =
        SecretVault::with_seal_key_from_source(&location, &source, VaultLimits::new(0, 0))?;
    assert_eq!(
        second.open_bytes(&blob)?.as_slice(),
        b"synthetic recoverable secret"
    );
    assert_eq!(source.writes.load(Ordering::SeqCst), 0);
    Ok(())
}
#[test]
fn invalid_or_missing_recovery_key_fails_without_writes() -> TestResult {
    let location =
        SecretLocation::Keyring(KeyringLocation::new("polaris.synthetic", "snapshot.key")?);
    for value in [
        None,
        Some(""),
        Some("short"),
        Some("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
    ] {
        let source = source(value);
        assert!(
            SecretVault::with_seal_key_from_source(&location, &source, VaultLimits::new(0, 0))
                .is_err()
        );
        assert_eq!(source.writes.load(Ordering::SeqCst), 0);
    }
    let first = SecretVault::with_seal_key_from_source(
        &location,
        &source(Some(&"01".repeat(32))),
        VaultLimits::new(0, 0),
    )?;
    let second = SecretVault::with_seal_key_from_source(
        &location,
        &source(Some(&"02".repeat(32))),
        VaultLimits::new(0, 0),
    )?;
    assert!(second.open_bytes(&first.seal_bytes(b"canary")?).is_err());
    Ok(())
}
