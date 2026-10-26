//! Ordered hybrid verification pipeline.

use crate::{EvidenceError, EvidenceStore, mutation_digest};
use polaris_code_intel::{AnalysisCoverage, MutationMetrics, ParserPool, analyze_safety, metrics};
use polaris_sdk::{
    contracts::{
        ApprovalWitness, ApproverId, Command, CommandBody, Timestamp, Unverified, Verified,
    },
    ids::{Cid, CommandId, FrameId},
};
use polaris_security_capability::{
    Authorization, CapabilityError, Clock, Facts, PublicKeyBytes, ResourcePath, Right, Token,
    authorize,
};
use polaris_system_one_verifier::{
    DegradedReason, MutationSafety, SafetyAssessment, SafetyRequest,
};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path, time::Duration};

/// Maximum encoded capability size accepted before signature parsing.
pub const MAX_TOKEN_BYTES: usize = 65_536;
/// Maximum workspace-relative target string length.
pub const MAX_TARGET_BYTES: usize = 4_096;
/// Maximum source snapshot size per version.
pub const MAX_SOURCE_BYTES: usize = 4_194_304;
/// Maximum unified-diff byte size.
pub const MAX_DIFF_BYTES: usize = 4_194_304;
/// Maximum intent text size.
pub const MAX_INTENT_BYTES: usize = 16_384;
/// Maximum serialized approval evidence size.
pub const MAX_EVIDENCE_BYTES: usize = 1_048_576;

/// Deterministic limits applied after signature-chain verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifierConfig {
    /// Maximum observed line additions plus removals.
    pub max_churn_lines: u32,
    /// Maximum positive cyclomatic-complexity delta.
    pub max_cyclomatic_increase: u32,
    /// Maximum exact tree-edit distance.
    pub max_tree_edit_distance: u32,
    /// Maximum duration for the S1 safety and intent assessment.
    pub safety_timeout: Duration,
}

impl Default for VerifierConfig {
    fn default() -> Self {
        Self {
            max_churn_lines: 150,
            max_cyclomatic_increase: 20,
            max_tree_edit_distance: 2_000,
            safety_timeout: Duration::from_secs(5),
        }
    }
}

/// A mutation target and its trusted immutable source snapshots.
///
/// The caller must have resolved the target inside the active workspace, checked canonical
/// containment, and obtained `old_source` and `new_source` from that same resolved target. The
/// caller must keep the base snapshot protected by an immutable snapshot or lease through the
/// eventual compare-and-swap write. The verifier records hashes but cannot enforce filesystem
/// containment or prevent an external writer from changing the target after this call.
#[derive(Debug, Clone, Copy)]
pub struct MutationRequest<'a> {
    /// Unverified command carrying the bearer capability and target diff CID.
    pub command: &'a Command<Unverified>,
    /// Workspace-relative path supplied by the trusted workspace resolver.
    pub target: &'a str,
    /// Exact original source bytes from the protected base snapshot.
    pub old_source: &'a [u8],
    /// Exact resulting source bytes proposed by the command.
    pub new_source: &'a [u8],
    /// Exact unified-diff bytes referenced by `CommandBody::ProposeDiff`.
    pub diff: &'a [u8],
    /// User or parent-frame intent the mutation is meant to serve.
    pub intent: &'a str,
}

/// Which input exceeded a byte cap before hashing or parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputField {
    /// Encoded capability token.
    Capability,
    /// Workspace-relative target.
    Target,
    /// Original source snapshot.
    OldSource,
    /// Proposed source snapshot.
    NewSource,
    /// Unified diff.
    Diff,
    /// Mutation intent.
    Intent,
}

/// Which deterministic metric limit rejected an authenticated mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    /// Total added plus removed lines.
    ChurnLines,
    /// Positive cyclomatic-complexity delta.
    CyclomaticIncrease,
    /// Exact tree-edit distance.
    TreeEditDistance,
}

