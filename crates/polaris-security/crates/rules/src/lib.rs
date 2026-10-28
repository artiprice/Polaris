//! Versioned security rule packs evaluated with ast-grep and bounded DFA scanners.
//!
//! Findings carry validated CWE/severity metadata and only a digest of matched source bytes.
//! Source scanning is restricted to caller-supplied changed byte spans. Coverage is explicit when
//! a language lacks configured rules or parsing cannot be completed.

mod arbitrate;
mod audit;
mod command;
mod error;
mod model;
mod pack;
mod supply_chain;

pub use arbitrate::{ArbitrationEvidence, ArbitrationReport, ArbitrationStatus, ArbitrationTrust};
pub use audit::{DiffAuditRequest, LanguageCoverage, RuleScanReport};
pub use error::RulesError;
pub use model::{Cwe, Finding, FindingCategory, RuleId, Severity};
pub use pack::{PackVersion, Rule, RuleEngine, RuleKind, RulePack, parse_pack};
pub use supply_chain::{
    ManifestAuditReport, ManifestAuditStatus, PackageEcosystem, TrustedPackageBaseline,
    TyposquatFinding,
};
