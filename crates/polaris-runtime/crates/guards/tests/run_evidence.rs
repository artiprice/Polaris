use polaris_runtime_guards::{
    Completion, IndependentProof, ParseStatus, ProofFailure, ProofPolicy, RunEvidence,
    TamperReport, proof_of_done,
};
use polaris_sdk::{
    ids::{Cid, CommandId},
    registry::ExecOutput,
};
use polaris_tools_rtk::LedgerSummary;

fn cid(bytes: &[u8]) -> Cid {
    Cid::from_bytes(*blake3::hash(bytes).as_bytes())
}
fn evidence(command: &str, stdout: &[u8], stderr: &[u8]) -> Result<RunEvidence, ProofFailure> {
    let output = ExecOutput {
        stdout: stdout.to_vec().into(),
        stderr: stderr.to_vec().into(),
        ..ExecOutput::default()
    };
    RunEvidence::from_sandbox(
        command.to_owned(),
        &output,
        cid(b"tree"),
        cid(stdout),
        cid(stderr),
        &LedgerSummary::default(),
        TamperReport::default(),
        IndependentProof::falsifier(cid(b"tree"), cid(stdout), cid(b"independent report"), 64, 0),
        Some(0),
    )
}
fn accepts(
    run: &RunEvidence,
    tree: Cid,
) -> Result<Completion<polaris_runtime_guards::Proven>, ProofFailure> {
    proof_of_done(
        Completion::pending(CommandId::from_u128(1), cid(b"tree")),
        run,
        tree,
        &ProofPolicy::new(3, true).expect("positive policy"),
    )
}

#[test]
fn cargo_summaries_include_all_binaries() {
    let run = evidence("cargo test --workspace", b"test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\ntest result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n", b"").expect("bound output");
    assert_eq!((run.tests().passed(), run.tests().ignored()), (5, 1));
    assert!(accepts(&run, cid(b"tree")).is_ok());
}
#[test]
fn stderr_nextest_summary_is_bound() {
    let run = evidence(
        "cargo nextest run",
        b"",
        b"Summary [ 0.123s] 3 tests run: 3 passed, 0 skipped\n",
    )
    .expect("bound output");
    assert_eq!(run.tests().passed(), 3);
    assert!(accepts(&run, cid(b"tree")).is_ok());
}
#[test]
fn earlier_cargo_failure_is_not_hidden_by_later_success() {
    let run = evidence("cargo test", b"test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s\ntest result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s\n", b"").expect("bound output");
    assert!(matches!(
        accepts(&run, cid(b"tree")),
        Err(ProofFailure::TestsFailed { failed: 1, .. })
    ));
}
#[test]
fn malformed_and_overflowing_counts_fail_closed() {
    for text in [
        "test result: ok. 4294967295 passed; 0 failed; 0 ignored;\ntest result: ok. 1 passed; 0 failed; 0 ignored;",
        "test result: ok. invalid passed; 0 failed; 0 ignored;\ntest result: ok. 3 passed; 0 failed; 0 ignored;",
    ] {
        let run = evidence("cargo test", text.as_bytes(), b"").expect("bound output");
        assert_eq!(run.tests().parse_status(), ParseStatus::Unparseable);
        assert!(accepts(&run, cid(b"tree")).is_err());
    }
}
#[test]
fn failed_or_stale_evidence_cannot_complete() {
    let run = evidence(
        "cargo test",
        b"test result: ok. 3 passed; 0 failed; 0 ignored;",
        b"",
    )
    .expect("bound output");
    assert!(accepts(&run, cid(b"changed tree")).is_err());
    let output = ExecOutput {
        stdout: b"test result: ok. 3 passed; 0 failed; 0 ignored;"
            .to_vec()
            .into(),
        ..ExecOutput::default()
    };
    assert!(matches!(
        RunEvidence::from_sandbox(
            "cargo test".into(),
            &output,
            cid(b"tree"),
            cid(b"forged"),
            cid(b""),
            &LedgerSummary::default(),
            TamperReport::default(),
            run.independent(),
            Some(0)
        ),
        Err(ProofFailure::OutputIdMismatch { .. })
    ));
    let tampered = RunEvidence::from_sandbox(
        "cargo test".into(),
        &output,
        cid(b"tree"),
        cid(&output.stdout),
        cid(b""),
        &LedgerSummary::default(),
        TamperReport {
            removed_assertions: 1,
            ..TamperReport::default()
        },
        run.independent(),
        Some(0),
    )
    .expect("captured output");
    assert!(matches!(
        accepts(&tampered, cid(b"tree")),
        Err(ProofFailure::Tampering { .. })
    ));
}

#[test]
fn malformed_optional_counts_and_duplicate_summaries_cannot_hide_failures() {
    for (command, text) in [
        (
            "cargo nextest run",
            "Summary [ 0.1s] 3 tests run: 3 passed, invalid failed, 0 skipped",
        ),
        ("pytest", "=== 3 passed, invalid failed in 0.01s ==="),
        ("pytest", "=== 3 passed, 0 failed, 1 failed in 0.01s ==="),
        (
            "cargo nextest run",
            "Summary [ 0.1s] 3 tests run: 3 passed, 0 skipped\nSummary [ 0.1s] 3 tests run: 3 passed, 0 skipped",
        ),
    ] {
        let run = evidence(command, text.as_bytes(), b"").expect("bound output");
        assert_eq!(run.tests().parse_status(), ParseStatus::Unparseable);
        assert!(accepts(&run, cid(b"tree")).is_err());
    }
}

#[test]
fn pytest_skipped_deselected_and_xfailed_counts_are_all_retained() {
    let run = evidence(
        "pytest",
        b"=== 3 passed, 2 skipped, 4 deselected, 1 xfailed in 0.01s ===",
        b"",
    )
    .expect("bound output");
    assert_eq!(run.tests().ignored(), 7);
    assert!(accepts(&run, cid(b"tree")).is_ok());
}
