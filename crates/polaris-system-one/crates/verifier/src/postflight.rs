//! A passing process is evidence of execution, not sufficient proof of meaningful tests.
use polaris_code_intel::{AnalysisCoverage, ByteSpan, analyze_safety};
use polaris_sdk::{ids::Cid, registry::DecisionEngine};
use polaris_system_one_schemas::SchemaCatalog;
use crate::{DegradedReason, Diagnostic, Finding, GateVerdict, PostflightReport,
    SafetyRequest, Scanner, SourceLocation, Verifier, VerifierError, rank_diagnostics};

/// Proof status deliberately distinct from a shell exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofVerdict {
    /// Execution, test quality and an independent falsifier all established the scoped claim.
    Pass,
    /// A test failed or deterministic test-quality rules rejected the result.
    Fail,
    /// Required independent evidence is missing or inconclusive.
    Unverified,
}

/// Mutation-run evidence from a trusted runner. This is data, not a caller-mintable witness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationEvidence {
    /// Exact workspace snapshot used by the independent runner.
    pub workspace_snapshot: Cid,
    /// Test selection/artifact identity used by the original green run.
    pub test_artifact: Cid,
    /// Mutant results stored in CAS by the runner owner.
    pub report: Cid,
    /// Number of selected scoped mutants before execution.
    pub planned: u32,
    /// Number of mutants completed with a conclusive result.
    pub completed: u32,
    /// Number of mutants killed by the selected tests.
    pub killed: u32,
    /// Mutants that survived.
    pub survived: u32,
    /// Invalid, timed-out or otherwise inconclusive mutants.
    pub inconclusive: u32,
}

/// Independent counterexample attempt against the exact claimed behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalsifierEvidence {
    /// Exact workspace snapshot that was evaluated.
    pub workspace_snapshot: Cid,
    /// Same test artifact as the original run.
    pub test_artifact: Cid,
    /// Evidence report in CAS, checked by the runner owner.
    pub report: Cid,
    /// Declared independent negative-control cases.
    pub planned_cases: u32,
    /// Cases actually executed to completion.
    pub completed_cases: u32,
    /// Negative controls rejected by the claimed checks.
    pub rejected_controls: u32,
    /// Whether any counterexample invalidated the claimed result.
    pub counterexample_found: bool,
}

/// Required independent evidence, never replaced by a model opinion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofEvidence {
    /// No independent evaluation was performed.
    None,
    /// Scoped mutation runner evidence.
    Mutation(MutationEvidence),
    /// Scoped falsification/negative-control evidence.
    Falsifier(FalsifierEvidence),
}

/// Execution facts supplied by the trusted execution and CAS owners, not by model text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestRunFacts {
    /// Immutable workspace revision executed by the runner.
    pub workspace_snapshot: Cid,
    /// Content id of the exact resulting source under review.
    pub source: Cid,
    /// Identity of the compiled or selected test suite.
    pub test_artifact: Cid,
    /// Stored process result and structured runner report.
    pub execution_report: Cid,
    /// Reported final process exit code; absent means it did not complete normally.
    pub exit_code: Option<i32>,
    /// Tests executed to completion.
    pub completed_tests: u32,
    /// Failed tests.
    pub failed_tests: u32,
    /// Skipped or ignored tests in the scoped selection.
    pub skipped_tests: u32,
    /// Incomplete/inconclusive tests.
    pub inconclusive_tests: u32,
    /// Independently collected proof evidence.
    pub proof: ProofEvidence,
    /// Structured, sanitized diagnostics from the execution owner.
    pub diagnostics: Vec<Diagnostic>,
}

