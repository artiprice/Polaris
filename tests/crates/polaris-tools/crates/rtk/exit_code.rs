//! Exit code (descriptor: specs/descriptors/rtk_exit_code.json).

use std::error::Error;

use jiff::Timestamp;
use polaris_tools_rtk::{Diagnostic, Ledger, Level, exit_code};

type TestResult = Result<(), Box<dyn Error>>;

fn error(message: &str) -> Diagnostic {
    Diagnostic::new(Level::Error, None, message)
}

#[test]
fn no_open_issue_is_zero() -> TestResult {
    let mut ledger = Ledger::default();
    let summary = ledger.reconcile(&[], "2026-09-22T11:04:00Z".parse::<Timestamp>()?);
    assert_eq!(exit_code(&summary), 0);
    Ok(())
}

#[test]
fn one_open_issue_is_one() -> TestResult {
    let mut ledger = Ledger::default();
    let summary = ledger.reconcile(
        &[error("boom")],
        "2026-09-22T11:04:00Z".parse::<Timestamp>()?,
    );
    assert_eq!(summary.open.len(), 1);
    assert_eq!(exit_code(&summary), 1);
    Ok(())
}

#[test]
fn closed_issues_do_not_count() -> TestResult {
    let mut ledger = Ledger::default();
    ledger.reconcile(
        &[error("boom")],
        "2026-09-22T11:04:00Z".parse::<Timestamp>()?,
    );
    let summary = ledger.reconcile(&[], "2026-09-23T11:04:00Z".parse::<Timestamp>()?);
    assert_eq!(summary.newly_closed, 1);
    assert_eq!(exit_code(&summary), 0);
    Ok(())
}
