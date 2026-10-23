use std::collections::HashSet;

use polaris_sdk::ids::{Cid, ModelId, SessionId};

/// Distance metric stored as immutable table metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    /// Cosine similarity; returned scores are cosine similarity in [-1, 1].
    Cosine,
    /// Squared Euclidean distance; returned scores are the negated squared distance.
    L2,
    /// Inner product; returned scores are the inner product.
    Dot,
}

impl Metric {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Cosine => "cosine",
            Self::L2 => "l2",
            Self::Dot => "dot",
        }
    }
}

/// Persistent identity and embedding contract for a vector table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSpec {
    /// Stable table name, ASCII letters/digits plus `-`, `_`, or `.`.
    pub name: String,
    /// Embedding dimension, bounded to avoid unreasonable index allocations.
    pub dimension: usize,
    /// Embedding model identity; a table cannot mix embedding spaces.
    pub embedder: ModelId,
    /// Distance metric selected when the table is created.
    pub metric: Metric,
}

impl TableSpec {
    pub(crate) fn validate(&self) -> Result<(), crate::VectorStoreError> {
        let valid_name = !self.name.is_empty()
            && self.name.len() <= 128
            && self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if !valid_name {
            return Err(crate::VectorStoreError::InvalidConfiguration(
                "table name must be 1..=128 ASCII letters, digits, '-', '_' or '.'".to_owned(),
            ));
        }
        if self.dimension == 0 || self.dimension > 65_536 {
            return Err(crate::VectorStoreError::InvalidConfiguration(
                "dimension must be between 1 and 65536".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Immutable access boundary for an opened store handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VectorScope {
    /// Rows explicitly placed in the shared global partition.
    Global,
    /// Rows visible only to the named session.
    Session(SessionId),
}

impl VectorScope {
    pub(crate) fn key(self) -> Vec<u8> {
        match self {
            Self::Global => vec![0],
            Self::Session(session) => {
                let mut key = Vec::with_capacity(17);
                key.push(1);
                key.extend_from_slice(session.as_uuid().as_bytes());
                key
            }
        }
    }
}

/// Optional allow-list for filtering search results by CAS payload reference.
#[derive(Debug, Clone, Default)]
pub struct PayloadFilter {
    allowed: HashSet<Cid>,
}

impl PayloadFilter {
    /// Creates a filter that allows only the supplied payload references.
    #[must_use]
    pub fn only(payloads: impl IntoIterator<Item = Cid>) -> Self {
        Self {
            allowed: payloads.into_iter().collect(),
        }
    }

    pub(crate) fn allows(&self, payload: &Cid) -> bool {
        self.allowed.contains(payload)
    }

    pub(crate) fn allowed_is_empty(&self) -> bool {
        self.allowed.is_empty()
    }
}
