//! Descriptor-backed contract tests. All credential values here are synthetic fixtures.
use std::sync::Mutex;

use polaris_sdk::secrets::{KeyringLocation, SecretLocation, SecretSource, SecretSourceError};
use polaris_security_secrets::{
    CapacityResource, MAX_KEYRING_SECRET_BYTES, MAX_SECRET_BYTES, SecretName, SecretVault,
    SystemSecretSource, VaultLimits,
};
use proptest::prelude::*;
use secrecy::{ExposeSecret, SecretString};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[derive(Debug, Default)]
struct FixtureSource {
    value: Mutex<Option<String>>,
    fail: bool,
}

impl FixtureSource {
    fn with_value(value: &str) -> Self {
        Self {
            value: Mutex::new(Some(value.to_owned())),
            fail: false,
        }
    }

    fn missing() -> Self {
        Self::default()
    }

    fn with_failure() -> Self {
        Self {
            value: Mutex::new(None),
            fail: true,
        }
    }
}

impl SecretSource for FixtureSource {
    fn load(&self, _location: &SecretLocation) -> Result<Option<SecretString>, SecretSourceError> {
        if self.fail {
            return Err(SecretSourceError::Failed);
        }
        let value = self
            .value
            .lock()
            .map_err(|_| SecretSourceError::Unavailable)?;
        Ok(value.as_deref().map(SecretString::from))
    }

    fn store(
        &self,
        _location: &SecretLocation,
        value: &SecretString,
    ) -> Result<(), SecretSourceError> {
        *self
            .value
            .lock()
            .map_err(|_| SecretSourceError::Unavailable)? = Some(value.expose_secret().to_owned());
        Ok(())
    }

    fn delete(&self, _location: &SecretLocation) -> Result<bool, SecretSourceError> {
        let mut value = self
            .value
            .lock()
            .map_err(|_| SecretSourceError::Unavailable)?;
        Ok(value.take().is_some())
    }
}

fn env_location() -> Result<SecretLocation, polaris_sdk::error::ParseError> {
    Ok(SecretLocation::Environment(
        polaris_sdk::secrets::EnvName::new("NEBIUS_API_KEY")?,
    ))
}

#[test]
fn seal_open_roundtrip_and_nonce_uniqueness() -> TestResult {
    let vault = SecretVault::new()?;
    let first = vault.seal_bytes(b"synthetic-secret-canary")?;
    let second = vault.seal_bytes(b"synthetic-secret-canary")?;
    assert_eq!(
        vault.open_bytes(&first)?.as_slice(),
        b"synthetic-secret-canary"
    );
    assert_ne!(first.nonce(), second.nonce());
    Ok(())
}

#[test]
fn tampered_ciphertext_is_rejected() -> TestResult {
    let vault = SecretVault::new()?;
    let mut sealed = vault.seal_bytes(b"fixture-value")?;
    let first = sealed
        .ciphertext_mut()
        .first_mut()
        .ok_or("authenticated ciphertext has no tag or message bytes")?;
    *first ^= 1;
    assert!(vault.open_bytes(&sealed).is_err());
    Ok(())
}

#[test]
fn fixture_source_loads_into_scoped_decryption() -> TestResult {
    let mut vault = SecretVault::new()?;
    let name = SecretName::new("NEBIUS_API_KEY")?;
    let source = FixtureSource::with_value("synthetic-secret-canary");
    vault.load(name.clone(), &env_location()?, &source)?;
    assert!(vault.contains(&name));
    let observed = vault.with_secret(&name, |value| {
        value == "synthetic-secret-canary" && value.len() == 23
    })?;
    assert!(observed);
    Ok(())
}

#[test]
fn missing_source_is_typed_and_closure_is_not_called() -> TestResult {
    let mut vault = SecretVault::new()?;
    let name = SecretName::new("NEBIUS_API_KEY")?;
    let source = FixtureSource::missing();
    assert!(vault.load(name.clone(), &env_location()?, &source).is_err());
    assert!(!vault.contains(&name));
    let mut called = false;
    assert!(vault.with_secret(&name, |_| called = true).is_err());
    assert!(!called);
    Ok(())
}

