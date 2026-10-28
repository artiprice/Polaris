//! Bounded AST-diff auditing and content-addressed redacted findings.

use crate::error::RulesError;
use crate::model::{Finding, FindingCategory};
use crate::pack::{Rule, RuleEngine, RuleKind};
use polaris_code_intel::{ByteSpan, Lang, ParseLimits, ParserPool};
use polaris_sdk::ids::Cid;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

const MAX_SOURCE_BYTES: usize = 2 * 1024 * 1024;
const MAX_HUNKS: usize = 4096;
const MAX_AST_MATCHES_PER_RULE: usize = 512;
const MAX_AST_NODES: usize = 100_000;
const MAX_TOTAL_FINDINGS: usize = 4096;
const PARSE_TIMEOUT: Duration = Duration::from_millis(500);

/// Request to audit one new file revision against caller-computed changed byte spans.
#[derive(Debug, Clone, Copy)]
pub struct DiffAuditRequest<'a> {
    path: &'a Path,
    old_source: &'a [u8],
    new_source: &'a [u8],
    changed_hunks: &'a [ByteSpan],
}

impl<'a> DiffAuditRequest<'a> {
    /// Constructs a request; validation is performed by [`RuleEngine::audit_diff`].
    #[must_use]
    pub const fn new(
        path: &'a Path,
        old_source: &'a [u8],
        new_source: &'a [u8],
        changed_hunks: &'a [ByteSpan],
    ) -> Self {
        Self {
            path,
            old_source,
            new_source,
            changed_hunks,
        }
    }
}

/// Whether the source language was scanned by configured AST rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageCoverage {
    /// The finite configured rule set was evaluated; this does not claim language completeness.
    Partial(Lang),
    /// The parser understands this language, but the loaded packs have no AST rules for it.
    Incomplete(Lang),
    /// The path has no supported parser grammar.
    Unsupported,
}

/// Findings and explicit grammar/rule coverage for one changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleScanReport {
    findings: Vec<Finding>,
    coverage: LanguageCoverage,
}

impl RuleScanReport {
    /// Findings whose match span intersects a changed hunk.
    #[must_use]
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    /// AST-language coverage for this request.
    #[must_use]
    pub const fn coverage(&self) -> LanguageCoverage {
        self.coverage
    }
}

impl RuleEngine {
    /// Audits only caller-supplied changed byte ranges in a bounded parsed new revision.
    ///
    /// Source bytes and raw matched snippets are not retained in returned findings. The caller
    /// must derive `changed_hunks` from its authoritative diff; every range is validated here.
    ///
    /// # Errors
    /// Returns a typed error when request bounds, paths, parsing, AST matching or DFA scanning
    /// cannot be completed safely.
    pub fn audit_diff(&self, request: &DiffAuditRequest<'_>) -> Result<RuleScanReport, RulesError> {
        let path = validate_request(request)?;
        let head_end = request.new_source.len().min(256);
        let head = request
            .new_source
            .get(..head_end)
            .map_or(request.new_source, |bytes| bytes);
        let Some(lang) = polaris_code_intel::detect_lang(&path, head) else {
            return Ok(RuleScanReport {
                findings: Vec::new(),
                coverage: LanguageCoverage::Unsupported,
            });
        };
        if request.old_source == request.new_source || request.changed_hunks.is_empty() {
            return Ok(RuleScanReport {
                findings: Vec::new(),
                coverage: coverage_for(self, lang),
            });
        }
        let parser = ParserPool::with_limits(ParseLimits {
            max_source_bytes: MAX_SOURCE_BYTES,
            timeout: PARSE_TIMEOUT,
        });
        let parsed = parser
            .parse(lang, request.new_source, None)
            .map_err(|error| RulesError::Ast(error.to_string()))?;
        if parsed.has_syntax_error() {
            return Err(RulesError::Ast(String::from(
                "new source contains syntax errors; refusing partial AST analysis",
            )));
        }
        if parsed.node_count() > MAX_AST_NODES {
            return Err(RulesError::Bound(format!(
                "AST node count exceeds {MAX_AST_NODES}"
            )));
        }
        let mut findings = Vec::new();
        for pack in &self.packs {
            for rule in pack.rules() {
                let RuleKind::AstPattern {
                    lang: rule_lang,
                    pattern,
                } = rule.kind()
                else {
                    continue;
                };
                if *rule_lang != lang {
                    continue;
                }
                let matches = parsed
                    .find_ast_pattern(pattern, MAX_AST_MATCHES_PER_RULE, MAX_AST_NODES)
                    .map_err(|error| RulesError::Ast(error.to_string()))?;
                for span in matches {
                    if !request
                        .changed_hunks
                        .iter()
                        .any(|hunk| intersects(*hunk, span))
                    {
                        continue;
                    }
                    push_finding(
                        &mut findings,
                        rule,
                        &path,
                        span,
                        request.new_source,
                        FindingCategory::Ast,
                    )?;
                    if findings.len() > MAX_TOTAL_FINDINGS {
                        return Err(RulesError::Bound(format!(
                            "finding count exceeds {MAX_TOTAL_FINDINGS}"
                        )));
                    }
                }
            }
        }
        self.scan_changed_secrets(
            request.new_source,
            request.changed_hunks,
            &path,
            &mut findings,
        )?;
        Ok(RuleScanReport {
            findings,
            coverage: coverage_for(self, lang),
        })
    }

