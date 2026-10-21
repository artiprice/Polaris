//! Parse errors. Offsets are byte offsets into the input, always on char boundaries.

use polaris_sdk::error::{ErrorKind, HasKind};

/// Why a message could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DirectiveError {
    /// The input exceeds `ParseLimits::max_input_bytes`.
    #[error("input too long")]
    TooLong,
    /// Parentheses nest deeper than `ParseLimits::max_depth`.
    #[error("directive arguments nested too deeply")]
    TooDeep,
    /// More arguments than `ParseLimits::max_args` in one directive.
    #[error("too many directive arguments")]
    TooManyArgs,
    /// The `(` at `at` is never closed.
    #[error("unbalanced parenthesis opened at byte {at}")]
    UnbalancedParen {
        /// Offset of the unclosed `(`.
        at: usize,
    },
    /// The quote at `at` is never closed.
    #[error("unterminated quote opened at byte {at}")]
    UnterminatedQuote {
        /// Offset of the opening quote.
        at: usize,
    },
    /// An effort range is malformed or reversed (`lo > hi`).
    #[error(
        "invalid effort range at byte {at}: expected low|medium|high|xhigh|max or 1-5, lo <= hi"
    )]
    BadEffortRange {
        /// Offset of the directive or command.
        at: usize,
    },
    /// `/name` is not a known command.
    #[error("unknown command /{0}")]
    UnknownCommand(String),
    /// A known directive or command got arguments it does not accept.
    #[error("invalid arguments for {directive} at byte {at}: {reason}")]
    BadArg {
        /// Directive (`$name`) or command (`/name`).
        directive: String,
        /// Offset of the directive or command.
        at: usize,
        /// What was wrong.
        reason: &'static str,
    },
}

impl HasKind for DirectiveError {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Permanent
    }
}