#[test]
fn provider_failure_is_typed_and_does_not_echo_backend_text() -> TestResult {
    let mut vault = SecretVault::new()?;
    let name = SecretName::new("NEBIUS_API_KEY")?;
    let source = FixtureSource::with_failure();
    let error = vault
        .load(name, &env_location()?, &source)
        .err()
        .ok_or("fixture failure should be returned")?;
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("synthetic-secret-canary"));
    assert!(!rendered.contains("provider supplied details"));
    Ok(())
}

#[test]
fn oversized_secret_is_rejected() -> TestResult {
    let mut vault = SecretVault::new()?;
    let name = SecretName::new("NEBIUS_API_KEY")?;
    let oversized = "x".repeat(MAX_SECRET_BYTES.saturating_add(1));
    let source = FixtureSource::with_value(&oversized);
    assert!(vault.load(name.clone(), &env_location()?, &source).is_err());
    assert!(!vault.contains(&name));
    Ok(())
}

#[test]
fn configured_vault_capacity_accounts_replacements_and_forget() -> TestResult {
    let mut vault = SecretVault::with_limits(VaultLimits::new(2, 12))?;
    let source = FixtureSource::with_value("123456");
    let first = SecretName::new("FIRST")?;
    let second = SecretName::new("SECOND")?;
    let third = SecretName::new("THIRD")?;
    let location = env_location()?;
    vault.load(first.clone(), &location, &source)?;
    vault.load(second.clone(), &location, &FixtureSource::with_value("123"))?;
    assert!(matches!(
        vault.load(third.clone(), &location, &FixtureSource::with_value("1")),
        Err(polaris_security_secrets::SecretsError::Capacity {
            resource: CapacityResource::Entries,
            limit: 2,
        })
    ));

    let replacement = FixtureSource::with_value("123456789");
    vault.load(first.clone(), &location, &replacement)?;
    vault.forget(&second);
    let third_value = FixtureSource::with_value("123");
    vault.load(third.clone(), &location, &third_value)?;
    assert!(vault.contains(&third));
    vault.forget(&first);
    let after_reclaim = FixtureSource::with_value("123");
    vault.load(second.clone(), &location, &after_reclaim)?;
    assert!(vault.contains(&second));
    Ok(())
}

#[test]
fn vault_aggregate_plaintext_limit_rejects_without_replacing_existing_entry() -> TestResult {
    let mut vault = SecretVault::with_limits(VaultLimits::new(4, 8))?;
    let location = env_location()?;
    let name = SecretName::new("FIRST")?;
    vault.load(
        name.clone(),
        &location,
        &FixtureSource::with_value("123456"),
    )?;
    let error = vault
        .load(
            name.clone(),
            &location,
            &FixtureSource::with_value("123456789"),
        )
        .err()
        .ok_or("oversized replacement should fail")?;
    assert!(matches!(
        error,
        polaris_security_secrets::SecretsError::Capacity {
            resource: CapacityResource::PlaintextBytes,
            limit: 8,
        }
    ));
    assert!(vault.with_secret(&name, |value| value == "123456")?);
    Ok(())
}

#[test]
fn oversized_ciphertext_is_rejected_before_decryption() -> TestResult {
    let vault = SecretVault::new()?;
    let mut sealed = vault.seal_bytes(b"fixture")?;
    sealed
        .ciphertext_mut()
        .resize(MAX_SECRET_BYTES.saturating_add(17), 0);
    assert!(vault.open_bytes(&sealed).is_err());
    Ok(())
}

#[test]
fn debug_output_redacts_vault_ciphertext_and_secret_wrapper() -> TestResult {
    let mut vault = SecretVault::new()?;
    let name = SecretName::new("NEBIUS_API_KEY")?;
    let source = FixtureSource::with_value("synthetic-secret-canary");
    vault.load(name, &env_location()?, &source)?;
    let rendered = format!(
        "{vault:?} {:?}",
        SecretString::from("synthetic-secret-canary")
    );
    assert!(!rendered.contains("synthetic-secret-canary"));
    Ok(())
}

#[test]
fn production_environment_adapter_rejects_mutation_without_reading_values() -> TestResult {
    let adapter = SystemSecretSource;
    let location = env_location()?;
    let value = SecretString::from("fixture-only-value");
    assert!(matches!(
        adapter.store(&location, &value),
        Err(SecretSourceError::Unsupported)
    ));
    assert!(matches!(
        adapter.delete(&location),
        Err(SecretSourceError::Unsupported)
    ));
    drop(KeyringLocation::new("polaris-test", "synthetic-account")?);
    Ok(())
}

