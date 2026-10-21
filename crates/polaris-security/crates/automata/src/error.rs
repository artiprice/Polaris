//! Scanner and guard errors. Messages never echo scanned input (it may contain secrets).

use polaris_sdk::error::{ErrorKind, HasKind};

/// Rule compilation, guard and scan failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AutomataError {
    /// The pattern is not valid regex syntax.
    #[error("invalid pattern: {0}")]
    Syntax(String),
    /// A size bound was exceeded (pattern length, compiled size, class size or haystack length).
    #[error("size limit exceeded (limit {limit})")]
    TooLarge {
        /// The limit that was exceeded.
        limit: usize,
    },
    /// Group nesting exceeded the limit.
    #[error("pattern nesting exceeds the limit")]
    TooNested,
    /// A counted repetition (or a product of nested ones) exceeds the limit.
    #[error("counted repetition exceeds the limit {limit}")]
    RepetitionTooLarge {
        /// Maximum allowed repetition product.
        limit: u32,
    },
    /// More patterns than `CompileLimits::max_patterns`.
    #[error("too many patterns (limit {limit})")]
    TooManyPatterns {
        /// Maximum allowed patterns.
        limit: u32,
    },
    /// Two rules share a name.
    #[error("duplicate rule name {0:?}")]
    DuplicateRule(String),
    /// The caller's match buffer is full.
    #[error("match buffer full (capacity {capacity})")]
    BufferFull {
        /// Buffer capacity.
        capacity: usize,
    },
}

impl HasKind for AutomataError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::BufferFull { .. } => ErrorKind::Transient,
            Self::Syntax(_)
            | Self::TooLarge { .. }
            | Self::TooNested
            | Self::RepetitionTooLarge { .. }
            | Self::TooManyPatterns { .. }
            | Self::DuplicateRule(_) => ErrorKind::Permanent,
        }
    }
}
