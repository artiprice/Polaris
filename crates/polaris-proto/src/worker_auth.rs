//! Parent-issued bearer credentials for a private worker RPC channel.
//!
//! The parent creates a credential, transfers its 64 lowercase hexadecimal bytes over an
//! explicitly controlled channel (normally child stdin), and initializes the worker from those
//! bytes. The caller must not put the credential in argv or environment variables.
//!
//! This bearer check authenticates the caller to the worker only. It does not authenticate the
//! worker to the caller, establish endpoint ownership, enforce operating-system ACLs or sandbox
//! isolation, or prevent replay by a party that steals the token. The parent/client composition
//! must establish ownership of the endpoint and channel before sending the credential.

use std::io::{self, Write};

use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;
use tonic::metadata::{Ascii, MetadataKey, MetadataValue};
use tonic::service::Interceptor;
use tonic::{Request, Status};
use zeroize::{Zeroize, Zeroizing};

const TOKEN_BYTES: usize = 64;
const RANDOM_BYTES: usize = 32;

fn token_header() -> MetadataKey<Ascii> {
    MetadataKey::from_static("x-polaris-worker-token")
}

/// Failure to create, validate, or transfer a worker credential.
#[derive(Debug, Error)]
pub enum WorkerAuthError {
    /// Operating-system randomness was unavailable.
    #[error("worker credential randomness unavailable")]
    Randomness,
    /// The supplied secret was not exactly 64 lowercase hexadecimal bytes.
    #[error("invalid worker credential encoding")]
    InvalidEncoding,
    /// Memory reservation for an encoded credential failed.
    #[error("worker credential allocation failed")]
    Allocation,
}

/// A secret bearer credential issued by the parent to exactly one worker process.
///
/// Possession of this value authorizes calls to the matching worker. It does not prove the
/// identity of a server endpoint and must only be transmitted after the caller has established
/// ownership of the endpoint/channel through the surrounding composition.
///
/// The type intentionally implements no `Debug`, `Display`, or serialization trait.
///
/// ```compile_fail
/// use polaris_proto::worker_auth::WorkerCredential;
/// fn requires_debug<T: std::fmt::Debug>() {}
/// fn check_type_does_not_expose_debug() {
///     requires_debug::<WorkerCredential>();
/// }
/// ```
///
/// ```compile_fail
/// use polaris_proto::worker_auth::WorkerCredential;
/// fn requires_display<T: std::fmt::Display>() {}
/// fn check_type_does_not_expose_display() {
///     requires_display::<WorkerCredential>();
/// }
/// ```
pub struct WorkerCredential {
    secret: SecretString,
}

impl WorkerCredential {
    /// Generate a 256-bit credential using the operating-system random source.
    ///
    /// # Errors
    /// Returns [`WorkerAuthError::Randomness`] if system randomness is unavailable and
    /// [`WorkerAuthError::Allocation`] if the fixed-size encoded string cannot be reserved.
    pub fn generate() -> Result<Self, WorkerAuthError> {
        let mut random = Zeroizing::new([0_u8; RANDOM_BYTES]);
        getrandom::fill(&mut *random).map_err(|_| WorkerAuthError::Randomness)?;

        let mut encoded = Zeroizing::new(String::new());
        encoded
            .try_reserve_exact(TOKEN_BYTES)
            .map_err(|_| WorkerAuthError::Allocation)?;
        for byte in random.iter() {
            for nibble in [byte >> 4, byte & 0x0f] {
                let digit = match char::from_digit(u32::from(nibble), 16) {
                    Some(value) => value,
                    None => return Err(WorkerAuthError::InvalidEncoding),
                };
                encoded.push(digit);
            }
        }
        random.zeroize();
        let secret = SecretString::from(std::mem::take(&mut *encoded));
        Ok(Self { secret })
    }

    /// Initialize a worker credential from the exact secret bytes supplied by its parent.
    ///
    /// # Errors
    /// Returns [`WorkerAuthError::InvalidEncoding`] unless the secret contains exactly 64
    /// lowercase hexadecimal bytes. The rejected secret is zeroized when dropped.
    pub fn from_secret(secret: SecretString) -> Result<Self, WorkerAuthError> {
        let bytes = secret.expose_secret().as_bytes();
        if bytes.len() != TOKEN_BYTES
            || !bytes
                .iter()
                .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(WorkerAuthError::InvalidEncoding);
        }
        Ok(Self { secret })
    }

    /// Write the credential bytes to a parent-controlled transfer channel.
    ///
    /// The caller is responsible for controlling the destination and never logging its contents.
    /// The token is not included in any error produced by this method.
    ///
    /// # Errors
    /// Returns the writer's error if the destination fails before all credential bytes are written.
    pub fn write_to(&self, destination: &mut impl Write) -> io::Result<()> {
        destination.write_all(self.secret.expose_secret().as_bytes())
    }

    /// Attach exactly one sensitive worker bearer token to an outgoing tonic request.
    ///
    /// Any caller-supplied value under the reserved header is removed before the credential is
    /// installed, preventing accidental duplicate metadata values.
    ///
    /// # Errors
    /// Returns a generic internal status if tonic rejects the validated ASCII metadata value.
    pub fn authorize_request<T>(&self, mut request: Request<T>) -> Result<Request<T>, Status> {
        let mut token = MetadataValue::try_from(self.secret.expose_secret())
            .map_err(|_| Status::internal("worker credential metadata could not be encoded"))?;
        token.set_sensitive(true);
        let header = token_header();
        drop(request.metadata_mut().remove(header.clone()));
        drop(request.metadata_mut().insert(header, token));
        Ok(request)
    }
}

/// Tonic server interceptor that checks one parent-issued worker bearer credential.
///
/// Only a BLAKE3 digest is retained. Equality for `blake3::Hash` is constant-time in the pinned
/// blake3 implementation used by this workspace. Successful checks authenticate the caller to
/// this worker; they do not replace endpoint ACLs, process isolation, or client-side endpoint
/// ownership checks. A stolen bearer value can be replayed until the worker exits or rotates it.
#[derive(Clone)]
pub struct WorkerAuthenticator {
    expected: blake3::Hash,
}

impl WorkerAuthenticator {
    /// Build an authenticator from the secret configured for this worker process.
    #[must_use]
    pub fn new(credential: &WorkerCredential) -> Self {
        Self {
            expected: blake3::hash(credential.secret.expose_secret().as_bytes()),
        }
    }
}

impl Drop for WorkerAuthenticator {
    fn drop(&mut self) {
        self.expected.zeroize();
    }
}

impl Interceptor for WorkerAuthenticator {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        let mut values = request.metadata().get_all(token_header()).iter();
        let Some(value) = values.next() else {
            return Err(Status::unauthenticated("worker authentication failed"));
        };
        if values.next().is_some() {
            return Err(Status::unauthenticated("worker authentication failed"));
        }
        let token = value.as_bytes();
        if token.len() != TOKEN_BYTES
            || !token
                .iter()
                .all(|byte| matches!(*byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(Status::unauthenticated("worker authentication failed"));
        }
        if blake3::hash(token) != self.expected {
            return Err(Status::unauthenticated("worker authentication failed"));
        }
        drop(request.metadata_mut().remove(token_header()));
        Ok(request)
    }
}
