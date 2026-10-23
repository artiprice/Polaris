//! Per-RPC capability authentication. Transport authentication never replaces command verification.
use polaris_proto::pb;
use polaris_security_capability::{
    Authorization, CapabilityError, Facts, PublicKeyBytes, ResourcePath, Right, SystemClock, Token,
    authorize_with_revocations,
};
use std::sync::Arc;
use tonic::{
    Request, Status,
    metadata::{KeyAndValueRef, MetadataMap},
};

/// Maximum ASCII bearer header size, including scheme and hex encoding.
pub const MAX_BEARER_HEADER_BYTES: usize = 2 * polaris_security_capability::MAX_TOKEN_BYTES + 7;
/// Maximum number of metadata entries accepted per RPC.
pub const MAX_METADATA_ENTRIES: usize = 32;
/// Maximum encoded bytes across all incoming metadata values and keys.
pub const MAX_METADATA_BYTES: usize = MAX_BEARER_HEADER_BYTES + 4096;

/// RPC subject passed to a trusted context resolver. Request values are hints only; the resolver
/// must establish workspace-contained facts from trusted state before returning them.
pub enum RequestSubject<'a> {
    /// Command being submitted to the kernel.
    SubmitCommand(&'a pb::CommandBody),
    /// Session event subscription request.
    Subscribe(&'a pb::SubscribeRequest),
    /// Terminal stream attachment.
    AttachTerminal,
    /// Liveness request.
    Health,
}

/// Trusted operation facts supplied by the composition root after resolving request references.
#[derive(Debug, Clone)]
pub struct TrustedFacts {
    /// Lexical workspace-relative resource; symlink containment must already be established.
    pub resource: ResourcePath,
    /// Exact qualified symbol, when trusted analysis found one.
    pub symbol: Option<String>,
    /// Actual observed line churn.
    pub churn_lines: u32,
    /// Actual trusted unsafe-change observation.
    pub touches_unsafe: bool,
    /// Hash of the exact proposed mutation, when applicable.
    pub diff_blake3: Option<polaris_sdk::ids::Cid>,
}

/// Resolver for operation facts. Implementations are security-sensitive and must not simply echo
/// untrusted request data as trusted filesystem facts.
pub trait TrustedFactsResolver: Send + Sync + 'static {
    /// Resolve actual facts for this method and decoded request.
    fn resolve(&self, right: Right, subject: RequestSubject<'_>) -> Result<TrustedFacts, Status>;

    /// Check whether a token has been revoked since its stream or RPC was authorized.
    ///
    /// The default is no revocation source. A production composition root that supports revocation
    /// must override this method; external revocation remains pending otherwise.
    fn is_revoked(&self, _token: &Token) -> Result<bool, Status> {
        Ok(false)
    }
}

/// Biscuit verifier configuration shared by every RPC method.
pub struct RpcAuthenticator<R> {
    root: PublicKeyBytes,
    resolver: Arc<R>,
}

/// Revalidatable capability lease for one authorized operation or stream.
///
/// It retains only the verified bearer, pinned authorization facts, and root key. Every check
/// reevaluates expiration and asks the trusted resolver for revocation state.
pub struct AuthorizationLease<R> {
    token: Token,
    facts: TrustedFacts,
    right: Right,
    root: PublicKeyBytes,
    resolver: Arc<R>,
}
impl<R> Clone for AuthorizationLease<R> {
    fn clone(&self) -> Self {
        Self {
            token: self.token.clone(),
            facts: self.facts.clone(),
            right: self.right,
            root: self.root,
            resolver: Arc::clone(&self.resolver),
        }
    }
}
impl<R: TrustedFactsResolver> AuthorizationLease<R> {
    /// Recheck token expiry, revocation, resource, mutation facts, and method right.
    /// # Errors
    /// Returns `UNAUTHENTICATED` for revocation or invalid credentials and `PERMISSION_DENIED`
    /// when a valid signature fails a scope, time, or method-right check. The capability
    /// evaluator deliberately does not expose which sensitive Datalog constraint failed.
    pub fn revalidate(&self) -> Result<(), Status> {
        if self.resolver.is_revoked(&self.token)? {
            return Err(Status::unauthenticated("capability revoked"));
        }
        let facts = Facts {
            resource: &self.facts.resource,
            symbol: self.facts.symbol.as_deref(),
            churn_lines: self.facts.churn_lines,
            touches_unsafe: self.facts.touches_unsafe,
            diff_blake3: self.facts.diff_blake3,
            right: self.right,
        };
        match authorize_with_revocations(&self.token, &facts, &self.root, &SystemClock, &[])
            .map_err(map_capability_error)?
        {
            Authorization::Allowed => Ok(()),
            Authorization::Denied { .. } => Err(Status::permission_denied("capability denied")),
        }
    }
}

impl<R: TrustedFactsResolver> RpcAuthenticator<R> {
    /// Create a fail-closed authenticator for one pinned issuer and trusted context resolver.
    #[must_use]
    pub fn new(root: PublicKeyBytes, resolver: R) -> Self {
        Self {
            root,
            resolver: Arc::new(resolver),
        }
    }

    /// Require and authorize the bearer credential for a decoded RPC operation.
    ///
    /// # Errors
    /// Returns `UNAUTHENTICATED` for missing, malformed, or invalid credentials and
    /// `PERMISSION_DENIED` when a valid credential lacks the required right. Context resolver
    /// errors are preserved as tonic statuses.
    pub fn authorize<T>(
        &self,
        request: &Request<T>,
        right: Right,
        subject: RequestSubject<'_>,
        embedded_command_token: Option<&[u8]>,
    ) -> Result<AuthorizationLease<R>, Status> {
        let metadata = request.metadata();
        if metadata.len() > MAX_METADATA_ENTRIES {
            return Err(Status::resource_exhausted("too many metadata entries"));
        }
        let metadata_bytes = metadata.iter().fold(0_usize, |total, pair| {
            let (key, value) = match pair {
                KeyAndValueRef::Ascii(key, value) => (key.as_str().len(), value.as_bytes().len()),
                KeyAndValueRef::Binary(key, value) => {
                    (key.as_str().len(), value.as_encoded_bytes().len())
                }
            };
            total.saturating_add(key).saturating_add(value)
        });
        if metadata_bytes > MAX_METADATA_BYTES {
            return Err(Status::resource_exhausted("metadata too large"));
        }
        let bearer = bearer_bytes(metadata)?;
        if let Some(command_token) = embedded_command_token
            && command_token != bearer.as_slice()
        {
            return Err(Status::unauthenticated(
                "command and bearer capabilities do not match",
            ));
        }
        let token = Token::from_bytes(&bearer, &self.root).map_err(map_capability_error)?;
        let trusted = self.resolver.resolve(right, subject)?;
        let lease = AuthorizationLease {
            token,
            facts: trusted,
            right,
            root: self.root,
            resolver: Arc::clone(&self.resolver),
        };
        lease.revalidate()?;
        Ok(lease)
    }
}

fn bearer_bytes(metadata: &MetadataMap) -> Result<Vec<u8>, Status> {
    let values = metadata.get_all("authorization");
    let mut entries = values.iter();
    let Some(value) = entries.next() else {
        return Err(Status::unauthenticated("missing bearer credential"));
    };
    if entries.next().is_some() {
        return Err(Status::unauthenticated("duplicate bearer credential"));
    }
    let encoded = value
        .to_str()
        .map_err(|_| Status::unauthenticated("invalid bearer credential"))?;
    if encoded.len() > MAX_BEARER_HEADER_BYTES {
        return Err(Status::resource_exhausted("bearer credential too large"));
    }
    let hex = encoded
        .strip_prefix("Bearer ")
        .ok_or_else(|| Status::unauthenticated("bearer scheme required"))?;
    if hex.is_empty() || hex.len() % 2 != 0 {
        return Err(Status::unauthenticated("malformed bearer credential"));
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let mut nibbles = hex.bytes();
    while let (Some(high), Some(low)) = (nibbles.next(), nibbles.next()) {
        let high = hex_nibble(high)
            .ok_or_else(|| Status::unauthenticated("malformed bearer credential"))?;
        let low = hex_nibble(low)
            .ok_or_else(|| Status::unauthenticated("malformed bearer credential"))?;
        bytes.push((high << 4) | low);
    }
    if bytes.len() > polaris_security_capability::MAX_TOKEN_BYTES {
        return Err(Status::resource_exhausted("bearer credential too large"));
    }
    Ok(bytes)
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn map_capability_error(_error: CapabilityError) -> Status {
    Status::unauthenticated("invalid capability")
}
