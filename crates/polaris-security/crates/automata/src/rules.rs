//! The default rule pack: secrets, PII and destructive shell commands.
//!
//! Patterns are ASCII byte-mode (see [`crate::RuleSetBuilder::build`]). Coverage is deliberately
//! conservative; residual risk (encoded, split or homoglyph secrets) is documented in the PLAN.

use crate::error::AutomataError;
use crate::scan::{Category, CompileLimits, DfaScanner};

/// `(name, pattern, category)` of every default rule.
pub const DEFAULT_RULES: &[(&str, &str, Category)] = &[
    (
        "aws_access_key_id",
        r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
        Category::Secret,
    ),
    (
        "private_key_block",
        r"-----BEGIN (?:RSA |EC |OPENSSH |DSA |ENCRYPTED )?PRIVATE KEY-----",
        Category::Secret,
    ),
    (
        "github_token",
        r"\bgh[pousr]_[A-Za-z0-9]{36,255}\b",
        Category::Secret,
    ),
    ("sk_api_key", r"\bsk-[A-Za-z0-9_-]{20,}", Category::Secret),
    (
        "slack_token",
        r"\bxox[abprs]-[A-Za-z0-9-]{10,}",
        Category::Secret,
    ),
    (
        "tavily_api_key",
        r"\btvly-[A-Za-z0-9_-]{20,}",
        Category::Secret,
    ),
    (
        "bearer_header",
        r"(?i)\bauthorization:[ \t]*bearer[ \t]+[A-Za-z0-9._~+/=-]{16,}",
        Category::Secret,
    ),
    (
        "jwt",
        r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
        Category::Secret,
    ),
    (
        "secret_assignment",
        r#"(?i)\b(?:api[_-]?key|secret|token|passw(?:or)?d)["']?[ \t]*[:=][ \t]*["']?[A-Za-z0-9_\-/+=.]{16,}"#,
        Category::Secret,
    ),
    (
        "email",
        r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}\b",
        Category::Pii,
    ),
    (
        "ipv4",
        r"\b(?:(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])\.){3}(?:25[0-5]|2[0-4][0-9]|1[0-9]{2}|[1-9]?[0-9])\b",
        Category::Pii,
    ),
    (
        "rm_rf_root",
        r"(?m)\brm[ \t]+(?:-[A-Za-z]+[ \t]+)*-[A-Za-z]*[rR][A-Za-z]*[ \t]+(?:--no-preserve-root[ \t]+)?(?:/|~|\$HOME)(?:/?\*)?(?:[ \t;&|]|$)",
        Category::DangerousShell,
    ),
    (
        "curl_pipe_shell",
        r"\b(?:curl|wget)\b[^\n|]*\|[ \t]*(?:sudo[ \t]+)?(?:ba|z|da)?sh\b",
        Category::DangerousShell,
    ),
    (
        "fork_bomb",
        r":\(\)[ \t]*\{[ \t]*:[ \t]*\|[ \t]*:[ \t]*&[ \t]*\}[ \t]*;[ \t]*:",
        Category::DangerousShell,
    ),
    (
        "dd_to_device",
        r"\bdd\b[^\n]*\bof=/dev/(?:sd[a-z]|nvme[0-9]|hd[a-z]|disk[0-9]|mmcblk[0-9])",
        Category::DangerousShell,
    ),
    (
        "mkfs_device",
        r"\bmkfs(?:\.[a-z0-9]+)?[ \t]+/dev/",
        Category::DangerousShell,
    ),
    (
        "chmod_777_root",
        r"(?m)\bchmod[ \t]+-R[ \t]+0?777[ \t]+/(?:[ \t;&|]|$)",
        Category::DangerousShell,
    ),
    (
        "windows_format_drive",
        r"(?i)\bformat(?:\.com)?[ \t]+[a-z]:",
        Category::DangerousShell,
    ),
    (
        "windows_rd_drive_root",
        r"(?im)\b(?:rd|rmdir)[ \t]+/s[ \t]+/q[ \t]+[a-z]:\\?(?:[ \t]|$)",
        Category::DangerousShell,
    ),
];

impl DfaScanner {
    /// Every default rule.
    ///
    /// # Errors
    /// Only if the built-in pack fails to compile under the default limits (a test guards this).
    pub fn default_pack() -> Result<Self, AutomataError> {
        Self::default_pack_for(&[Category::Secret, Category::Pii, Category::DangerousShell])
    }

    /// The default rules of the given categories.
    ///
    /// # Errors
    /// As [`DfaScanner::default_pack`].
    pub fn default_pack_for(categories: &[Category]) -> Result<Self, AutomataError> {
        let mut builder = Self::builder();
        for (name, pattern, category) in DEFAULT_RULES {
            if categories.contains(category) {
                builder.add(name, pattern, *category)?;
            }
        }
        builder.build(&CompileLimits::default())
    }
}
