//! Structured safety evidence without sensitive source excerpts.
use core::{fmt, future::Future};
use polaris_code_intel::{ByteSpan, Parsed, SymbolRef, TautologyKind};
use polaris_security_automata::RuleId;

/// Byte location in one of the two mutation snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    /// Original source byte range.
    pub span: ByteSpan,
    /// True for the old snapshot.
    pub in_old: bool,
}

/// Ways a proposal can neutralize verification machinery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TamperKind {
    /// An existing test disappeared.
    TestDeleted,
    /// A recognized assertion was removed.
    AssertionRemoved,
    /// Changed assertion semantics need separate review.
    AssertionWeakened,
    /// An ignore or skip marker was added.
    IgnoreAttributeAdded,
    /// A condition disabling a test was introduced.
    FeatureGateDisablesTest,
    /// An unfamiliar disabling marker needs review.
    UnknownDisableMarker,
    /// CI or verification policy configuration changed.
    CiConfigEdited,
}

/// Observations that contain no credential or source value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    /// A test or assertion may have been neutralized.
    TestTampering {
        /// Tamper classification.
        kind: TamperKind,
        /// Location without source text.
        loc: SourceLocation,
    },
    /// A signature or entropy rule found a possible credential.
    SecretLeak {
        /// Scanner rule identifier.
        rule: RuleId,
        /// Position in proposed source.
        loc: SourceLocation,
        /// Shannon entropy in millibits per byte.
        entropy_milli: Option<u32>,
    },
    /// A dangerous-shell rule matched.
    DestructiveCommand {
        /// Rule identifier in the command pack.
        rule: RuleId,
    },
    /// A privileged operation needs an explicit execution policy.
    PrivilegedCommand,
    /// A command attempts to access an out-of-scope resource.
    SandboxEscapeAttempt,
    /// An assertion matches a tautology rule.
    Tautology {
        /// AST rule identifier.
        kind: TautologyKind,
        /// Assertion location.
        loc: SourceLocation,
    },
    /// A discovered test has no recognized assertions.
    NoAssertions {
        /// Test location.
        loc: SourceLocation,
    },
    /// The model rejected an affirmative safety statement.
    SemanticRejection {
        /// Static schema identifier, never model output.
        schema: &'static str,
    },
}

/// Missing evidence that prevents an automatic permission decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradedReason {
    /// The required schema is not installed.
    MissingSchema,
    /// No usable backend or checkpoint is available.
    EngineUnavailable,
    /// A per-schema deadline elapsed.
    EngineTimeout,
    /// Output did not satisfy the decision schema.
    InvalidDecision,
    /// Calibration provenance does not match the expected data and checkpoint.
    Uncalibrated,
    /// Confidence or returned answers were inconclusive.
    AmbiguousDecision,
    /// Relevant AST constructs are not covered.
    IncompleteAnalysis,
    /// The parser reported malformed or incomplete syntax.
    SyntaxError,
    /// The shell grammar is too complex for deterministic interpretation.
    OpaqueShell,
    /// Independent proof is absent or bound to a different revision.
    IncompleteProof,
    /// Protected verification configuration changed.
    ProtectedConfiguration,
    /// An assertion changed in a way that cannot be ordered mechanically.
    ChangedAssertion,
}

/// Only `Allow` permits automatic continuation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateVerdict {
    /// Required checks established permission.
    Allow,
    /// An explicit approval decision is required.
    AskUser {
        /// Missing evidence.
        reason: DegradedReason,
    },
    /// A deterministic or semantic check rejected this operation.
    Block {
        /// Non-sensitive observations.
        findings: Vec<Finding>,
    },
}

/// Report before executing a mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightReport {
    /// Permission decision.
    pub verdict: GateVerdict,
    /// Deterministic observations, including those needing review.
    pub findings: Vec<Finding>,
    /// Whether any inference call was attempted.
    pub s1_used: bool,
    /// Missing evidence when applicable.
    pub degraded: Option<DegradedReason>,
}

/// Already-parsed snapshots from a trusted workspace owner.
pub struct SafetyRequest<'a> {
    /// Validated workspace-relative target.
    pub target: &'a str,
    /// Trusted pre-mutation bytes and tree.
    pub old: &'a Parsed,
    /// Proposed bytes and tree.
    pub new: &'a Parsed,
    /// Stated intent, masked before inference.
    pub intent: &'a str,
    /// Changed declarations from trusted analysis.
    pub changed_symbols: &'a [SymbolRef],
}

impl fmt::Debug for SafetyRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SafetyRequest")
            .field("old_bytes", &self.old.source().len())
            .field("new_bytes", &self.new.source().len())
            .field("intent_bytes", &self.intent.len())
            .field("changed_symbols", &self.changed_symbols.len())
            .finish_non_exhaustive()
    }
}

/// Result consumed by the hybrid kernel verifier. No witness is minted here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafetyAssessment {
    /// Preflight and semantic congruence accepted the exact snapshots.
    Allow,
    /// A deterministic or semantic safety check failed.
    Block {
        /// Non-sensitive findings.
        findings: Vec<Finding>,
    },
    /// Evidence is insufficient for automatic approval.
    NeedsApproval {
        /// Missing evidence.
        reason: DegradedReason,
    },
}

/// Injectable safety boundary; implementations must not execute mutations.
pub trait MutationSafety: Send + Sync {
    /// Assess the supplied snapshots, failing closed on errors.
    fn assess(&self, request: &SafetyRequest<'_>)
        -> impl Future<Output = Result<SafetyAssessment, crate::VerifierError>> + Send;
}

/// Report after execution, requiring independent proof for a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostflightReport {
    /// Independent proof result.
    pub verdict: crate::ProofVerdict,
    /// Test-quality observations.
    pub findings: Vec<Finding>,
    /// Deduplicated structured diagnostics.
    pub ranked_diagnostics: Vec<crate::RankedDiagnostic>,
    /// Missing evidence, if any.
    pub degraded: Option<DegradedReason>,
}

/// No-I/O result of concurrent patch arbitration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeDecision {
    /// Syntax-checked bytes with calibrated semantic compatibility evidence.
    CompatibleAutoMerge {
        /// Complete merged source; caller owns atomic workspace commit.
        merged: Vec<u8>,
    },
    /// The edits conflict structurally or semantically.
    SemanticConflict {
        /// Conflicting declarations.
        symbols: Vec<SymbolRef>,
    },
    /// Required evidence is missing; no merged bytes are released.
    RequiresSupervisorAttention {
        /// Missing evidence.
        reason: DegradedReason,
    },
}
