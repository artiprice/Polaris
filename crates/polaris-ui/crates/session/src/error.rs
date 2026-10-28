//! Failure taxonomy of the session host; messages are identical to the terminal client's.

/// Why a session host operation failed. Messages never contain credentials.
#[derive(Debug, thiserror::Error)]
pub enum SessionHostError {
    /// A validated storage, CAS or adapter operation failed (message is already sanitized).
    #[error("{0}")]
    Ui(String),
    /// A host owner failed or an invariant does not hold.
    #[error("{0}")]
    Unavailable(&'static str),
    /// Kernel storage/lifecycle failure.
    #[error(transparent)]
    Kernel(#[from] polaris_kernel::KernelError),
    /// Local control rejection (grammar, bounds, authorization, queue, persistence).
    #[error(transparent)]
    Control(#[from] polaris_sdk::registry::KernelError),
}
