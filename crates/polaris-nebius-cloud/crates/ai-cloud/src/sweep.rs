//! Exact ownership-marker filtering for crash-recovery sweeps.

use std::collections::HashMap;

use thiserror::Error;

/// Errors returned while constructing an owner marker.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum OwnershipError {
    /// Both label key and value must be nonempty.
    #[error("ownership marker key and value cannot be empty")]
    EmptyMarker,
}

/// Exact label key/value required to claim ownership of a resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerMarker {
    key: String,
    value: String,
}

/// Provider resource identifier stored without interpreting provider semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceId(String);

/// Resource ID plus labels returned by a provider listing operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceLabels {
    id: ResourceId,
    labels: HashMap<String, String>,
}

impl OwnerMarker {
    /// Construct an exact marker; prefix or partial matches are never accepted.
    ///
    /// The caller must bound the key and value lengths and reject control characters
    /// before construction; this type treats their contents as opaque provider data.
    pub fn new(key: String, value: String) -> Result<Self, OwnershipError> {
        if key.is_empty() || value.is_empty() {
            return Err(OwnershipError::EmptyMarker);
        }
        Ok(Self { key, value })
    }
}

impl ResourceId {
    /// Return the provider ID as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ResourceLabels {
    /// Build a provider resource description for ownership filtering.
    ///
    /// The caller must bound the ID and label count/key/value lengths and reject
    /// control characters before construction.
    #[must_use]
    pub fn new(id: String, labels: HashMap<String, String>) -> Self {
        Self {
            id: ResourceId(id),
            labels,
        }
    }
}

/// Return only IDs carrying the exact ownership label.
///
/// This function only selects candidates. It performs no provider calls or deletion.
/// The caller must pass a bounded provider listing (for example, one capped page),
/// since every matching ID is collected into the returned vector.
#[must_use]
pub fn select_owned_resources(
    resources: impl IntoIterator<Item = ResourceLabels>,
    owner_marker: &OwnerMarker,
) -> Vec<ResourceId> {
    resources
        .into_iter()
        .filter(|resource| resource.labels.get(&owner_marker.key) == Some(&owner_marker.value))
        .map(|resource| resource.id)
        .collect()
}
