use crate::dynamic::run_dynamic_stress;
use crate::static_scan::static_scan;
use crate::{AuditReport, HackerError, SourceFile, StressPlan};
use polaris_sdk::registry::SandboxBackend;
use polaris_security_automata::DfaScanner;
use std::sync::Arc;

/// Run deterministic static analysis and, when configured, bounded sandbox stress execution.
///
/// The SDK backend and policy are selected by the composition root. No model client is accepted,
/// so model tokens used by this pipeline are structurally fixed at zero. A successful report only
/// contains observations; a clean dynamic run is not evidence of resilience.
///
/// # Errors
/// Returns parser, automata or sandbox errors. Dynamic sandbox execution is omitted when `stress`
/// is `None`.
pub async fn audit<B: SandboxBackend + 'static>(
    files: &[SourceFile],
    rules: &DfaScanner,
    stress: Option<&StressPlan>,
    backend: Arc<B>,
) -> Result<AuditReport, HackerError> {
    let static_report = static_scan(files, rules)?;
    let dynamic_report = match stress {
        Some(plan) => Some(run_dynamic_stress(plan, backend).await?),
        None => None,
    };
    Ok(AuditReport::from_parts(static_report, dynamic_report))
}
