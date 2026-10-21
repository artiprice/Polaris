//! AST symbol scopes used by the file lease manager.

use super::key::LeaseKey;
use polaris_code_intel::Lang;
use super::manager::LeaseError;

/// A validated qualified symbol path, represented by bounded named components.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SymbolPath(Box<[Box<str>]>);

impl SymbolPath {
    /// Creates a symbol path from qualified components.
    ///
    /// # Errors
    /// Returns [`LeaseError::InvalidScope`] for empty, oversized, or malformed components.
    pub fn new<I, S>(parts: I) -> Result<Self, LeaseError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut out = Vec::new();
        for part in parts {
            let value = part.as_ref();
            if value.is_empty() || value.len() > 256 || value.contains("::") || value.chars().any(char::is_whitespace) {
                return Err(LeaseError::InvalidScope);
            }
            if out.len() >= 128 {
                return Err(LeaseError::LimitExceeded);
            }
            out.push(value.into());
        }
        if out.is_empty() {
            return Err(LeaseError::InvalidScope);
        }
        Ok(Self(out.into_boxed_slice()))
    }

    /// Returns the number of qualified components.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether this path contains no components.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the component slice.
    #[must_use]
    pub fn components(&self) -> &[Box<str>] {
        &self.0
    }
}

/// Exclusive whole-file scope or an AST-resolved symbol scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LeaseScope {
    /// Excludes all other scopes for the same file.
    EntireFile,
    /// A syntactic symbol; ancestor and descendant symbol paths conflict.
    AstNode(SymbolPath),
}

impl LeaseScope {
    /// Creates a symbol lease only when a parser-supported language and an AST-resolved path are supplied.
    /// The caller must resolve the symbol against the parsed source before this call. Unknown languages conservatively receive whole-file scope.
    #[must_use]
    pub fn for_language(language: Option<Lang>, symbol: SymbolPath) -> Self {
        match language { Some(_) => Self::AstNode(symbol), None => Self::EntireFile }
    }
}

/// Determines whether two scopes on the same key overlap.
#[must_use]
pub fn scopes_conflict(left_key: &LeaseKey, left: &LeaseScope, right_key: &LeaseKey, right: &LeaseScope) -> bool {
    if left_key != right_key {
        return false;
    }
    match (left, right) {
        (LeaseScope::EntireFile, _) | (_, LeaseScope::EntireFile) => true,
        (LeaseScope::AstNode(a), LeaseScope::AstNode(b)) => {
            is_prefix(a, b) || is_prefix(b, a)
        }
    }
}

fn is_prefix(prefix: &SymbolPath, candidate: &SymbolPath) -> bool {
    prefix.len() <= candidate.len()
        && prefix.components().iter().zip(candidate.components()).all(|(a, b)| a == b)
}