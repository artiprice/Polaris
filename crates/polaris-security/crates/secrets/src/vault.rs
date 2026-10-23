//! ChaCha20-Poly1305 sealing and bounded secret storage.

use std::collections::HashMap;
use std::fmt;

use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce, Tag,
    aead::{AeadInOut, KeyInit},
};
use getrandom::fill;
use polaris_sdk::secrets::{KeyringLocation, SecretLocation, SecretSource};
use secrecy::{ExposeSecret, SecretString};
use zeroize::{Zeroize, Zeroizing};

use crate::error::SecretsError;

/// Maximum size of a secret value, in bytes.
pub const MAX_SECRET_BYTES: usize = 65_536;
const NONCE_BYTES: usize = 12;
const KEY_BYTES: usize = 32;
const TAG_BYTES: usize = 16;
/// Maximum plaintext accepted by Windows Credential Manager's pinned keyring backend.
///
/// `windows-native-keyring-store 1.1.0` validates against
/// `windows-sys 0.61.2::CRED_MAX_CREDENTIAL_BLOB_SIZE`, which is 2560 bytes.
pub const MAX_KEYRING_SECRET_BYTES: usize = 2560;
const DEFAULT_MAX_ENTRIES: usize = 64;
const DEFAULT_MAX_AGGREGATE_BYTES: usize = 1_048_576;

/// Validated secret identifier used as authenticated associated data.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretName(Box<str>);

impl SecretName {
    /// Validates a secret name in `[A-Z0-9_]{1,64}`.
    ///
    /// # Errors
    /// [`SecretsError::InvalidName`] when the name is empty, too long, or contains
    /// unsupported characters.
    pub fn new(name: &str) -> Result<Self, SecretsError> {
        if name.is_empty()
            || name.len() > 64
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(SecretsError::InvalidName);
        }
        Ok(Self(name.into()))
    }

    /// Returns the validated secret identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretName([REDACTED])")
    }
}

/// A random nonce and authenticated ciphertext. Ciphertext bytes are zeroized on drop.
pub struct SealedBlob {
    nonce: [u8; NONCE_BYTES],
    ciphertext: Zeroizing<Vec<u8>>,
}

impl SealedBlob {
    /// Constructs a sealed blob from persisted components after enforcing its size bound.
    ///
    /// # Errors
    /// [`SecretsError::TooLarge`] when the ciphertext exceeds the maximum accepted bound.
    pub fn from_parts(nonce: [u8; NONCE_BYTES], ciphertext: Vec<u8>) -> Result<Self, SecretsError> {
        let limit = MAX_SECRET_BYTES.saturating_add(TAG_BYTES);
        if ciphertext.len() > limit {
            return Err(SecretsError::TooLarge {
                bytes: ciphertext.len(),
                limit,
            });
        }
        Ok(Self {
            nonce,
            ciphertext: Zeroizing::new(ciphertext),
        })
    }

    /// Returns the public 96-bit nonce.
    #[must_use]
    pub const fn nonce(&self) -> &[u8; NONCE_BYTES] {
        &self.nonce
    }

    /// Returns authenticated ciphertext bytes. This does not reveal plaintext.
    #[must_use]
    pub fn ciphertext(&self) -> &[u8] {
        self.ciphertext.as_slice()
    }

    /// Mutably exposes ciphertext for serialization adapters and integrity tests.
    pub fn ciphertext_mut(&mut self) -> &mut Vec<u8> {
        &mut self.ciphertext
    }
}

impl fmt::Debug for SealedBlob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedBlob")
            .field("nonce", &"[REDACTED]")
            .field("ciphertext_bytes", &self.ciphertext.len())
            .finish()
    }
}

