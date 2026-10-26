use polaris_code_intel::Lang;

/// Caller-provided source file; bytes are never read from the host filesystem by this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    /// Workspace-relative display path used in findings.
    pub path: String,
    /// Explicit grammar selection; unknown languages are not guessed.
    pub lang: Lang,
    /// Source bytes parsed as provided.
    pub bytes: Vec<u8>,
}

/// Whether evidence is an advisory heuristic or a direct observed fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// Pattern evidence can have false positives and needs human review.
    AdvisoryHeuristic,
    /// A parser/automata observation directly corresponds to the cited syntax/rule.
    DeterministicObservation,
    /// A backend command produced an outcome; this does not imply the outcome is a bug.
    RuntimeObserved,
}

/// Broad finding category emitted by static rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingClass {
    /// Blocking thread sleep candidate inside a syntactically identified async function region.
    BlockingInAsync,
    /// A code-intel observation of an unsafe construct.
    UnsafeConstruct,
    /// An assertion recognized by code-intel as trivially true or self-equal.
    TautologicalAssertion,
    /// Caller-configured automata rule matched source bytes.
    RuleMatch,
}

/// A finding with a byte-accurate source range and inspectable rule evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Relative source path.
    pub path: String,
    /// Finding class.
    pub class: FindingClass,
    /// Rule name or stable rule identifier.
    pub rule: String,
    /// Inclusive byte offset in the original source.
    pub start: usize,
    /// Exclusive byte offset in the original source.
    pub end: usize,
    /// Non-source rule/node metadata; byte offsets identify the relevant original source span.
    pub evidence: String,
    /// Strength of the evidence; heuristics are never promoted to deterministic facts.
    pub confidence: Confidence,
}

/// Static findings and explicit parser coverage notes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StaticReport {
    /// Findings ordered by input file and source byte position.
    pub findings: Vec<Finding>,
    /// Per-file notes where syntax had errors or a supported observation family is incomplete.
    pub coverage_notes: Vec<String>,
}

/// Full report combining static and optional dynamic observations.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AuditReport {
    /// Deterministic static findings.
    pub findings: Vec<Finding>,
    /// Coverage limitations from the parser and configured checks.
    pub coverage_notes: Vec<String>,
    /// Dynamic execution records, when requested.
    pub runs: Vec<crate::DynamicRun>,
    /// Model-token count; no model client or increment path exists in this crate.
    tokens_used: u64,
}

impl AuditReport {
    /// Model tokens consumed by this crate, always zero because it has no model client.
    #[must_use]
    pub const fn tokens_used(&self) -> u64 {
        self.tokens_used
    }

    pub(crate) fn from_parts(
        static_report: StaticReport,
        dynamic_report: Option<crate::DynamicReport>,
    ) -> Self {
        Self {
            findings: static_report.findings,
            coverage_notes: static_report.coverage_notes,
            runs: dynamic_report.map_or_else(Vec::new, |report| report.runs),
            tokens_used: 0,
        }
    }
}
