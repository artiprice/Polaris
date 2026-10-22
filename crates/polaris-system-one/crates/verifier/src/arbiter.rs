//! Concurrent changes are merged in memory only; conflicts release no partial merged bytes.
use crate::{DegradedReason, GateVerdict, MergeDecision, Scanner, Verifier, VerifierError};
use polaris_code_intel::{MergeOutcome, Parsed, merge3};
use polaris_sdk::registry::DecisionEngine;
use polaris_system_one_schemas::SchemaCatalog;

impl<D: DecisionEngine, C: SchemaCatalog, S: Scanner> Verifier<D, C, S> {
    /// Arbitrate two parsed edits of one immutable ancestor. The workspace owner must bind the
    /// three inputs to one path/snapshot and perform a final compare-and-swap before writing.
    ///
    /// # Errors
    /// Input bounds, incompatible languages, parse limits and scanner failures are propagated.
    pub async fn arbitrate(
        &self,
        base: &Parsed,
        ours: &Parsed,
        theirs: &Parsed,
    ) -> Result<MergeDecision, VerifierError> {
        for parsed in [base, ours, theirs] {
            self.bound(parsed.source())?;
            if parsed.has_syntax_error() {
                return Ok(MergeDecision::RequiresSupervisorAttention {
                    reason: DegradedReason::SyntaxError,
                });
            }
        }
        let merged = match merge3(base, ours, theirs)? {
            MergeOutcome::Conflict { symbols } => {
                return Ok(MergeDecision::SemanticConflict { symbols });
            }
            MergeOutcome::Merged(bytes) => bytes,
        };
        self.bound(&merged)?;
        let merged_tree = self.parsers.parse(base.lang(), &merged, None)?;
        if merged_tree.has_syntax_error() {
            return Ok(MergeDecision::RequiresSupervisorAttention {
                reason: DegradedReason::SyntaxError,
            });
        }
        let analysis = polaris_code_intel::analyze_safety(&merged_tree)?;
        if analysis.syntax_error {
            return Ok(MergeDecision::RequiresSupervisorAttention {
                reason: DegradedReason::SyntaxError,
            });
        }
        let secrets = crate::scan::secret_findings(
            &self.scanner,
            &merged,
            self.config.max_scan_matches,
            self.config.entropy,
        )?;
        if !secrets.is_empty() {
            return Ok(MergeDecision::RequiresSupervisorAttention {
                reason: DegradedReason::IncompleteAnalysis,
            });
        }
        let text = |parsed: &Parsed| {
            std::str::from_utf8(parsed.source())
                .map(str::to_owned)
                .map_err(|_| VerifierError::InvalidInput {
                    reason: "merge source is not UTF-8",
                })
        };
        let input = format!(
            "BASE\n{}\nCHANGE A\n{}\nCHANGE B\n{}\nMERGED\n{}",
            text(base)?,
            text(ours)?,
            text(theirs)?,
            text(&merged_tree)?
        );
        match self
            .semantic("merge.congruence", input.as_bytes(), None)
            .await?
        {
            GateVerdict::Allow => Ok(MergeDecision::CompatibleAutoMerge { merged }),
            GateVerdict::Block { .. } => Ok(MergeDecision::SemanticConflict {
                symbols: Vec::new(),
            }),
            GateVerdict::AskUser { reason } => {
                Ok(MergeDecision::RequiresSupervisorAttention { reason })
            }
        }
    }
}