/// Ephemeral 256-bit in-memory sealing key; never copied or exposed via formatting.
///
/// Session key types intentionally implement no formatting or serialization traits.
///
/// ```compile_fail
/// use polaris_security_secrets::SessionSealKey;
/// fn needs_debug<T: core::fmt::Debug>() {}
/// needs_debug::<SessionSealKey>();
/// ```
///
/// ```compile_fail
/// use polaris_security_secrets::SessionSealKey;
/// fn needs_display<T: core::fmt::Display>() {}
/// needs_display::<SessionSealKey>();
/// ```
///
/// ```compile_fail
/// use polaris_security_secrets::SessionSealKey;
/// fn needs_serialize<T: serde::Serialize>() {}
/// needs_serialize::<SessionSealKey>();
/// ```
pub struct SessionSealKey(Box<Zeroizing<[u8; KEY_BYTES]>>);

/// Bounds applied to the in-memory secret vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultLimits {
    max_entries: usize,
    max_plaintext_bytes: usize,
}

impl VaultLimits {
    /// Creates vault limits; zero is valid and creates a vault that rejects all entries.
    #[must_use]
    pub const fn new(max_entries: usize, max_plaintext_bytes: usize) -> Self {
        Self {
            max_entries,
            max_plaintext_bytes,
        }
    }

    /// Maximum number of entries.
    #[must_use]
    pub const fn max_entries(self) -> usize {
        self.max_entries
    }

    /// Maximum aggregate plaintext byte count.
    #[must_use]
    pub const fn max_plaintext_bytes(self) -> usize {
        self.max_plaintext_bytes
    }
}

#[derive(Debug)]
struct EntryRecord {
    sealed: SealedBlob,
    plaintext_bytes: usize,
}

/// In-memory secret vault. The key and ciphertexts live in distinct heap allocations.
pub struct SecretVault {
    key: SessionSealKey,
    entries: HashMap<SecretName, EntryRecord>,
    limits: VaultLimits,
    plaintext_bytes: usize,
}

impl fmt::Debug for SecretVault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretVault")
            .field("key", &"[REDACTED]")
            .field("entry_count", &self.entries.len())
            .field("plaintext_bytes", &self.plaintext_bytes)
            .field("limits", &self.limits)
            .finish()
    }
}

impl SecretVault {
    /// Creates an empty vault with an independently generated session key.
    ///
    /// # Errors
    /// [`SecretsError::Entropy`] when the operating system secure random source fails.
    pub fn new() -> Result<Self, SecretsError> {
        Self::with_limits(VaultLimits::new(
            DEFAULT_MAX_ENTRIES,
            DEFAULT_MAX_AGGREGATE_BYTES,
        ))
    }

    /// Creates a vault with caller-selected entry and aggregate plaintext bounds.
    ///
    /// # Errors
    /// [`SecretsError::Entropy`] when the operating system secure random source fails.
    pub fn with_limits(limits: VaultLimits) -> Result<Self, SecretsError> {
        let mut key = Box::new(Zeroizing::new([0; KEY_BYTES]));
        fill(&mut **key).map_err(|_| SecretsError::Entropy)?;
        Ok(Self {
            key: SessionSealKey(key),
            entries: HashMap::new(),
            limits,
            plaintext_bytes: 0,
        })
    }

    /// Loads and seals a value from the provided registry source.
    ///
    /// # Errors
    /// Returns a typed source error, a source-specific missing error, or a size/entropy/sealing
    /// error. No source error includes OS text or secret values.
    pub fn load(
        &mut self,
        name: SecretName,
        location: &SecretLocation,
        source: &impl SecretSource,
    ) -> Result<(), SecretsError> {
        let value = match source.load(location)? {
            Some(value) => value,
            None => return Err(missing_error(&name, location)),
        };
        let plaintext = value.expose_secret().as_bytes();
        validate_size(plaintext.len())?;
        self.check_capacity(&name, plaintext.len())?;
        let sealed = self.seal_named(name.as_str().as_bytes(), plaintext)?;
        self.insert(name, sealed, plaintext.len());
        Ok(())
    }

    /// Loads from the host credential store or process environment through the production adapter.
    ///
    /// # Errors
    /// Returns a typed source error, a source-specific missing error, or a size/entropy/sealing
    /// error.
    pub fn load_system(
        &mut self,
        name: SecretName,
        location: &SecretLocation,
    ) -> Result<(), SecretsError> {
        self.load(name, location, &crate::SystemSecretSource)
    }