    fn scan_changed_secrets(
        &self,
        source: &[u8],
        hunks: &[ByteSpan],
        path: &Path,
        findings: &mut Vec<Finding>,
    ) -> Result<(), RulesError> {
        let Some(scanner) = &self.scanner else {
            return Ok(());
        };
        if std::str::from_utf8(source).is_err() {
            return Ok(());
        }
        for hunk in hunks {
            let Some(changed) = hunk.text(source) else {
                return Err(RulesError::Bound(String::from("changed hunk is not UTF-8")));
            };
            let mut buffer = polaris_security_automata::ScanBuf::with_capacity(256);
            scanner
                .scan(changed.as_bytes(), &mut buffer)
                .map_err(|error| RulesError::Dfa(error.to_string()))?;
            for matched in buffer.matches() {
                let category = match scanner.rule_category(matched.rule) {
                    Some(polaris_security_automata::Category::Secret) => FindingCategory::Secret,
                    Some(polaris_security_automata::Category::DangerousShell) => {
                        FindingCategory::DangerousShell
                    }
                    Some(polaris_security_automata::Category::Pii) | None => continue,
                };
                let name = scanner.rule_name(matched.rule).ok_or_else(|| {
                    RulesError::Dfa(String::from("scanner returned unknown rule index"))
                })?;
                let rule = self
                    .dfa_rules
                    .iter()
                    .find(|candidate| candidate.id().as_str() == name)
                    .ok_or_else(|| {
                        RulesError::Dfa(String::from("scanner rule metadata missing"))
                    })?;
                let relative_start = usize::try_from(matched.start)
                    .map_err(|_| RulesError::Bound(String::from("match offset overflow")))?;
                let relative_end = usize::try_from(matched.end)
                    .map_err(|_| RulesError::Bound(String::from("match offset overflow")))?;
                let absolute_start = hunk
                    .start
                    .checked_add(relative_start)
                    .ok_or_else(|| RulesError::Bound(String::from("match offset overflow")))?;
                let absolute_end = hunk
                    .start
                    .checked_add(relative_end)
                    .ok_or_else(|| RulesError::Bound(String::from("match offset overflow")))?;
                if absolute_end > source.len() || absolute_start > absolute_end {
                    return Err(RulesError::Bound(String::from(
                        "scanner returned invalid offsets",
                    )));
                }
                push_finding(
                    findings,
                    rule,
                    path,
                    ByteSpan::new(absolute_start, absolute_end),
                    source,
                    category,
                )?;
                if findings.len() > MAX_TOTAL_FINDINGS {
                    return Err(RulesError::Bound(format!(
                        "finding count exceeds {MAX_TOTAL_FINDINGS}"
                    )));
                }
            }
        }
        Ok(())
    }
}

fn validate_request(request: &DiffAuditRequest<'_>) -> Result<PathBuf, RulesError> {
    let path = normalize_relative_path(request.path)?;
    if request.old_source.len() > MAX_SOURCE_BYTES || request.new_source.len() > MAX_SOURCE_BYTES {
        return Err(RulesError::Bound(format!(
            "source exceeds {MAX_SOURCE_BYTES} bytes"
        )));
    }
    if request.changed_hunks.len() > MAX_HUNKS {
        return Err(RulesError::Bound(format!("hunk count exceeds {MAX_HUNKS}")));
    }
    for span in request.changed_hunks {
        if span.start > span.end || span.end > request.new_source.len() {
            return Err(RulesError::Bound(String::from(
                "changed hunk lies outside new source",
            )));
        }
    }
    Ok(path)
}

fn coverage_for(engine: &RuleEngine, lang: Lang) -> LanguageCoverage {
    if engine
        .packs
        .iter()
        .flat_map(crate::pack::RulePack::rules)
        .any(|rule| matches!(rule.kind(), RuleKind::AstPattern { lang: rule_lang, .. } if *rule_lang == lang))
    {
        LanguageCoverage::Partial(lang)
    } else {
        LanguageCoverage::Incomplete(lang)
    }
}

const fn intersects(left: ByteSpan, right: ByteSpan) -> bool {
    if left.is_empty() {
        right.start <= left.start && left.end <= right.end
    } else if right.is_empty() {
        left.start <= right.start && right.end <= left.end
    } else {
        left.start < right.end && right.start < left.end
    }
}

fn normalize_relative_path(path: &Path) -> Result<PathBuf, RulesError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(RulesError::Bound(String::from(
            "path must be a non-empty relative path",
        )));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(RulesError::Bound(String::from(
                    "path traversal or platform prefix is forbidden",
                )));
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(RulesError::Bound(String::from(
            "path has no normal components",
        )));
    }
    Ok(normalized)
}

pub fn push_finding(
    findings: &mut Vec<Finding>,
    rule: &Rule,
    path: &Path,
    span: ByteSpan,
    bytes: &[u8],
    category: FindingCategory,
) -> Result<(), RulesError> {
    let matched = span
        .slice(bytes)
        .ok_or_else(|| RulesError::Bound(String::from("finding span lies outside source")))?;
    findings.push(Finding {
        rule: rule.id().clone(),
        cwe: rule.cwe(),
        severity: rule.severity(),
        path: path.to_path_buf(),
        span,
        snippet_digest: Cid::from_bytes(*blake3::hash(matched).as_bytes()),
        category,
        message: rule.message().to_owned(),
        disputed: rule.disputed(),
    });
    Ok(())
}
