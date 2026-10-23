use thiserror::Error;

/// Vector store validation, persistence, and index failures.
#[derive(Debug, Error)]
pub enum VectorStoreError {
    /// The table name or configured dimension is outside the accepted bounds.
    #[error("invalid vector table configuration: {0}")]
    InvalidConfiguration(String),
    /// The requested table conflicts with metadata already stored for that table.
    #[error("vector table metadata mismatch: {0}")]
    TableMismatch(String),
    /// A vector has a different dimension than its table.
    #[error("dimension mismatch: table has {expected}, got {found}")]
    DimensionMismatch { expected: usize, found: usize },
    /// A vector contains NaN or infinity, or is otherwise invalid for its metric.
    #[error("invalid vector: {0}")]
    InvalidVector(String),
    /// A mutation batch exceeds the configured bound.
    #[error("vector batch has {found} rows; maximum is {limit}")]
    BatchTooLarge { found: usize, limit: usize },
    /// SQLite rejected a query or could not open the database.
    #[error("vector SQLite error: {0}")]
    Sql(#[from] rusqlite::Error),
    /// Filesystem operation failed.
    #[error("vector filesystem error: {0}")]
    Io(#[from] std::io::Error),
    /// usearch rejected an index operation or artifact.
    #[error("usearch index error: {0}")]
    Index(String),
    /// A bounded worker pool for parallel native index insertion could not be created.
    #[error("vector insertion worker pool error: {0}")]
    WorkerPool(String),
    /// SQLite contains malformed vector records or unsafe index metadata.
    #[error("corrupt vector store: {0}")]
    Corrupt(String),
    /// The in-memory state lock was poisoned by a thread panic.
    #[error("vector store state lock is poisoned")]
    LockPoisoned,
}