#[test]
fn direct_system_keyring_store_rejects_oversized_value_before_backend_access() -> TestResult {
    let location = SecretLocation::Keyring(KeyringLocation::new(
        "polaris-test-oversize",
        "synthetic-account",
    )?);
    let value = SecretString::from("x".repeat(MAX_KEYRING_SECRET_BYTES.saturating_add(1)));
    assert!(matches!(
        SystemSecretSource.store(&location, &value),
        Err(SecretSourceError::TooLarge)
    ));
    Ok(())
}

#[test]
fn system_environment_adapter_reports_a_missing_synthetic_name() -> TestResult {
    let name = polaris_sdk::secrets::EnvName::new("POLARIS_TEST_SECRET_MISSING_6F3A1")?;
    let location = SecretLocation::Environment(name);
    assert!(SystemSecretSource.load(&location)?.is_none());
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn prop_secret_names_are_bounded_and_restricted(input in ".{0,96}") {
        if let Ok(name) = SecretName::new(&input) {
            prop_assert!(!name.as_str().is_empty());
            prop_assert!(name.as_str().len() <= 64);
            prop_assert!(name.as_str().bytes().all(|byte|
                byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'));
        }
    }
}

#[test]
fn keyring_store_load_and_delete_use_the_injected_fixture_provider() -> TestResult {
    let vault = SecretVault::new()?;
    let location = KeyringLocation::new("polaris-test", "synthetic-account")?;
    let source = FixtureSource::missing();
    let secret = SecretString::from("synthetic-keyring-value");
    vault.store_keyring(&location, &secret, &source)?;
    let keyring_location = SecretLocation::Keyring(location);
    let loaded = source
        .load(&keyring_location)?
        .ok_or("fixture lost stored value")?;
    assert_eq!(loaded.expose_secret(), "synthetic-keyring-value");
    assert!(source.delete(&keyring_location)?);
    assert!(source.load(&keyring_location)?.is_none());
    Ok(())
}

#[test]
#[ignore = "run only with POLARIS_SECRET_FIXTURE set to an isolated synthetic value"]
fn system_environment_adapter_reads_only_the_explicit_fixture_name() -> TestResult {
    let name = polaris_sdk::secrets::EnvName::new("POLARIS_SECRET_FIXTURE")?;
    let location = SecretLocation::Environment(name);
    let loaded = SystemSecretSource
        .load(&location)?
        .ok_or("isolated environment fixture was not supplied")?;
    assert_eq!(loaded.expose_secret(), "synthetic-environment-value");
    Ok(())
}

struct KeyringCleanup {
    location: SecretLocation,
    source: SystemSecretSource,
    owns_entry: bool,
}

impl Drop for KeyringCleanup {
    fn drop(&mut self) {
        if self.owns_entry {
            let _ = self.source.delete(&self.location);
        }
    }
}

#[test]
#[ignore = "opt in with POLARIS_TEST_KEYRING=1; creates and removes one random synthetic credential"]
fn system_keyring_store_load_delete_use_only_a_fresh_test_owned_location() -> TestResult {
    if std::env::var_os("POLARIS_TEST_KEYRING").is_none() {
        return Err("set POLARIS_TEST_KEYRING=1 to opt in".into());
    }
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)?;
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let location = SecretLocation::Keyring(KeyringLocation::new(
        "polaris-test-secret-contract",
        &format!("POLARIS_TEST_{suffix}"),
    )?);
    let source = SystemSecretSource;
    let mut cleanup = KeyringCleanup {
        location: location.clone(),
        source,
        owns_entry: false,
    };
    let canary = SecretString::from("synthetic-keyring-canary-only");
    source.store(&location, &canary)?;
    cleanup.owns_entry = true;
    let loaded = source
        .load(&location)?
        .ok_or("newly created test credential was missing")?;
    assert_eq!(loaded.expose_secret(), "synthetic-keyring-canary-only");
    assert!(source.delete(&location)?);
    cleanup.owns_entry = false;
    Ok(())
}