/// Why deterministic verification rejected a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// The command was not a proposed diff or its embedded diff CID did not match the bytes.
    CommandBinding,
    /// Capability parsing, signature, scope, TTL, or Datalog authorization denied the request.
    Authorization,
    /// A measured metric exceeded its configured bound.
    Metric(Metric),
    /// An input exceeded a byte cap before hashing or parsing.
    InputLimit { field: InputField, max_bytes: usize },
    /// S1 found a safety issue in the proposed mutation.
    Safety,
}

/// Outcome of a hybrid mutation verification.
#[derive(Debug, PartialEq, Eq)]
pub enum VerificationDecision {
    /// The command has been promoted only after a durable, digest-checked evidence record exists.
    /// The cold approval payload is boxed so rejection and escalation do not carry its full size.
    Approved(Box<ApprovedMutation>),
    /// A deterministic layer rejected the request.
    Rejected(Rejection),
    /// The safety assessor degraded or timed out; a human decision is required.
    NeedsApproval { reason: ApprovalReason },
}

/// Why an otherwise valid mutation needs a human decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalReason {
    /// The supported-language analyzer cannot prove complete unsafe-change coverage.
    UnsafeAnalysisCoverage,
    /// The semantic safety assessor requested human review.
    SafetyDegraded(DegradedReason),
    /// S1 did not finish within the configured timeout.
    SafetyTimeout,
    /// The tree-edit distance was a lower bound and did not exceed the limit, so compliance could
    /// not be proven.
    TreeEditDistanceLowerBound { lower_bound: u32, limit: u32 },
}

