//! # polaris-security-automata
//!
//! Multi-pattern scanners compiled once at startup (L1, P2; pulled forward because
//! `polaris-tools-builtin` depends on it):
//!
//! - [`DfaScanner`] — one leftmost-first pass over all rules (regex-automata `meta`: lazy DFA,
//!   one-pass and literal prefilters chosen per set), start/end offsets into a caller-owned
//!   [`ScanBuf`]; the default pack ([`DEFAULT_RULES`]) covers secrets, PII and destructive shell
//!   commands. Scanning is allocation-free after warm-up (ADR-003, dhat-tested).
//! - [`validate_user_regex`] — bounds user patterns (length, nesting, repetition products, class
//!   size, compiled size): with linear-time engines, compilation size is the attack surface.
//! - [`shannon_entropy_milli`] / [`high_entropy_spans`] — integer fixed-point entropy for
//!   secret candidates.
//! - [`DfaRedactor`] — the sdk `Redactor` for logs and model-bound text.
//!
//! Residual risk: secrets that are encoded, split across tokens or disguised with homoglyphs or
//! zero-width characters are not detected.
//!
//! Test-first descriptors: `specs/descriptors/automata_*.json`.

mod entropy;
mod error;
mod guard;
mod redactor;
mod rules;
mod scan;

pub use entropy::{EntropyCfg, high_entropy_spans, shannon_entropy_milli};
pub use error::AutomataError;
pub use guard::{UserRegexLimits, ValidatedPattern, validate_user_regex};
pub use redactor::DfaRedactor;
pub use rules::DEFAULT_RULES;
pub use scan::{Category, CompileLimits, DfaScanner, Match, RuleId, RuleSetBuilder, ScanBuf};