    /// Stores a secret in the keyring via the injected registry provider.
    ///
    /// # Errors
    /// Returns a typed provider error. Environment locations are unsupported for writes.
    pub fn store_keyring(
        &self,
        location: &KeyringLocation,
        value: &SecretString,
        source: &impl SecretSource,
    ) -> Result<(), SecretsError> {
        let location = SecretLocation::Keyring(location.clone());
        validate_size(value.expose_secret().len())?;
        source.store(&location, value).map_err(Into::into)
    }

    /// Stores a secret in the real platform keyring.
    ///
    /// # Errors
    /// Returns a sanitized provider error or a size-bound error.
    pub fn store_keyring_system(
        &self,
        location: &KeyringLocation,
        value: &SecretString,
    ) -> Result<(), SecretsError> {
        self.store_keyring(location, value, &crate::SystemSecretSource)
    }

    /// Invokes `f` with temporary UTF-8 plaintext decrypted from the named entry.
    ///
    /// # Errors
    /// [`SecretsError::NotFound`] when no entry exists and [`SecretsError::Open`] when
    /// authentication or UTF-8 validation fails.
    ///
    /// Plaintext is borrowed only for this callback. The borrow cannot be returned from the
    /// callback, although callers can of course deliberately copy the text while inside it.
    ///
    /// ```compile_fail
    /// use polaris_security_secrets::{SecretName, SecretVault, SecretsError};
    ///
    /// fn escape<'a>(
    ///     vault: &'a SecretVault,
    ///     name: &SecretName,
    /// ) -> Result<&'a str, SecretsError> {
    ///     vault.with_secret(name, |plaintext| plaintext)
    /// }
    /// ```
    pub fn with_secret<R>(
        &self,
        name: &SecretName,
        f: impl FnOnce(&str) -> R,
    ) -> Result<R, SecretsError> {
        let blob = self
            .entries
            .get(name)
            .ok_or_else(|| SecretsError::NotFound(name.clone()))?;
        let plaintext = self.open_named(name.as_str().as_bytes(), &blob.sealed)?;
        let text = core::str::from_utf8(&plaintext).map_err(|_| SecretsError::Open)?;
        Ok(f(text))
    }

    /// Returns whether a named secret is present in the vault.
    #[must_use]
    pub fn contains(&self, name: &SecretName) -> bool {
        self.entries.contains_key(name)
    }

    /// Removes a secret; the stored ciphertext allocation is zeroized during drop.
    pub fn forget(&mut self, name: &SecretName) {
        if let Some(entry) = self.entries.remove(name) {
            self.plaintext_bytes = self.plaintext_bytes.saturating_sub(entry.plaintext_bytes);
        }
    }

    /// Seals arbitrary bounded bytes for in-memory consumers such as the PII vault.
    ///
    /// # Errors
    /// [`SecretsError::TooLarge`] when input exceeds [`MAX_SECRET_BYTES`], or entropy/encryption
    /// failures.
    pub fn seal_bytes(&self, plaintext: &[u8]) -> Result<SealedBlob, SecretsError> {
        validate_size(plaintext.len())?;
        self.seal_named(&[], plaintext)
    }

    /// Opens a blob created by [`Self::seal_bytes`], returning zeroizing plaintext bytes.
    ///
    /// # Errors
    /// [`SecretsError::Open`] for an invalid nonce, ciphertext, or authentication tag.
    pub fn open_bytes(&self, blob: &SealedBlob) -> Result<Zeroizing<Vec<u8>>, SecretsError> {
        self.open_named(&[], blob)
    }

