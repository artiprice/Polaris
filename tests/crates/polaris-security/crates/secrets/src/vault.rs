// Unit tests of crates/polaris-security/crates/secrets/src/vault.rs (`tests`); compiled into that module via #[path].
use super::{SecretName, SecretVault};

#[test]
fn associated_secret_name_prevents_ciphertext_relabeling() -> Result<(), super::SecretsError> {
    let vault = SecretVault::new()?;
    let original = SecretName::new("NEBIUS_API_KEY")?;
    let replacement = SecretName::new("TAVILY_API_KEY")?;
    let blob = vault.seal_named(original.as_str().as_bytes(), b"synthetic-canary")?;
    assert!(
        vault
            .open_named(replacement.as_str().as_bytes(), &blob)
            .is_err()
    );
    Ok(())
}

#[test]
fn session_key_and_ciphertext_use_distinct_allocations() -> Result<(), super::SecretsError> {
    let vault = SecretVault::new()?;
    let blob = vault.seal_bytes(b"synthetic-canary")?;
    let key_start = vault.key.0.as_ref().as_ptr();
    let ciphertext_start = blob.ciphertext.as_ptr();
    assert!(!core::ptr::eq(key_start, ciphertext_start));
    Ok(())
}
