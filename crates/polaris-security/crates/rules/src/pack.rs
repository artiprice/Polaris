//! Versioned, schema-validated rule packs and compilation.

use crate::error::RulesError;
use crate::model::{Cwe, FindingCategory, RuleId, Severity};
use ast_grep_config::from_str;
use polaris_code_intel::{Lang, validate_ast_pattern};
use polaris_security_automata::{Category, CompileLimits, DfaScanner, RuleSetBuilder};
use serde::Deserialize;
use std::collections::BTreeSet;

/// Maximum UTF-8 YAML pack size.
pub const MAX_PACK_BYTES: usize = 1_048_576;
/// Maximum number of rules in a single pack.
pub const MAX_RULES_PER_PACK: usize = 256;
/// Maximum total rules accepted by one engine.
pub const MAX_ENGINE_RULES: usize = 1024;
/// Maximum pattern length in bytes.
pub const MAX_PATTERN_BYTES: usize = 16_384;

/// Positive integer pack version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PackVersion(u32);

impl PackVersion {
    /// Returns the positive version number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// One validated security rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    id: RuleId,
    cwe: Option<Cwe>,
    severity: Severity,
    message: String,
    disputed: bool,
    kind: RuleKind,
}

impl Rule {
    /// Stable rule identifier.
    #[must_use]
    pub const fn id(&self) -> &RuleId {
        &self.id
    }

    /// CWE mapping when one is sound and explicit.
    #[must_use]
    pub const fn cwe(&self) -> Option<Cwe> {
        self.cwe
    }

    /// Rule severity.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    /// Static rule message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whether the deterministic match is marked for S1 review.
    #[must_use]
    pub const fn disputed(&self) -> bool {
        self.disputed
    }

    /// Pattern implementation.
    #[must_use]
    pub const fn kind(&self) -> &RuleKind {
        &self.kind
    }
}

/// Supported rule pattern types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleKind {
    /// Structural syntax pattern compiled with ast-grep.
    AstPattern { lang: Lang, pattern: String },
    /// DFA pattern scanned in exactly one category.
    DfaPattern {
        pattern: String,
        category: FindingCategory,
    },
}

/// One versioned pack of validated rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulePack {
    id: String,
    version: PackVersion,
    rules: Vec<Rule>,
}

impl RulePack {
    /// Parses, bounds and validates a versioned YAML rule pack.
    ///
    /// # Errors
    /// Returns a typed error for malformed YAML, schema violations, unknown CWEs, duplicate ids,
    /// unsupported languages, invalid patterns or exceeded bounds.
    pub fn parse(yaml: &str) -> Result<Self, RulesError> {
        if yaml.len() > MAX_PACK_BYTES {
            return Err(RulesError::Bound(format!(
                "pack exceeds {MAX_PACK_BYTES} bytes"
            )));
        }
        let raw: RawPack = from_str(yaml).map_err(|error| RulesError::Yaml(error.to_string()))?;
        validate_pack_id(&raw.id)?;
        if raw.version == 0 {
            return Err(RulesError::Bound(String::from(
                "pack version must be positive",
            )));
        }
        if raw.rules.is_empty() || raw.rules.len() > MAX_RULES_PER_PACK {
            return Err(RulesError::Bound(format!(
                "pack rule count must be 1..={MAX_RULES_PER_PACK}"
            )));
        }
        let mut ids = BTreeSet::new();
        let mut rules = Vec::with_capacity(raw.rules.len());
        for raw_rule in raw.rules {
            let id = RuleId::new(&raw_rule.id)?;
            if !ids.insert(id.as_str().to_owned()) {
                return Err(RulesError::DuplicateId(id.as_str().to_owned()));
            }
            let cwe = raw_rule.cwe.map(Cwe::new).transpose()?;
            let severity = parse_severity(&raw_rule.severity)?;
            if raw_rule.message.trim().is_empty() || raw_rule.message.len() > 1024 {
                return Err(RulesError::Bound(format!(
                    "invalid message for {}",
                    id.as_str()
                )));
            }
            let kind = match (
                raw_rule.language,
                raw_rule.ast_pattern,
                raw_rule.dfa_pattern,
                raw_rule.dfa_category,
            ) {
                (Some(language), Some(pattern), None, None) => {
                    let lang = parse_language(&language)?;
                    validate_pattern(&id, &pattern)?;
                    validate_ast_pattern(lang, &pattern).map_err(|error| {
                        RulesError::BadPattern {
                            rule: id.as_str().to_owned(),
                            reason: error.to_string(),
                        }
                    })?;
                    RuleKind::AstPattern { lang, pattern }
                }
                (None, None, Some(pattern), Some(category)) => {
                    validate_pattern(&id, &pattern)?;
                    let category = parse_category(&category)?;
                    RuleKind::DfaPattern { pattern, category }
                }
                _ => return Err(RulesError::InvalidRuleKind),
            };
            rules.push(Rule {
                id,
                cwe,
                severity,
                message: raw_rule.message,
                disputed: raw_rule.disputed,
                kind,
            });
        }
        Ok(Self {
            id: raw.id,
            version: PackVersion(raw.version),
            rules,
        })
    }

    /// Stable pack identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Pack version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version.get()
    }

    /// Validated rules in source order.
    #[must_use]
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }
}

