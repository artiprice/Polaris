//! Production OS credential-manager and process-environment adapter.

use keyring::Entry;
use polaris_sdk::secrets::{
    EnvName, KeyringLocation, SecretLocation, SecretSource, SecretSourceError,
};
use secrecy::{ExposeSecret, SecretString};
use zeroize::{Zeroize, Zeroizing};

/// System adapter backed by the platform credential store and read-only environment access.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemSecretSource;

impl SystemSecretSource {
    fn entry(location: &KeyringLocation) -> Result<Entry, SecretSourceError> {
        Entry::new(location.service(), location.account()).map_err(map_keyring_error)
    }

    fn load_environment(name: &EnvName) -> Result<Option<SecretString>, SecretSourceError> {
        match std::env::var_os(name.as_str()) {
            None => Ok(None),
            Some(value) => match value.into_string() {
                Ok(value) => {
                    let value = Zeroizing::new(value);
                    if value.len() > crate::MAX_SECRET_BYTES {
                        return Err(SecretSourceError::TooLarge);
                    }
                    Ok(Some(SecretString::from(value.as_str())))
                }
                Err(_) => Err(SecretSourceError::Failed),
            },
        }
    }

    fn load_keyring(location: &KeyringLocation) -> Result<Option<SecretString>, SecretSourceError> {
        let entry = Self::entry(location)?;
        let bytes = match entry.get_secret() {
            Ok(bytes) => Zeroizing::new(bytes),
            Err(error) if is_missing(&error) => return Ok(None),
            Err(error) => return Err(map_keyring_error(error)),
        };
        if bytes.len() > crate::MAX_SECRET_BYTES {
            return Err(SecretSourceError::TooLarge);
        }
        let value =
            core::str::from_utf8(bytes.as_slice()).map_err(|_| SecretSourceError::Failed)?;
        Ok(Some(SecretString::from(value)))
    }
}

impl SecretSource for SystemSecretSource {
    fn load(&self, location: &SecretLocation) -> Result<Option<SecretString>, SecretSourceError> {
        match location {
            SecretLocation::Environment(name) => Self::load_environment(name),
            SecretLocation::Keyring(location) => Self::load_keyring(location),
        }
    }

    fn store(
        &self,
        location: &SecretLocation,
        value: &SecretString,
    ) -> Result<(), SecretSourceError> {
        match location {
            SecretLocation::Environment(_) => Err(SecretSourceError::Unsupported),
            SecretLocation::Keyring(location) => {
                let bytes = value.expose_secret().as_bytes();
                if bytes.len() > crate::MAX_KEYRING_SECRET_BYTES {
                    return Err(SecretSourceError::TooLarge);
                }
                Self::entry(location)?
                    .set_secret(bytes)
                    .map_err(map_keyring_error)
            }
        }
    }

    fn delete(&self, location: &SecretLocation) -> Result<bool, SecretSourceError> {
        match location {
            SecretLocation::Environment(_) => Err(SecretSourceError::Unsupported),
            SecretLocation::Keyring(location) => match Self::entry(location)?.delete_credential() {
                Ok(()) => Ok(true),
                Err(error) if is_missing(&error) => Ok(false),
                Err(error) => Err(map_keyring_error(error)),
            },
        }
    }
}

fn is_missing(error: &keyring::Error) -> bool {
    matches!(error, keyring::Error::NoEntry)
}

fn map_keyring_error(error: keyring::Error) -> SecretSourceError {
    match error {
        keyring::Error::NoDefaultStore => SecretSourceError::Unavailable,
        keyring::Error::NoEntry => SecretSourceError::Failed,
        keyring::Error::NoStorageAccess(_) => SecretSourceError::PermissionDenied,
        keyring::Error::Invalid(_, _) => SecretSourceError::InvalidLocation,
        keyring::Error::TooLong(_, _) => SecretSourceError::TooLarge,
        keyring::Error::BadEncoding(mut bytes) => {
            bytes.zeroize();
            SecretSourceError::Failed
        }
        keyring::Error::BadDataFormat(mut bytes, _) => {
            bytes.zeroize();
            SecretSourceError::Failed
        }
        _ => SecretSourceError::Failed,
    }
}
