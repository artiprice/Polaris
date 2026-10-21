//! Cache keys: scope + kind + a domain-separated, length-prefixed blake3 digest.
//!
//! blake3 (never xxh3) for every key: S1 safety verdicts are cached, and a non-cryptographic hash
//! would let crafted inputs collide with a cached "allow" (PLAN risk "safety-verdict poisoning").
//! The scope is hashed into the digest because the sdk `ExactCacheStore::get` looks up by key only:
//! isolation must come from the key itself.

use polaris_sdk::ids::{Cid, ModelId};
use polaris_sdk::registry::CacheScope;

/// blake3 `derive_key` context (domain separation from every other blake3 use in Polaris).
const CONTEXT: &str = "polaris-storage-cache 2026-10-21 cache key v1";

/// What a cached value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyKind {
    /// A System-One decision (safety-relevant).
    S1Decision,
    /// A System-Two completion.
    S2Completion,
    /// An embedding vector.
    Embedding,
}

impl KeyKind {
    const fn tag(self) -> u8 {
        match self {
            Self::S1Decision => 1,
            Self::S2Completion => 2,
            Self::Embedding => 3,
        }
    }
}

/// A derived cache key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    scope: CacheScope,
    kind: KeyKind,
    digest: [u8; 32],
}

impl CacheKey {
    /// The 32-byte digest (the L1 and L2 lookup key).
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// The kind of value.
    #[must_use]
    pub const fn kind(&self) -> KeyKind {
        self.kind
    }

    /// The scope (`/cache purge`, `$storage-level`).
    #[must_use]
    pub const fn scope(&self) -> &CacheScope {
        &self.scope
    }
}

struct KeyHasher {
    scope: CacheScope,
    kind: KeyKind,
    inner: blake3::Hasher,
}

impl KeyHasher {
    fn new(scope: CacheScope, kind: KeyKind) -> Self {
        let mut inner = blake3::Hasher::new_derive_key(CONTEXT);
        inner.update(&[kind.tag()]);
        match &scope {
            CacheScope::Global => {
                inner.update(&[0]);
            }
            CacheScope::Workspace { workspace } => {
                inner.update(&[1]);
                inner.update(workspace.as_bytes());
            }
            CacheScope::Session { session } => {
                inner.update(&[2]);
                inner.update(session.as_uuid().as_bytes());
            }
        }
        Self { scope, kind, inner }
    }

    /// Appends a length-prefixed field, so field boundaries are unambiguous.
    fn field(mut self, bytes: &[u8]) -> Self {
        let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.inner.update(&len.to_le_bytes());
        self.inner.update(bytes);
        self
    }

    fn finish(self) -> CacheKey {
        CacheKey {
            scope: self.scope,
            kind: self.kind,
            digest: *self.inner.finalize().as_bytes(),
        }
    }
}

/// Key for an S2 completion: model, sampling-parameters digest, stable prompt-prefix `Cid`, tail.
#[must_use]
pub fn key_for_completion(
    scope: CacheScope,
    model: &ModelId,
    params_digest: &[u8; 32],
    prefix: &Cid,
    tail: &[u8],
) -> CacheKey {
    KeyHasher::new(scope, KeyKind::S2Completion)
        .field(model.as_str().as_bytes())
        .field(params_digest)
        .field(prefix.as_bytes())
        .field(tail)
        .finish()
}

/// Key for an S1 decision: schema digest and input bytes.
#[must_use]
pub fn key_for_s1(scope: CacheScope, schema_digest: &[u8; 32], input: &[u8]) -> CacheKey {
    KeyHasher::new(scope, KeyKind::S1Decision)
        .field(schema_digest)
        .field(input)
        .finish()
}

/// Key for an embedding: model and text bytes.
#[must_use]
pub fn key_for_embedding(scope: CacheScope, model: &ModelId, text: &[u8]) -> CacheKey {
    KeyHasher::new(scope, KeyKind::Embedding)
        .field(model.as_str().as_bytes())
        .field(text)
        .finish()
}
