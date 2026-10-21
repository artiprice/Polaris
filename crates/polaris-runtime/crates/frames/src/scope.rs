//! Fail-closed attenuation rules for the capability crate's lexical scopes.
use polaris_security_capability::{ResourcePath, ResourceScope, Right, Scope};

/// Scope constraints held beside the bearer token so the runtime can prove child monotonicity.
/// The capability token remains authoritative at every action boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameScope {
    /// Signed capability checks used when creating the frame.
    pub capability: Scope,
    /// Absolute expiry from the trusted clock at issuance, in microseconds.
    pub expires_at_micros: i64,
}

/// Why a requested child scope could not be proven narrower than its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScopeError {
    /// Child resource is broader or incomparable.
    #[error("child resource scope is not a proven subset")]
    Resource,
    /// Child rights include a right absent from the parent.
    #[error("child rights exceed the parent")]
    Rights,
    /// Child churn limit exceeds the parent.
    #[error("child churn limit exceeds the parent")]
    Churn,
    /// Child enables unsafe mutations where the parent forbids them.
    #[error("child unsafe allowance exceeds the parent")]
    Unsafe,
    /// Child omitted or changed a parent exact-diff binding.
    #[error("child diff binding does not preserve the parent binding")]
    Diff,
    /// Requested lifetime does not fit within the parent's remaining lifetime.
    #[error("child lifetime exceeds the parent's remaining lifetime")]
    Lifetime,
}

/// Prove all non-time capability constraints are monotone. Unknown relations reject.
pub fn scope_is_subset(parent: &Scope, child: &Scope) -> Result<(), ScopeError> {
    if !resource_subset(&parent.resource, &child.resource) {
        return Err(ScopeError::Resource);
    }
    for right in Right::ALL {
        if child.rights.contains(right) && !parent.rights.contains(right) {
            return Err(ScopeError::Rights);
        }
    }
    if child.max_churn_lines > parent.max_churn_lines {
        return Err(ScopeError::Churn);
    }
    if child.allow_unsafe && !parent.allow_unsafe {
        return Err(ScopeError::Unsafe);
    }
    if let Some(parent_hash) = parent.diff_blake3 {
        if child.diff_blake3 != Some(parent_hash) {
            return Err(ScopeError::Diff);
        }
    }
    Ok(())
}

fn resource_subset(parent: &ResourceScope, child: &ResourceScope) -> bool {
    match (parent, child) {
        (ResourceScope::Exact(parent_path), ResourceScope::Exact(child_path)) => {
            parent_path == child_path
        }
        (ResourceScope::Exact(parent_path), ResourceScope::Symbol { path, .. }) => {
            parent_path == path
        }
        (ResourceScope::Prefix(parent_path), ResourceScope::Exact(child_path)) => {
            is_same_or_descendant(parent_path, child_path)
        }
        (ResourceScope::Prefix(parent_path), ResourceScope::Prefix(child_path)) => {
            is_same_or_descendant(parent_path, child_path)
        }
        (ResourceScope::Prefix(parent_path), ResourceScope::Symbol { path, .. }) => {
            is_same_or_descendant(parent_path, path)
        }
        (
            ResourceScope::Symbol { path: parent_path, symbol: parent_symbol },
            ResourceScope::Symbol { path: child_path, symbol: child_symbol },
        ) => parent_path == child_path && parent_symbol == child_symbol,
        (ResourceScope::Symbol { .. }, ResourceScope::Exact(_) | ResourceScope::Prefix(_)) => false,
        (ResourceScope::Exact(_), ResourceScope::Prefix(_)) => false,
    }
}

fn is_same_or_descendant(parent: &ResourcePath, child: &ResourcePath) -> bool {
    child == parent || child.as_str().strip_prefix(parent.as_str()).is_some_and(|rest| rest.starts_with('/'))
}

/// Validate the relative lifetime against a previously recorded absolute parent expiry.
pub(crate) fn child_expiry(parent_expires_at_micros: i64, now_micros: i64, ttl_micros: u128) -> Result<i64, ScopeError> {
    let ttl = i64::try_from(ttl_micros).map_err(|_| ScopeError::Lifetime)?;
    if ttl <= 0 || now_micros >= parent_expires_at_micros {
        return Err(ScopeError::Lifetime);
    }
    let expiry = now_micros.checked_add(ttl).ok_or(ScopeError::Lifetime)?;
    if expiry > parent_expires_at_micros {
        return Err(ScopeError::Lifetime);
    }
    Ok(expiry)
}