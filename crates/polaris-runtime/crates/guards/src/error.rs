//! Guard input and provider errors.

use polaris_sdk::error::{ErrorKind, HasKind};

/// Failures while inspecting guard inputs or provider facts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GuardError {
    /// A workspace-relative path is malformed or escapes the workspace root.
    #[error("invalid workspace path")]
    InvalidPath,
    /// A glob expression could not be compiled.
    #[error("invalid workspace glob")]
    InvalidGlob,
    /// The injected workspace view failed to return facts.
    #[error("workspace view failed")]
    Workspace(ViewError),
    /// A checked total exceeded the representable range.
    #[error("guard accounting overflow")]
    ArithmeticOverflow,
    /// A configured bound is zero or internally inconsistent.
    #[error("invalid guard limits")]
    InvalidLimits,
    /// AST impact enumeration exceeded its explicit bound.
    #[error("changed symbol limit exceeded ({limit})")]
    TooManySymbols {
        /// Maximum changed symbols accepted for one analysis.
        limit: usize,
    },
}

impl HasKind for GuardError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Workspace(ViewError::Unavailable) => ErrorKind::Transient,
            Self::InvalidPath | Self::InvalidGlob | Self::InvalidLimits => ErrorKind::Policy,
            Self::Workspace(ViewError::InvalidFacts)
            | Self::ArithmeticOverflow
            | Self::TooManySymbols { .. } => ErrorKind::Fatal,
        }
    }
}

/// Errors returned by injected workspace and AST-reference providers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ViewError {
    /// The provider is temporarily unavailable.
    #[error("provider unavailable")]
    Unavailable,
    /// The provider returned facts that violate the requested contract.
    #[error("provider returned invalid facts")]
    InvalidFacts,
}

impl HasKind for ViewError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Unavailable => ErrorKind::Transient,
            Self::InvalidFacts => ErrorKind::Fatal,
        }
    }
}