/// Errors from trusted parser, safety, or evidence infrastructure. Callers must fail closed.
#[derive(Debug, thiserror::Error)]
pub enum VerifierError {
    /// A mutation field length could not be encoded safely.
    #[error(transparent)]
    Binding(#[from] crate::MutationBindingError),
    /// The code-intelligence parser or metric engine failed.
    #[error("mutation analysis failed")]
    CodeIntel(#[from] polaris_code_intel::CodeIntelError),
    /// Capability authorization infrastructure failed.
    #[error("capability authorization failed")]
    Capability(#[from] CapabilityError),
    /// Safety analysis infrastructure failed.
    #[error("semantic safety analysis failed")]
    Safety(#[from] polaris_system_one_verifier::VerifierError),
    /// The evidence store failed or its read-back was missing or corrupt.
    #[error("approval evidence could not be durably verified")]
    Evidence(#[from] EvidenceError),
    /// A valid verifier identifier could not be constructed.
    #[error("verifier identity is invalid")]
    Approver(#[from] polaris_sdk::error::ParseError),
    /// The report could not be encoded deterministically.
    #[error("approval evidence encoding failed")]
    EvidenceEncoding,
    /// The report exceeded its bounded record size.
    #[error("approval evidence exceeds the configured byte bound")]
    EvidenceTooLarge,
}

/// Opaque approval result; its fields cannot be forged outside this crate.
#[derive(Debug, PartialEq, Eq)]
pub struct ApprovedMutation {
    command: Command<Verified>,
    command_id: CommandId,
    agent: FrameId,
    target: String,
    diff: Cid,
    old_source: Cid,
    new_source: Cid,
    metrics: MutationMetrics,
    evidence: Cid,
}

impl ApprovedMutation {
    /// The command promoted to the SDK's verified typestate.
    #[must_use]
    pub const fn command(&self) -> &Command<Verified> {
        &self.command
    }

    /// Command identity included in the durable report.
    #[must_use]
    pub const fn command_id(&self) -> CommandId {
        self.command_id
    }

    /// Agent frame identity included in capability facts and the durable report.
    #[must_use]
    pub const fn agent(&self) -> FrameId {
        self.agent
    }

    /// Canonical lexical target path authorized by the capability.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// BLAKE3 CID of the exact unified-diff bytes.
    #[must_use]
    pub const fn diff(&self) -> Cid {
        self.diff
    }

    /// BLAKE3 CID of the original source snapshot.
    #[must_use]
    pub const fn old_source(&self) -> Cid {
        self.old_source
    }

    /// BLAKE3 CID of the proposed source snapshot.
    #[must_use]
    pub const fn new_source(&self) -> Cid {
        self.new_source
    }

    /// Exact AST metrics used for the approval decision.
    #[must_use]
    pub const fn metrics(&self) -> &MutationMetrics {
        &self.metrics
    }

    /// Verified CAS record proving this approval decision.
    #[must_use]
    pub const fn evidence(&self) -> Cid {
        self.evidence
    }
}

/// Production hybrid mutation verifier.
#[derive(Debug)]
pub struct HybridMutationVerifier<S, C, E> {
    root: PublicKeyBytes,
    config: VerifierConfig,
    safety: S,
    clock: C,
    evidence: E,
    parser: ParserPool,
}

impl<S, C, E> HybridMutationVerifier<S, C, E>
where
    S: MutationSafety,
    C: Clock,
    E: EvidenceStore,
{
    /// Construct a verifier with its root issuer key, semantic safety boundary, clock and CAS.
    #[must_use]
    pub fn new(
        root: PublicKeyBytes,
        config: VerifierConfig,
        safety: S,
        clock: C,
        evidence: E,
    ) -> Self {
        Self {
            root,
            config,
            safety,
            clock,
            evidence,
            parser: ParserPool::new(),
        }
    }

    /// Verify a proposed mutation in signature, exact-metric, Datalog and S1 order.
    ///
    /// The signature chain is parsed before source analysis. Datalog authorization then consumes
    /// the observed facts from the exact parsed snapshots. An inexact tree-distance lower bound
    /// above the limit is a deterministic rejection; a lower bound at or below the limit cannot
    /// prove safety and requires human review. S1 failures fail closed, and no approval witness is
    /// created until the evidence bytes have been written and read back under their BLAKE3 CID.
    ///
    /// # Errors
    /// Returns an infrastructure error for parser, authorization, safety, or CAS failures. These
    /// errors must be treated as rejection by the command dispatcher.
    pub async fn verify(
        &self,
        request: MutationRequest<'_>,
    ) -> Result<VerificationDecision, VerifierError> {
        let input_caps = [
            (
                InputField::Capability,
                request.command.token().as_bytes().len(),
                MAX_TOKEN_BYTES,
            ),
            (InputField::Target, request.target.len(), MAX_TARGET_BYTES),
            (
                InputField::OldSource,
                request.old_source.len(),
                MAX_SOURCE_BYTES,
            ),
            (
                InputField::NewSource,
                request.new_source.len(),
                MAX_SOURCE_BYTES,
            ),
            (InputField::Diff, request.diff.len(), MAX_DIFF_BYTES),
            (InputField::Intent, request.intent.len(), MAX_INTENT_BYTES),
        ];
        if let Some((field, _observed, max_bytes)) = input_caps
            .iter()
            .find(|(_, observed, limit)| observed > limit)
        {
            return Ok(VerificationDecision::Rejected(Rejection::InputLimit {
                field: *field,
                max_bytes: *max_bytes,
            }));
        }
        let expected_diff = cid_for(request.diff);
        let (command_id, agent, token_bytes) = match request.command.body() {
            CommandBody::ProposeDiff { diff } if *diff == expected_diff => (
                request.command.id(),
                request.command.frame(),
                request.command.token().as_bytes(),
            ),
            CommandBody::ProposeDiff { .. } => {
                return Ok(VerificationDecision::Rejected(Rejection::CommandBinding));
            }
            _ => return Ok(VerificationDecision::Rejected(Rejection::CommandBinding)),
        };

        // Token::from_bytes verifies the Biscuit chain before analysis or a model call.
        let token = match Token::from_bytes(token_bytes, &self.root) {
            Ok(token) => token,
            Err(_) => return Ok(VerificationDecision::Rejected(Rejection::Authorization)),
        };
        let resource = match ResourcePath::new(request.target) {
            Ok(resource) => resource,
            Err(_) => return Ok(VerificationDecision::Rejected(Rejection::Authorization)),
        };
        let old = self
            .parser
            .parse_file(Path::new(request.target), request.old_source)?;
        let new = self
            .parser
            .parse_file(Path::new(request.target), request.new_source)?;
        if old.has_syntax_error() || new.has_syntax_error() {
            return Ok(VerificationDecision::Rejected(Rejection::Metric(
                Metric::TreeEditDistance,
            )));
        }
        let measured = metrics(&old, &new)?;
        let old_safety = analyze_safety(&old)?;
        let new_safety = analyze_safety(&new)?;
        if old_safety.unsafe_coverage != AnalysisCoverage::Complete
            || new_safety.unsafe_coverage != AnalysisCoverage::Complete
        {
            return Ok(VerificationDecision::NeedsApproval {
                reason: ApprovalReason::UnsafeAnalysisCoverage,
            });
        }
        let touches_unsafe =
            unsafe_observations(&old, &old_safety) != unsafe_observations(&new, &new_safety);
        let diff = expected_diff;
        let old_source = cid_for(request.old_source);
        let new_source = cid_for(request.new_source);
        let mutation_binding = mutation_digest(
            command_id,
            agent,
            resource.as_str(),
            diff,
            old_source,
            new_source,
        )?;
        let facts = Facts {
            resource: &resource,
            symbol: single_symbol(&measured),
            churn_lines: measured.churn_lines,
            touches_unsafe,
            diff_blake3: Some(mutation_binding),
            right: Right::ProposeDiff,
        };
        match authorize(&token, &facts, &self.root, &self.clock)? {
            Authorization::Allowed => {}
            Authorization::Denied { .. } => {
                return Ok(VerificationDecision::Rejected(Rejection::Authorization));
            }
        }
        if measured.churn_lines > self.config.max_churn_lines {
            return Ok(VerificationDecision::Rejected(Rejection::Metric(
                Metric::ChurnLines,
            )));
        }
        let cyclomatic_increase = measured.cyclomatic_delta.max(0).unsigned_abs();
        if cyclomatic_increase > self.config.max_cyclomatic_increase {
            return Ok(VerificationDecision::Rejected(Rejection::Metric(
                Metric::CyclomaticIncrease,
            )));
        }
        if measured.tree_edit_distance > self.config.max_tree_edit_distance {
            return Ok(VerificationDecision::Rejected(Rejection::Metric(
                Metric::TreeEditDistance,
            )));
        }
        if !measured.tree_edit_distance_exact {
            return Ok(VerificationDecision::NeedsApproval {
                reason: ApprovalReason::TreeEditDistanceLowerBound {
                    lower_bound: measured.tree_edit_distance,
                    limit: self.config.max_tree_edit_distance,
                },
            });
        }

        let assessment = match tokio::time::timeout(
            self.config.safety_timeout,
            self.safety.assess(&SafetyRequest {
                target: resource.as_str(),
                old: &old,
                new: &new,
                intent: request.intent,
                changed_symbols: &measured.changed_symbols,
            }),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                return Ok(VerificationDecision::NeedsApproval {
                    reason: ApprovalReason::SafetyTimeout,
                });
            }
        };
        match assessment {
            SafetyAssessment::Allow => {}
            SafetyAssessment::Block { .. } => {
                return Ok(VerificationDecision::Rejected(Rejection::Safety));
            }
            SafetyAssessment::NeedsApproval { reason } => {
                return Ok(VerificationDecision::NeedsApproval {
                    reason: ApprovalReason::SafetyDegraded(reason),
                });
            }
        }

        // S1 may take long enough for a short-lived capability to expire. Re-evaluate the exact
        // same observed facts immediately before persisting evidence.
        match authorize(&token, &facts, &self.root, &self.clock)? {
            Authorization::Allowed => {}
            Authorization::Denied { .. } => {
                return Ok(VerificationDecision::Rejected(Rejection::Authorization));
            }
        }

        let report = ApprovalEvidence {
            schema: 1,
            command_id,
            agent,
            target: resource.as_str(),
            diff,
            old_source,
            new_source,
            mutation_binding,
            intent_blake3: cid_for(request.intent.as_bytes()),
            churn_lines: measured.churn_lines,
            max_churn_lines: self.config.max_churn_lines,
            cyclomatic_delta: measured.cyclomatic_delta,
            max_cyclomatic_increase: self.config.max_cyclomatic_increase,
            tree_edit_distance: measured.tree_edit_distance,
            tree_edit_distance_exact: measured.tree_edit_distance_exact,
            max_tree_edit_distance: self.config.max_tree_edit_distance,
            touches_unsafe,
            safety_result: EvidenceSafetyResult::PreflightAndCongruenceAllowed,
            changed_symbols: measured
                .changed_symbols
                .iter()
                .map(|symbol| symbol.name.clone())
                .collect(),
            checked_at: self.clock.now(),
        };
        let symbol_bytes = report
            .changed_symbols
            .iter()
            .fold(0_usize, |size, symbol| size.saturating_add(symbol.len()));
        if symbol_bytes > MAX_EVIDENCE_BYTES {
            return Err(VerifierError::EvidenceTooLarge);
        }
        let evidence_bytes =
            serde_json::to_vec(&report).map_err(|_| VerifierError::EvidenceEncoding)?;
        if evidence_bytes.len() > MAX_EVIDENCE_BYTES {
            return Err(VerifierError::EvidenceTooLarge);
        }
        let evidence_cid = self.evidence.put(&evidence_bytes)?;
        if evidence_cid != cid_for(&evidence_bytes) {
            return Err(EvidenceError::DigestMismatch.into());
        }
        match self.evidence.get(&evidence_cid)? {
            Some(stored) if stored == evidence_bytes && cid_for(&stored) == evidence_cid => {}
            _ => return Err(EvidenceError::DigestMismatch.into()),
        }
        // CAS latency is another expiry boundary. The final authorization is adjacent to witness
        // construction and prevents an expired command from becoming Verified after read-back.
        match authorize(&token, &facts, &self.root, &self.clock)? {
            Authorization::Allowed => {}
            Authorization::Denied { .. } => {
                return Ok(VerificationDecision::Rejected(Rejection::Authorization));
            }
        }
        let approver = ApproverId::new("polaris-kernel-verifier")?;
        let witness = ApprovalWitness::new(approver, evidence_cid, self.clock.now());
        let command = request.command.clone().verify(witness);
        Ok(VerificationDecision::Approved(Box::new(ApprovedMutation {
            command,
            command_id,
            agent,
            target: resource.as_str().to_owned(),
            diff,
            old_source,
            new_source,
            metrics: measured,
            evidence: evidence_cid,
        })))
    }
}

#[derive(Serialize)]
struct ApprovalEvidence<'a> {
    schema: u16,
    command_id: CommandId,
    agent: FrameId,
    target: &'a str,
    diff: Cid,
    old_source: Cid,
    new_source: Cid,
    mutation_binding: Cid,
    intent_blake3: Cid,
    churn_lines: u32,
    max_churn_lines: u32,
    cyclomatic_delta: i32,
    max_cyclomatic_increase: u32,
    tree_edit_distance: u32,
    tree_edit_distance_exact: bool,
    max_tree_edit_distance: u32,
    touches_unsafe: bool,
    safety_result: EvidenceSafetyResult,
    changed_symbols: Vec<String>,
    checked_at: Timestamp,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum EvidenceSafetyResult {
    PreflightAndCongruenceAllowed,
}

fn cid_for(bytes: &[u8]) -> Cid {
    Cid::from_bytes(*blake3::hash(bytes).as_bytes())
}

fn single_symbol(metrics: &MutationMetrics) -> Option<&str> {
    match metrics.changed_symbols.as_slice() {
        [symbol] => Some(symbol.name.as_str()),
        _ => None,
    }
}

fn unsafe_observations(
    parsed: &polaris_code_intel::Parsed,
    analysis: &polaris_code_intel::SafetyAnalysis,
) -> BTreeMap<(String, Vec<u8>), u32> {
    let mut observations = BTreeMap::new();
    for observation in &analysis.unsafe_constructs {
        if let Some(source) = observation.span.slice(parsed.source()) {
            let key = (observation.node_kind.clone(), source.to_vec());
            let count = observations.entry(key).or_insert(0_u32);
            *count = count.saturating_add(1);
        }
    }
    observations
}
