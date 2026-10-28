//! Stable public finding types and CWE validation.

use polaris_sdk::ids::Cid;
use std::path::PathBuf;

/// Rule identifier within its versioned pack.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleId(String);

impl RuleId {
    /// Creates a bounded, printable rule identifier.
    ///
    /// # Errors
    /// Returns [`crate::RulesError::InvalidRuleId`] if the value is empty, too long or contains
    /// characters outside the stable ASCII identifier alphabet.
    pub fn new(value: &str) -> Result<Self, crate::RulesError> {
        let valid = !value.is_empty()
            && value.len() <= 96
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(crate::RulesError::InvalidRuleId)
        }
    }

    /// The stable identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Supported, explicitly mapped Common Weakness Enumeration identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cwe(u16);

impl Cwe {
    /// Validates an identifier against this library's explicit CWE mapping table.
    ///
    /// # Errors
    /// Returns [`crate::RulesError::UnknownCwe`] unless this versioned library explicitly maps
    /// the identifier to a supported weakness category.
    pub const fn new(value: u16) -> Result<Self, crate::RulesError> {
        match value {
            22 | 59 | 78 | 125 | 362 | 415 | 416 | 787 | 798 => Ok(Self(value)),
            _ => Err(crate::RulesError::UnknownCwe(value)),
        }
    }

    /// Numeric CWE identifier.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Finding severity from the versioned pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Informational review hint.
    Info,
    /// Low impact or weakly exploitable condition.
    Low,
    /// Material weakness with bounded impact.
    Medium,
    /// Serious security weakness.
    High,
    /// Critical security weakness.
    Critical,
}

/// Why the rule produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingCategory {
    /// Structural AST pattern.
    Ast,
    /// Secret-shaped DFA match.
    Secret,
    /// Dangerous shell DFA match.
    DangerousShell,
}

impl FindingCategory {
    /// Stable lowercase category label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ast => "ast",
            Self::Secret => "secret",
            Self::DangerousShell => "dangerous_shell",
        }
    }
}

/// A redacted structured security finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub(crate) rule: RuleId,
    pub(crate) cwe: Option<Cwe>,
    pub(crate) severity: Severity,
    pub(crate) path: PathBuf,
    pub(crate) span: polaris_code_intel::ByteSpan,
    pub(crate) snippet_digest: Cid,
    pub(crate) category: FindingCategory,
    pub(crate) message: String,
    pub(crate) disputed: bool,
}

impl Finding {
    /// Rule that matched.
    #[must_use]
    pub const fn rule(&self) -> &RuleId {
        &self.rule
    }

    /// Mapped CWE, or `None` for heuristics without a sound CWE mapping.
    #[must_use]
    pub const fn cwe(&self) -> Option<Cwe> {
        self.cwe
    }

    /// Rule severity.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// Normalized relative path or the virtual `<command>` path.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Matched byte span in the source or command.
    #[must_use]
    pub const fn span(&self) -> polaris_code_intel::ByteSpan {
        self.span
    }

    /// BLAKE3 content id of the matched bytes. Raw matched bytes are never retained.
    #[must_use]
    pub const fn snippet_digest(&self) -> Cid {
        self.snippet_digest
    }

    /// Rule category.
    #[must_use]
    pub const fn category(&self) -> FindingCategory {
        self.category
    }

    /// Pack-authored explanation with no source text interpolation.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whether the deterministic rule marks this result as needing arbitration.
    #[must_use]
    pub const fn disputed(&self) -> bool {
        self.disputed
    }
}