/// Parses and validates one bounded versioned YAML rule pack.
///
/// # Errors
/// Returns a typed error for malformed YAML, schema violations, unknown CWEs, duplicate ids,
/// unsupported languages, invalid patterns or exceeded bounds.
pub fn parse_pack(yaml: &str) -> Result<RulePack, RulesError> {
    RulePack::parse(yaml)
}

/// Compiled engine containing bounded AST patterns and one category-aware DFA scanner.
#[derive(Debug, Clone)]
pub struct RuleEngine {
    pub(crate) packs: Vec<RulePack>,
    pub(crate) scanner: Option<DfaScanner>,
    pub(crate) dfa_rules: Vec<Rule>,
}

impl RuleEngine {
    /// Compiles validated packs and rejects duplicate ids or excessive aggregate rule counts.
    ///
    /// # Errors
    /// Returns a typed error when packs conflict, exceed limits or contain an invalid DFA rule.
    pub fn load(packs: &[RulePack]) -> Result<Self, RulesError> {
        let total_rules = packs
            .iter()
            .try_fold(0_usize, |total, pack| total.checked_add(pack.rules.len()))
            .ok_or_else(|| RulesError::Bound(String::from("rule count overflow")))?;
        if packs.is_empty() || total_rules > MAX_ENGINE_RULES {
            return Err(RulesError::Bound(format!(
                "engine rule count must be 1..={MAX_ENGINE_RULES}"
            )));
        }
        let mut pack_ids = BTreeSet::new();
        let mut rule_ids = BTreeSet::new();
        let mut dfa_rules = Vec::new();
        let mut scanner_builder = RuleSetBuilder::default();
        for pack in packs {
            if !pack_ids.insert(pack.id.clone()) {
                return Err(RulesError::DuplicateId(pack.id.clone()));
            }
            for rule in &pack.rules {
                if !rule_ids.insert(rule.id.as_str().to_owned()) {
                    return Err(RulesError::DuplicateId(rule.id.as_str().to_owned()));
                }
                if let RuleKind::DfaPattern { pattern, category } = &rule.kind {
                    let scanner_category = match category {
                        FindingCategory::Secret => Category::Secret,
                        FindingCategory::DangerousShell => Category::DangerousShell,
                        FindingCategory::Ast => {
                            return Err(RulesError::InvalidRuleKind);
                        }
                    };
                    scanner_builder
                        .add(rule.id.as_str(), pattern, scanner_category)
                        .map_err(|error| RulesError::Dfa(error.to_string()))?;
                    dfa_rules.push(rule.clone());
                }
            }
        }
        let scanner = if dfa_rules.is_empty() {
            None
        } else {
            Some(
                scanner_builder
                    .build(&CompileLimits::default())
                    .map_err(|error| RulesError::Dfa(error.to_string()))?,
            )
        };
        Ok(Self {
            packs: packs.to_vec(),
            scanner,
            dfa_rules,
        })
    }

    /// Loads the versioned packs embedded in this release.
    ///
    /// # Errors
    /// Returns a typed error if embedded YAML, AST patterns or DFA patterns fail validation.
    pub fn load_embedded() -> Result<Self, RulesError> {
        let packs = [
            RulePack::parse(include_str!("../packs/core-v1.yaml"))?,
            RulePack::parse(include_str!("../packs/commands-v1.yaml"))?,
        ];
        Self::load(&packs)
    }

    /// Loaded packs in engine order.
    #[must_use]
    pub fn packs(&self) -> &[RulePack] {
        &self.packs
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPack {
    id: String,
    version: u32,
    rules: Vec<RawRule>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    id: String,
    language: Option<String>,
    cwe: Option<u16>,
    severity: String,
    message: String,
    #[serde(default)]
    disputed: bool,
    ast_pattern: Option<String>,
    dfa_pattern: Option<String>,
    dfa_category: Option<String>,
}

fn validate_pack_id(id: &str) -> Result<(), RulesError> {
    if id.is_empty()
        || id.len() > 64
        || !id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        return Err(RulesError::Bound(String::from("invalid pack id")));
    }
    Ok(())
}

fn validate_pattern(id: &RuleId, pattern: &str) -> Result<(), RulesError> {
    if pattern.is_empty() || pattern.len() > MAX_PATTERN_BYTES {
        return Err(RulesError::BadPattern {
            rule: id.as_str().to_owned(),
            reason: format!("pattern must contain 1..={MAX_PATTERN_BYTES} bytes"),
        });
    }
    Ok(())
}

fn parse_language(language: &str) -> Result<Lang, RulesError> {
    match language {
        "rust" => Ok(Lang::Rust),
        "python" => Ok(Lang::Python),
        "cpp" | "c" => Ok(Lang::Cpp),
        "cuda" => Ok(Lang::Cuda),
        "typescript" => Ok(Lang::TypeScript),
        _ => Err(RulesError::UnsupportedLanguage(language.to_owned())),
    }
}

fn parse_category(category: &str) -> Result<FindingCategory, RulesError> {
    match category {
        "secret" => Ok(FindingCategory::Secret),
        "dangerous_shell" => Ok(FindingCategory::DangerousShell),
        _ => Err(RulesError::InvalidRuleKind),
    }
}

fn parse_severity(severity: &str) -> Result<Severity, RulesError> {
    match severity {
        "info" => Ok(Severity::Info),
        "low" => Ok(Severity::Low),
        "medium" => Ok(Severity::Medium),
        "high" => Ok(Severity::High),
        "critical" => Ok(Severity::Critical),
        _ => Err(RulesError::Bound(String::from("unknown severity"))),
    }
}
