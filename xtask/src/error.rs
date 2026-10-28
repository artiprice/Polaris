//! Error type for all xtask operations.

/// Failure while loading the crate table, scanning the tree or querying cargo.
#[derive(Debug, thiserror::Error)]
pub enum XtaskError {
    /// Calibration source, measured threshold, encoding or artifact creation failed.
    #[error(transparent)]
    Calibration(#[from] crate::calibration::CalibrationRunError),
    /// Filesystem access failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// The crate table is not valid TOML or has an unexpected shape.
    #[error("invalid crate table: {0}")]
    Table(#[from] toml::de::Error),
    /// `cargo metadata` produced output that is not the expected JSON.
    #[error("invalid cargo metadata JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// `cargo metadata` exited unsuccessfully.
    #[error("`cargo metadata` failed ({status}): {stderr}")]
    Cargo {
        /// Exit status description.
        status: String,
        /// Captured standard error.
        stderr: String,
    },
    /// Command-line usage error.
    #[error("usage: {0}")]
    Usage(String),
}
