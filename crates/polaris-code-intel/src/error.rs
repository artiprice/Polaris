//! Errors of the code-intelligence engine.

use crate::lang::Lang;
use polaris_sdk::error::{ErrorKind, HasKind};

/// Failures of parsing, skeletons, repo maps, metrics and merges.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodeIntelError {
    /// The file is not in one of the supported languages.
    #[error("unsupported language")]
    UnsupportedLanguage,
    /// Parsing exceeded the configured time budget and was cancelled.
    #[error("parse exceeded its time budget")]
    ParseTimeout,
    /// The parser produced no tree (cancelled or no language set).
    #[error("the parser produced no tree")]
    ParseFailed,
    /// An input or an output exceeded a hard limit (source bytes, item count, token count).
    #[error("limit of {limit} exceeded")]
    TooLarge {
        /// The limit that was exceeded, in the unit of the limited quantity.
        limit: usize,
    },
    /// A grammar crate is incompatible with the linked tree-sitter runtime.
    #[error("grammar incompatible with the tree-sitter runtime: {0}")]
    GrammarVersionMismatch(String),
    /// Two trees of different languages were compared or merged.
    #[error("cannot combine {old:?} with {new:?}")]
    LanguageMismatch {
        /// Language of the first (old / base) input.
        old: Lang,
        /// Language of the second input.
        new: Lang,
    },
}

impl HasKind for CodeIntelError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::ParseTimeout => ErrorKind::Transient,
            Self::GrammarVersionMismatch(_) => ErrorKind::Fatal,
            Self::UnsupportedLanguage
            | Self::ParseFailed
            | Self::TooLarge { .. }
            | Self::LanguageMismatch { .. } => ErrorKind::Permanent,
        }
    }
}
