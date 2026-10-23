//! Embedded, scope-bound vector storage using SQLite as the authority and usearch as its index.
//!
//! Each [`HnswStore`] is bound at construction to one [`TableSpec`] and one [`VectorScope`].
//! SQLite commits are authoritative; immutable, generation-tagged HNSW files are atomically
//! published caches and are rebuilt from committed rows when absent, stale, or corrupt.

mod error;
mod store;
mod types;

pub use error::VectorStoreError;
pub use store::{HnswStore, MAX_BATCH_SIZE};
pub use types::{Metric, PayloadFilter, TableSpec, VectorScope};
