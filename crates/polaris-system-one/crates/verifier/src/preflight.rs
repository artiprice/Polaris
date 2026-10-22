//! AST inventory comparison; raw diff line prefixes are never used as test evidence.
use crate::{
    DegradedReason, Finding, GateVerdict, PreflightReport, SafetyRequest, Scanner, SourceLocation,
    TamperKind, Verifier, VerifierError,
};
use polaris_code_intel::{AnalysisCoverage, ByteSpan, MarkerKind, SafetyAnalysis, analyze_safety};
use polaris_sdk::registry::DecisionEngine;
use polaris_system_one_schemas::SchemaCatalog;
use std::collections::{BTreeMap, BTreeSet};

fn location(span: ByteSpan, in_old: bool) -> SourceLocation {
    SourceLocation { span, in_old }
}

pub(crate) fn protected_configuration(target: &str) -> bool {
    let target = target.replace('\\', "/").to_ascii_lowercase();
    target.starts_with(".github/workflows/")
        || target.starts_with(".gitlab/")
        || target.starts_with(".buildkite/")
        || target.starts_with(".circleci/")
        || matches!(
            target.as_str(),
            ".gitlab-ci.yml"
                | "jenkinsfile"
                | "pytest.ini"
                | "tox.ini"
                | ".coveragerc"
                | "rust-toolchain.toml"
                | ".cargo/config.toml"
                | "deny.toml"
        )
        || target.ends_with("/conftest.py")
        || target == "conftest.py"
        || target.ends_with("/cargo.toml")
        || target == "cargo.toml"
        || target.ends_with("/pyproject.toml")
        || target == "pyproject.toml"
        || target.ends_with("/package.json")
        || target == "package.json"
        || target.ends_with("/stop-gate.js")
        || target.ends_with("/selftest.py")
}

fn normalized_assertion(span: ByteSpan, source: &[u8]) -> Option<Vec<u8>> {
    // Exact bytes avoid treating string literals with different whitespace as equivalent.
    // Reformatting can therefore request review, but cannot hide an altered assertion.
    span.slice(source).map(<[u8]>::to_vec)
}

fn inspect_tests(
    old: &SafetyAnalysis,
    new: &SafetyAnalysis,
    request: &SafetyRequest<'_>,
) -> (Vec<Finding>, Option<DegradedReason>) {
    let mut findings = Vec::new();
    let mut degraded = None;
    let current: BTreeMap<_, _> = new
        .tests
        .iter()
        .map(|test| (test.identity.as_str(), test))
        .collect();
    for previous in &old.tests {
        let Some(next) = current.get(previous.identity.as_str()) else {
            findings.push(Finding::TestTampering {
                kind: TamperKind::TestDeleted,
                loc: location(previous.span, true),
            });
            continue;
        };
        if next.assertion_spans.len() < previous.assertion_spans.len() {
            findings.push(Finding::TestTampering {
                kind: TamperKind::AssertionRemoved,
                loc: location(next.span, false),
            });
        } else {
            let prior: BTreeSet<_> = previous
                .assertion_spans
                .iter()
                .filter_map(|span| normalized_assertion(*span, request.old.source()))
                .collect();
            let after: BTreeSet<_> = next
                .assertion_spans
                .iter()
                .filter_map(|span| normalized_assertion(*span, request.new.source()))
                .collect();
            if !prior.is_subset(&after) {
                degraded = Some(DegradedReason::ChangedAssertion);
            }
        }
        for marker in &next.markers {
            let already_present = previous
                .markers
                .iter()
                .any(|prior| prior.kind == marker.kind && prior.evidence == marker.evidence);
            if already_present {
                continue;
            }
            let kind = match marker.kind {
                MarkerKind::IgnoreOrSkip => TamperKind::IgnoreAttributeAdded,
                MarkerKind::CfgDisabled => TamperKind::FeatureGateDisablesTest,
                MarkerKind::UnknownDisableMarker => {
                    degraded = Some(DegradedReason::IncompleteAnalysis);
                    continue;
                }
            };
            findings.push(Finding::TestTampering {
                kind,
                loc: location(marker.span, false),
            });
        }
    }
    let prior_ids: BTreeSet<_> = old
        .tests
        .iter()
        .map(|test| test.identity.as_str())
        .collect();
    for test in &new.tests {
        if prior_ids.contains(test.identity.as_str()) {
            continue;
        }
        for marker in &test.markers {
            let kind = match marker.kind {
                MarkerKind::IgnoreOrSkip => TamperKind::IgnoreAttributeAdded,
                MarkerKind::CfgDisabled => TamperKind::FeatureGateDisablesTest,
                MarkerKind::UnknownDisableMarker => {
                    degraded = Some(DegradedReason::IncompleteAnalysis);
                    continue;
                }
            };
            findings.push(Finding::TestTampering {
                kind,
                loc: location(marker.span, false),
            });
        }
    }
    (findings, degraded)
}

