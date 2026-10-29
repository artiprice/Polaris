use polaris_code_intel::ByteSpan;
use polaris_security_rules::{DiffAuditRequest, RuleEngine};
use proptest::prelude::*;
use std::path::Path;

proptest! {
    #[test]
    fn unchanged_revision_never_emits_new_findings(source in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let engine = RuleEngine::load_embedded().map_err(|error| TestCaseError::fail(error.to_string()))?;
        let hunks = [ByteSpan::new(0, source.len())];
        let request = DiffAuditRequest::new(Path::new("src/unchanged.py"), &source, &source, &hunks);
        let report = engine.audit_diff(&request).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert!(report.findings().is_empty());
    }

    #[test]
    fn successful_findings_stay_inside_source_bounds(source in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let engine = RuleEngine::load_embedded().map_err(|error| TestCaseError::fail(error.to_string()))?;
        let hunks = [ByteSpan::new(0, source.len())];
        let request = DiffAuditRequest::new(Path::new("src/changed.py"), b"", &source, &hunks);
        if let Ok(report) = engine.audit_diff(&request) {
            for finding in report.findings() {
                prop_assert!(finding.span().start <= finding.span().end);
                prop_assert!(finding.span().end <= source.len());
            }
        }
    }
}