    fn seal_named(
        &self,
        associated_data: &[u8],
        plaintext: &[u8],
    ) -> Result<SealedBlob, SecretsError> {
        validate_size(plaintext.len())?;
        let mut nonce = [0; NONCE_BYTES];
        fill(&mut nonce).map_err(|_| SecretsError::Entropy)?;
        let mut key = Key::from(**self.key.0);
        let cipher = ChaCha20Poly1305::new(&key);
        key.as_mut_slice().zeroize();
        let nonce_value = Nonce::from(nonce);
        let mut encrypted = Zeroizing::new(plaintext.to_vec());
        let tag = cipher
            .encrypt_inout_detached(
                &nonce_value,
                associated_data,
                encrypted.as_mut_slice().into(),
            )
            .map_err(|_| SecretsError::Seal)?;
        encrypted
            .try_reserve(TAG_BYTES)
            .map_err(|_| SecretsError::Seal)?;
        encrypted.extend_from_slice(tag.as_slice());
        let encrypted = core::mem::take(&mut *encrypted);
        SealedBlob::from_parts(nonce, encrypted)
    }

    fn open_named(
        &self,
        associated_data: &[u8],
        blob: &SealedBlob,
    ) -> Result<Zeroizing<Vec<u8>>, SecretsError> {
        let ciphertext_limit = MAX_SECRET_BYTES.saturating_add(TAG_BYTES);
        if blob.ciphertext.len() > ciphertext_limit {
            return Err(SecretsError::TooLarge {
                bytes: blob.ciphertext.len(),
                limit: ciphertext_limit,
            });
        }
        let mut key = Key::from(**self.key.0);
        let cipher = ChaCha20Poly1305::new(&key);
        key.as_mut_slice().zeroize();
        let nonce = Nonce::from(*blob.nonce());
        let mut plaintext = Zeroizing::new(blob.ciphertext().to_vec());
        let tag_start = plaintext
            .len()
            .checked_sub(TAG_BYTES)
            .ok_or(SecretsError::Open)?;
        let (message, tag_bytes) = plaintext.split_at_mut(tag_start);
        let tag = Tag::try_from(&*tag_bytes).map_err(|_| SecretsError::Open)?;
        cipher
            .decrypt_inout_detached(&nonce, associated_data, message.into(), &tag)
            .map_err(|_| SecretsError::Open)?;
        plaintext.truncate(tag_start);
        Ok(plaintext)
    }

    fn check_capacity(&self, name: &SecretName, incoming_bytes: usize) -> Result<(), SecretsError> {
        let prior_bytes = self
            .entries
            .get(name)
            .map_or(0, |entry| entry.plaintext_bytes);
        let next_bytes = self
            .plaintext_bytes
            .saturating_sub(prior_bytes)
            .checked_add(incoming_bytes)
            .ok_or(SecretsError::Capacity {
                resource: crate::CapacityResource::PlaintextBytes,
                limit: self.limits.max_plaintext_bytes,
            })?;
        if next_bytes > self.limits.max_plaintext_bytes {
            return Err(SecretsError::Capacity {
                resource: crate::CapacityResource::PlaintextBytes,
                limit: self.limits.max_plaintext_bytes,
            });
        }
        if !self.entries.contains_key(name) && self.entries.len() >= self.limits.max_entries {
            return Err(SecretsError::Capacity {
                resource: crate::CapacityResource::Entries,
                limit: self.limits.max_entries,
            });
        }
        Ok(())
    }

    fn insert(&mut self, name: SecretName, sealed: SealedBlob, plaintext_bytes: usize) {
        if let Some(previous) = self.entries.insert(
            name,
            EntryRecord {
                sealed,
                plaintext_bytes,
            },
        ) {
            self.plaintext_bytes = self
                .plaintext_bytes
                .saturating_sub(previous.plaintext_bytes);
        }
        self.plaintext_bytes = self.plaintext_bytes.saturating_add(plaintext_bytes);
    }
}

fn validate_size(bytes: usize) -> Result<(), SecretsError> {
    if bytes > MAX_SECRET_BYTES {
        return Err(SecretsError::TooLarge {
            bytes,
            limit: MAX_SECRET_BYTES,
        });
    }
    Ok(())
}

fn missing_error(name: &SecretName, location: &SecretLocation) -> SecretsError {
    match location {
        SecretLocation::Environment(env_name) => {
            SecretsError::EnvMissing(env_name.as_str().to_owned())
        }
        SecretLocation::Keyring(_) => SecretsError::NotFound(name.clone()),
    }
}

#[cfg(test)]
mod tests {
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
}