fn independent_proof(run: &TestRunFacts) -> ProofVerdict {
    match &run.proof {
        ProofEvidence::None => ProofVerdict::Unverified,
        ProofEvidence::Mutation(evidence) => {
            if evidence.workspace_snapshot != run.workspace_snapshot || evidence.test_artifact != run.test_artifact
                || evidence.planned == 0 || evidence.completed != evidence.planned || evidence.inconclusive != 0
                || evidence.killed.checked_add(evidence.survived) != Some(evidence.completed)
            { return ProofVerdict::Unverified; }
            if evidence.survived != 0 { ProofVerdict::Fail } else { ProofVerdict::Pass }
        },
        ProofEvidence::Falsifier(evidence) => {
            if evidence.workspace_snapshot != run.workspace_snapshot || evidence.test_artifact != run.test_artifact
                || evidence.planned_cases == 0 || evidence.completed_cases != evidence.planned_cases
                || evidence.rejected_controls > evidence.completed_cases
            { return ProofVerdict::Unverified; }
            if evidence.counterexample_found { return ProofVerdict::Fail; }
            if evidence.rejected_controls != evidence.planned_cases { ProofVerdict::Unverified } else { ProofVerdict::Pass }
        },
    }
}

impl<D: DecisionEngine, C: SchemaCatalog, S: Scanner> Verifier<D, C, S> {
    /// Assess a completed run. CAS references must be resolved and verified by the execution
    /// owner before these structured facts are supplied; this crate does not invent runner data.
    ///
    /// # Errors
    /// Returns bounded-input or AST analysis errors. No error can create a proof pass.
    pub async fn postflight(&self, run: &TestRunFacts, request: &SafetyRequest<'_>)
        -> Result<PostflightReport, VerifierError>
    {
        self.bound(request.new.source())?;
        if run.diagnostics.len() > 8192 {
            return Err(VerifierError::InvalidInput { reason: "too many structured diagnostics" });
        }
        let analysis = analyze_safety(request.new)?;
        let mut findings: Vec<_> = analysis.tautologies.iter().map(|tautology| Finding::Tautology {
            kind: tautology.kind,
            loc: SourceLocation { span: tautology.span, in_old: false },
        }).collect();
        let mut changed_ranges = Vec::new();
        for test in &analysis.tests {
            changed_ranges.push(test.span);
            if test.assertion_spans.is_empty() {
                findings.push(Finding::NoAssertions { loc: SourceLocation { span: test.span, in_old: false } });
            }
        }
        if changed_ranges.is_empty() { changed_ranges.push(ByteSpan::new(0, request.new.source().len())); }
        let ranked_diagnostics = rank_diagnostics(&run.diagnostics, &changed_ranges, self.config.diagnostic_limit);
        let report = |verdict, findings, degraded| PostflightReport { verdict, findings,
            ranked_diagnostics: ranked_diagnostics.clone(), degraded };
        if run.failed_tests != 0 || run.exit_code.is_some_and(|code| code != 0)
            || findings.iter().any(|finding| matches!(finding, Finding::Tautology { .. }))
        { return Ok(report(ProofVerdict::Fail, findings, None)); }
        let source_cid = Cid::from_bytes(*blake3::hash(request.new.source()).as_bytes());
        if run.source != source_cid || run.exit_code != Some(0) || run.completed_tests == 0
            || run.skipped_tests != 0 || run.inconclusive_tests != 0
        { return Ok(report(ProofVerdict::Unverified, findings, Some(DegradedReason::IncompleteProof))); }
        if analysis.syntax_error || analysis.test_coverage != AnalysisCoverage::Complete
            || analysis.assertion_coverage != AnalysisCoverage::Complete || !findings.is_empty()
        { return Ok(report(ProofVerdict::Unverified, findings, Some(DegradedReason::IncompleteAnalysis))); }
        let proof = independent_proof(run);
        if proof != ProofVerdict::Pass {
            return Ok(report(proof, findings, Some(DegradedReason::IncompleteProof)));
        }
        let input = crate::semantic::mutation_context(request)?;
        match self.semantic("safety.tautology", input.as_bytes(), None).await? {
            GateVerdict::Allow => Ok(report(ProofVerdict::Pass, findings, None)),
            GateVerdict::Block { findings: semantic_findings } => {
                findings.extend(semantic_findings);
                Ok(report(ProofVerdict::Fail, findings, None))
            },
            GateVerdict::AskUser { reason } => Ok(report(ProofVerdict::Unverified, findings, Some(reason))),
        }
    }
}