impl<D: DecisionEngine, C: SchemaCatalog, S: Scanner> Verifier<D, C, S> {
    /// Compare actual AST inventories, scan secrets, and obtain calibrated residual decisions.
    ///
    /// # Errors
    /// Returns processing, parser, scanner or schema errors. Errors never mean approval.
    pub async fn preflight(
        &self,
        request: &SafetyRequest<'_>,
    ) -> Result<PreflightReport, VerifierError> {
        self.bound(request.old.source())?;
        self.bound(request.new.source())?;
        self.bound(request.intent.as_bytes())?;
        if request.old.lang() != request.new.lang() {
            return Err(VerifierError::InvalidInput {
                reason: "source snapshots use different languages",
            });
        }
        let old = analyze_safety(request.old)?;
        let new = analyze_safety(request.new)?;
        let (mut findings, mut degraded) = inspect_tests(&old, &new, request);
        findings.extend(crate::scan::secret_findings(
            &self.scanner,
            request.new.source(),
            self.config.max_scan_matches,
            self.config.entropy,
        )?);
        if !findings.is_empty() {
            return Ok(PreflightReport {
                verdict: GateVerdict::Block {
                    findings: findings.clone(),
                },
                findings,
                s1_used: false,
                degraded: None,
            });
        }
        if old.syntax_error || new.syntax_error {
            degraded = Some(DegradedReason::SyntaxError);
        }
        if old.test_coverage != AnalysisCoverage::Complete
            || new.test_coverage != AnalysisCoverage::Complete
            || old.assertion_coverage != AnalysisCoverage::Complete
            || new.assertion_coverage != AnalysisCoverage::Complete
        {
            degraded = Some(DegradedReason::IncompleteAnalysis);
        }
        if protected_configuration(request.target) && request.old.source() != request.new.source() {
            degraded = Some(DegradedReason::ProtectedConfiguration);
        }
        if let Some(reason) = degraded {
            return Ok(PreflightReport {
                verdict: GateVerdict::AskUser { reason },
                findings,
                s1_used: false,
                degraded: Some(reason),
            });
        }
        let input = crate::semantic::mutation_context(request)?;
        let mut s1_used = false;
        for id in ["safety.diff_tamper", "safety.secret_semantic"] {
            let verdict = self
                .semantic_tracked(id, input.as_bytes(), None, &mut s1_used)
                .await?;
            match verdict {
                GateVerdict::Allow => {}
                GateVerdict::Block {
                    findings: semantic_findings,
                } => {
                    findings.extend(semantic_findings);
                    return Ok(PreflightReport {
                        verdict: GateVerdict::Block {
                            findings: findings.clone(),
                        },
                        findings,
                        s1_used,
                        degraded: None,
                    });
                }
                GateVerdict::AskUser { reason } => {
                    return Ok(PreflightReport {
                        verdict: GateVerdict::AskUser { reason },
                        findings,
                        s1_used,
                        degraded: Some(reason),
                    });
                }
            }
        }
        Ok(PreflightReport {
            verdict: GateVerdict::Allow,
            findings,
            s1_used,
            degraded: None,
        })
    }
}
