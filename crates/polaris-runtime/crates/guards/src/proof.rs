//! Parsed sandbox evidence and the Proof-of-Done gate.

use crate::completion::{Completion, ProofRecord, Proven};
use crate::error::GuardError;
use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::ids::Cid;
use polaris_sdk::registry::ExecOutput;
use polaris_tools_rtk::LedgerSummary;
use std::str;

/// How the captured test summary was parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseStatus {
    /// Complete recognized runner summaries were parsed from captured output.
    Parsed,
    /// No complete recognized summary was present or output was invalid or excessive.
    Unparseable,
}

/// Recognized test runner output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestRunner {
    /// `cargo test` result lines.
    Cargo,
    /// `cargo nextest run` summary lines.
    Nextest,
    /// Pytest terminal summary lines.
    Pytest,
}

impl TestRunner {
    fn from_command(command: &str) -> Option<Self> {
        if command
            .chars()
            .any(|character| matches!(character, '$' | '%' | '`' | ';' | '|' | '&' | '<' | '>'))
        {
            return None;
        }
        let mut parts = command.split_whitespace();
        let first = parts.next()?;
        let second = parts.next();
        match (first, second) {
            ("cargo", Some("test")) => Some(Self::Cargo),
            ("cargo", Some("nextest")) if parts.next() == Some("run") => Some(Self::Nextest),
            ("nextest", Some("run")) => Some(Self::Nextest),
            ("pytest", _) => Some(Self::Pytest),
            ("python", Some("-m")) if parts.next() == Some("pytest") => Some(Self::Pytest),
            _ => None,
        }
    }
}

/// Parsed test counts bound to both exact captured output streams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestSummary {
    runner: Option<TestRunner>,
    parse: ParseStatus,
    passed: u32,
    failed: u32,
    ignored: u32,
    stdout: Cid,
    stderr: Cid,
}

impl TestSummary {
    /// Parse both streams of actual captured output, binding their BLAKE3 identities.
    /// Malformed, ambiguous, excessive or unrecognized output is marked `Unparseable`.
    #[must_use]
    pub fn from_output(command: &str, output: &ExecOutput) -> Self {
        Self::parse(command, &output.stdout, &output.stderr, cid_for(&output.stdout), cid_for(&output.stderr))
    }

    fn parse(command: &str, stdout: &[u8], stderr: &[u8], stdout_cid: Cid, stderr_cid: Cid) -> Self {
        let runner = TestRunner::from_command(command);
        let counts = stdout.len().checked_add(stderr.len())
            .filter(|length| *length <= 16 * 1024 * 1024)
            .and_then(|_| Some((str::from_utf8(stdout).ok()?, str::from_utf8(stderr).ok()?)))
            .and_then(|(out, err)| runner.and_then(|runner| parse_summary(runner, out, err)));
        match counts {
            Some((passed, failed, ignored)) => Self {
                runner,
                parse: ParseStatus::Parsed,
                passed,
                failed,
                ignored,
                stdout: stdout_cid,
                stderr: stderr_cid,
            },
            None => Self {
                runner,
                parse: ParseStatus::Unparseable,
                passed: 0,
                failed: 0,
                ignored: 0,
                stdout: stdout_cid,
                stderr: stderr_cid,
            },
        }
    }

    /// Runner selected from the submitted command, when recognized.
    #[must_use]
    pub const fn runner(&self) -> Option<TestRunner> {
        self.runner
    }

    /// Parse status.
    #[must_use]
    pub const fn parse_status(&self) -> ParseStatus {
        self.parse
    }

    /// Reported passed tests.
    #[must_use]
    pub const fn passed(&self) -> u32 {
        self.passed
    }

    /// Reported failed tests.
    #[must_use]
    pub const fn failed(&self) -> u32 {
        self.failed
    }

    /// Reported ignored or skipped tests.
    #[must_use]
    pub const fn ignored(&self) -> u32 {
        self.ignored
    }

    /// Content id of the stdout bytes used for parsing.
    #[must_use]
    pub const fn stdout(&self) -> Cid {
        self.stdout
    }

    /// Content id of the stderr bytes included in parsing.
    #[must_use]
    pub const fn stderr(&self) -> Cid {
        self.stderr
    }
}

/// Assertion-tampering facts computed by the system-one verifier postflight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TamperReport {
    /// Assertions removed from changed tests.
    pub removed_assertions: u32,
    /// New ignore or skip markers in changed tests.
    pub new_ignores: u32,
    /// Tautological assertions in changed tests.
    pub tautologies: u32,
}

impl TamperReport {
    /// Whether all deterministic tamper counters are zero.
    #[must_use]
    pub const fn is_clean(self) -> bool {
        self.removed_assertions == 0 && self.new_ignores == 0 && self.tautologies == 0
    }
}

/// Independent postflight evidence required in addition to the ordinary test run and S1 report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndependentCheck {
    /// Mutation testing completed and every viable mutant was killed.
    Mutation {
        /// Number of viable mutants executed.
        mutants_run: u32,
        /// Number killed by the test suite.
        mutants_killed: u32,
        /// Number that survived.
        mutants_survived: u32,
        /// Number that timed out without a verdict.
        mutants_timed_out: u32,
    },
    /// An independent falsifier/property run completed without a counterexample.
    Falsifier {
        /// Number of falsifier cases executed.
        cases_run: u32,
        /// Number of counterexamples found.
        counterexamples: u32,
    },
}

/// Tree- and test-artifact-bound result from independent mutation or falsifier execution.
///
/// The adapter that creates this value must obtain it from an independent postflight runner.
/// This crate does not launch mutation or falsifier tools itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndependentProof {
    check: IndependentCheck,
    final_tree: Cid,
    test_output: Cid,
    report: Cid,
}

impl IndependentProof {
    /// Creates evidence from an independently executed mutation-test adapter.
    #[must_use]
    pub const fn mutation(
        final_tree: Cid,
        test_output: Cid,
        report: Cid,
        mutants_run: u32,
        mutants_killed: u32,
        mutants_survived: u32,
        mutants_timed_out: u32,
    ) -> Self {
        Self {
            check: IndependentCheck::Mutation {
                mutants_run,
                mutants_killed,
                mutants_survived,
                mutants_timed_out,
            },
            final_tree,
            test_output,
            report,
        }
    }

    /// Creates evidence from an independently executed falsifier adapter.
    #[must_use]
    pub const fn falsifier(
        final_tree: Cid,
        test_output: Cid,
        report: Cid,
        cases_run: u32,
        counterexamples: u32,
    ) -> Self {
        Self {
            check: IndependentCheck::Falsifier {
                cases_run,
                counterexamples,
            },
            final_tree,
            test_output,
            report,
        }
    }

    /// Independent check counts.
    #[must_use]
    pub const fn check(self) -> IndependentCheck {
        self.check
    }

    /// Final source-tree hash analyzed by the independent check.
    #[must_use]
    pub const fn final_tree(self) -> Cid {
        self.final_tree
    }

    /// Test output artifact bound to the independent check.
    #[must_use]
    pub const fn test_output(self) -> Cid {
        self.test_output
    }

    /// Content id of the independent runner report.
    #[must_use]
    pub const fn report(self) -> Cid {
        self.report
    }

    fn is_passing(self) -> bool {
        match self.check {
            IndependentCheck::Mutation {
                mutants_run,
                mutants_killed,
                mutants_survived,
                mutants_timed_out,
            } => {
                mutants_run > 0
                    && mutants_killed == mutants_run
                    && mutants_survived == 0
                    && mutants_timed_out == 0
            }
            IndependentCheck::Falsifier {
                cases_run,
                counterexamples,
            } => cases_run > 0 && counterexamples == 0,
        }
    }
}

/// Policy for accepting parsed run evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProofPolicy {
    /// Minimum number of passed tests required for proof.
    min_tests: u32,
    /// Whether caller-supplied build warning counts must be zero.
    require_zero_warnings: bool,
}

impl ProofPolicy {
    /// Creates a policy with a positive minimum test count.
    pub fn new(min_tests: u32, require_zero_warnings: bool) -> Result<Self, GuardError> {
        if min_tests == 0 {
            return Err(GuardError::InvalidLimits);
        }
        Ok(Self {
            min_tests,
            require_zero_warnings,
        })
    }

    /// Minimum passed-test count.
    #[must_use]
    pub const fn min_tests(self) -> u32 {
        self.min_tests
    }

    /// Whether warnings must be absent.
    #[must_use]
    pub const fn require_zero_warnings(self) -> bool {
        self.require_zero_warnings
    }
}

/// Parsed, content-bound facts collected from a real sandbox execution and external ledgers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEvidence {
    command: String,
    exit_code: i32,
    tree: Cid,
    stdout: Cid,
    stderr: Cid,
    open_issues: u32,
    tests: TestSummary,
    tamper: TamperReport,
    independent: IndependentProof,
    warnings: Option<u32>,
}

impl RunEvidence {
    /// Binds parsed counts, external verifier/RTK facts, and independent postflight evidence to
    /// the captured sandbox output.
    ///
    /// The stdout and stderr content ids must equal the BLAKE3 ids of the exact [`ExecOutput`]
    /// bytes. The independent check must be tied to the same final tree and test-output artifact
    /// before it can pass the completion gate. The command is parsed only for the supported
    /// test-runner prefix; no process is started here.
    ///
    /// # Errors
    /// Returns a failure carrying the stderr reference if an output id or RTK count is invalid.
    pub fn from_sandbox(
        command: String,
        output: &ExecOutput,
        tree: Cid,
        stdout: Cid,
        stderr: Cid,
        issues: &LedgerSummary,
        tamper: TamperReport,
        independent: IndependentProof,
        warnings: Option<u32>,
    ) -> Result<Self, ProofFailure> {
        if cid_for(output.stdout.as_ref()) != stdout || cid_for(output.stderr.as_ref()) != stderr {
            return Err(ProofFailure::OutputIdMismatch { stderr });
        }
        let open_issues =
            u32::try_from(issues.open.len()).map_err(|_| ProofFailure::CountOverflow { stderr })?;
        let tests = TestSummary::parse(&command, output.stdout.as_ref(), output.stderr.as_ref(), stdout, stderr);
        Ok(Self {
            command,
            exit_code: output.exit_code,
            tree,
            stdout,
            stderr,
            open_issues,
            tests,
            tamper,
            independent,
            warnings,
        })
    }

    /// Submitted command line.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Process exit code from the sandbox backend.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        self.exit_code
    }

    /// Tree hash captured for this run.
    #[must_use]
    pub const fn tree(&self) -> Cid {
        self.tree
    }

    /// Content id of captured stdout.
    #[must_use]
    pub const fn stdout(&self) -> Cid {
        self.stdout
    }

    /// Content id of captured stderr.
    #[must_use]
    pub const fn stderr(&self) -> Cid {
        self.stderr
    }

    /// Parsed RTK open-issue count.
    #[must_use]
    pub const fn open_issues(&self) -> u32 {
        self.open_issues
    }

    /// Parsed test summary.
    #[must_use]
    pub const fn tests(&self) -> &TestSummary {
        &self.tests
    }

    /// Injected system-one postflight tamper report.
    #[must_use]
    pub const fn tamper(&self) -> TamperReport {
        self.tamper
    }

    /// Independent mutation/falsifier evidence bound to this tree and test output.
    #[must_use]
    pub const fn independent(&self) -> IndependentProof {
        self.independent
    }
}

/// A reason a completion claim failed its evidence gate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProofFailure {
    /// Sandbox process failed or RTK still has open issues.
    #[error("build/run failed with {open_issues} open RTK issue(s)")]
    BuildFailed {
        /// Number of open issues after ledger reconciliation.
        open_issues: u32,
        /// Captured stderr content id for bounded self-correction.
        stderr: Cid,
    },
    /// The parsed test run reported failed tests.
    #[error("{failed} test(s) failed")]
    TestsFailed {
        /// Number of reported failed tests.
        failed: u32,
        /// Captured stderr content id for bounded self-correction.
        stderr: Cid,
    },
    /// No tests passed, so the run cannot prove completion.
    #[error("no tests passed")]
    NoTestsRan {
        /// Captured stderr content id for bounded self-correction.
        stderr: Cid,
    },
    /// The test runner output was not recognized or the command was not a supported runner.
    #[error("test output was unparseable")]
    Unparseable {
        /// Captured stderr content id for bounded self-correction.
        stderr: Cid,
    },
    /// Passed test count was below the policy minimum.
    #[error("only {passed} test(s) passed; minimum is {minimum}")]
    InsufficientTests {
        /// Number of passed tests.
        passed: u32,
        /// Required minimum.
        minimum: u32,
        /// Captured stderr content id for bounded self-correction.
        stderr: Cid,
    },
    /// The source tree changed after the successful run.
    #[error("tree changed after verification")]
    TreeChanged {
        /// Tree hash observed by the run.
        expected: Cid,
        /// Current tree hash at the gate.
        actual: Cid,
        /// Captured stderr content id for bounded self-correction.
        stderr: Cid,
    },
    /// Verifier postflight found assertion tampering.
    #[error("test tampering was detected")]
    Tampering {
        /// Tamper counters from system-one verifier.
        report: TamperReport,
        /// Captured stderr content id for bounded self-correction.
        stderr: Cid,
    },
    /// Warnings were present or the required warning count was unavailable.
    #[error("warning policy was not satisfied")]
    Warnings {
        /// Warning count, or `None` when the caller could not provide an independent count.
        count: Option<u32>,
        /// Captured stderr content id for bounded self-correction.
        stderr: Cid,
    },
    /// Independent mutation/falsifier evidence was absent, failing, or bound to different artifacts.
    #[error("independent postflight evidence was not valid for this run")]
    IndependentEvidence {
        /// Captured stderr content id for bounded self-correction.
        stderr: Cid,
    },
    /// Captured stdout or stderr content id did not match the backend bytes.
    #[error("sandbox output content id mismatch")]
    OutputIdMismatch {
        /// Captured stderr content id supplied by the caller.
        stderr: Cid,
    },
    /// An external issue count could not fit the gate's bounded counter.
    #[error("evidence count overflow")]
    CountOverflow {
        /// Captured stderr content id supplied by the caller.
        stderr: Cid,
    },
}

impl ProofFailure {
    /// Captured stderr content id associated with the blocked run.
    #[must_use]
    pub const fn stderr(&self) -> Cid {
        match self {
            Self::BuildFailed { stderr, .. }
            | Self::TestsFailed { stderr, .. }
            | Self::NoTestsRan { stderr }
            | Self::Unparseable { stderr }
            | Self::InsufficientTests { stderr, .. }
            | Self::TreeChanged { stderr, .. }
            | Self::Tampering { stderr, .. }
            | Self::Warnings { stderr, .. }
            | Self::IndependentEvidence { stderr }
            | Self::OutputIdMismatch { stderr }
            | Self::CountOverflow { stderr } => *stderr,
        }
    }
}

impl HasKind for ProofFailure {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::CountOverflow { .. } => ErrorKind::Fatal,
            Self::BuildFailed { .. }
            | Self::TestsFailed { .. }
            | Self::NoTestsRan { .. }
            | Self::Unparseable { .. }
            | Self::InsufficientTests { .. }
            | Self::TreeChanged { .. }
            | Self::Tampering { .. }
            | Self::Warnings { .. }
            | Self::IndependentEvidence { .. }
            | Self::OutputIdMismatch { .. } => ErrorKind::Policy,
        }
    }
}

/// Produces a proven completion only when every independent evidence check passes.
///
/// The current tree must equal both the run tree and the expected completion tree supplied when
/// the pending completion was created. This deliberately enforces freshness regardless of any
/// caller's proof-age preference. A green exit code alone never counts as test evidence.
///
/// # Errors
/// Returns a [`ProofFailure`] carrying the run's stderr Cid for bounded corrective feedback.
pub fn proof_of_done(
    completion: Completion<crate::Pending>,
    run: &RunEvidence,
    now_tree: Cid,
    policy: &ProofPolicy,
) -> Result<Completion<Proven>, ProofFailure> {
    let stderr = run.stderr;
    if run.exit_code != 0 || run.open_issues != 0 {
        return Err(ProofFailure::BuildFailed {
            open_issues: run.open_issues,
            stderr,
        });
    }
    if run.tests.parse != ParseStatus::Parsed {
        return Err(ProofFailure::Unparseable { stderr });
    }
    if run.tests.stdout != run.stdout || run.tests.stderr != run.stderr {
        return Err(ProofFailure::OutputIdMismatch { stderr });
    }
    if run.tests.passed == 0 {
        return Err(ProofFailure::NoTestsRan { stderr });
    }
    if run.tests.failed > 0 {
        return Err(ProofFailure::TestsFailed {
            failed: run.tests.failed,
            stderr,
        });
    }
    if run.tests.passed < policy.min_tests {
        return Err(ProofFailure::InsufficientTests {
            passed: run.tests.passed,
            minimum: policy.min_tests,
            stderr,
        });
    }
    if !run.tamper.is_clean() {
        return Err(ProofFailure::Tampering {
            report: run.tamper,
            stderr,
        });
    }
    if !run.independent.is_passing()
        || run.independent.final_tree != run.tree
        || run.independent.final_tree != now_tree
        || run.independent.test_output != run.stdout
    {
        return Err(ProofFailure::IndependentEvidence { stderr });
    }
    if policy.require_zero_warnings && run.warnings != Some(0) {
        return Err(ProofFailure::Warnings {
            count: run.warnings,
            stderr,
        });
    }
    if run.tree != completion.expected_tree() || run.tree != now_tree {
        return Err(ProofFailure::TreeChanged {
            expected: run.tree,
            actual: now_tree,
            stderr,
        });
    }
    let proof = ProofRecord::new(
        completion.command(),
        run.tree,
        run.stdout,
        run.stderr,
        run.tests.passed,
        run.tests.failed,
        run.open_issues,
        run.independent.check,
        run.independent.report,
    );
    Ok(completion.proven(proof))
}

fn cid_for(bytes: &[u8]) -> Cid {
    Cid::from_bytes(*blake3::hash(bytes).as_bytes())
}

fn parse_summary(runner: TestRunner, stdout: &str, stderr: &str) -> Option<(u32, u32, u32)> {
    let mut total: Option<(u32, u32, u32)> = None;
    for line in stdout.lines().chain(stderr.lines()).map(str::trim) {
        let candidate = match runner {
            TestRunner::Cargo => line.starts_with("test result:"),
            TestRunner::Nextest => line.starts_with("Summary ") && line.contains("tests run:"),
            TestRunner::Pytest => line.starts_with("===") && line.contains(" in "),
        };
        if !candidate { continue; }
        let counts = match runner {
            TestRunner::Cargo => parse_cargo_line(line),
            TestRunner::Nextest => parse_nextest_line(line),
            TestRunner::Pytest => parse_pytest_line(line),
        }?;
        total = Some(match total {
            Some((passed, failed, ignored)) => {
                // Cargo emits a separate summary for every binary and doctest group.
                // Nextest and pytest must emit exactly one terminal summary.
                if runner != TestRunner::Cargo { return None; }
                (passed.checked_add(counts.0)?, failed.checked_add(counts.1)?, ignored.checked_add(counts.2)?)
            }
            None => counts,
        });
    }
    total
}

fn parse_cargo_line(line: &str) -> Option<(u32, u32, u32)> {
    let summary = line.strip_prefix("test result: ")?;
    let (status, summary) = summary.split_once(". ")?;
    let passed = named_count(summary, "passed")?;
    let failed = named_count(summary, "failed")?;
    let ignored = named_count(summary, "ignored")?;
    if !matches!((status, failed), ("ok", 0) | ("FAILED", 1..)) { return None; }
    Some((passed, failed, ignored))
}

fn parse_nextest_line(line: &str) -> Option<(u32, u32, u32)> {
    let line = line.strip_prefix("Summary ")?;
    let (prefix, summary) = line.split_once("tests run:")?;
    let run = prefix.split_whitespace().last()?.parse::<u32>().ok()?;
    let passed = named_count(summary, "passed")?;
    let failed = match named_count(summary, "failed") {
        Some(value) => value,
        None => 0,
    };
    let ignored = named_count(summary, "skipped")?;
    // Unrecognized timeout, exec-failure and leak classifications cannot prove success.
    if summary.contains("timed out") || summary.contains("exec failed") || summary.contains("leaky")
        || passed.checked_add(failed)? != run { return None; }
    Some((passed, failed, ignored))
}

fn parse_pytest_line(line: &str) -> Option<(u32, u32, u32)> {
    if !line.contains(" in ") || !(line.contains("passed") || line.contains("failed")) {
        return None;
    }
    let passed = match named_count(line, "passed") {
        Some(value) => value,
        None => 0,
    };
    let failed = match named_count(line, "failed") {
        Some(value) => value,
        None => 0,
    };
    if line.contains("error") || line.contains("xpassed") { return None; }
    let skipped = match named_count(line, "skipped") { Some(count) => count, None => 0 };
    let deselected = match named_count(line, "deselected") { Some(count) => count, None => 0 };
    let ignored = skipped.checked_add(deselected)?;
    Some((passed, failed, ignored))
}

fn named_count(text: &str, name: &str) -> Option<u32> {
    let mut previous: Option<&str> = None;
    for word in text.split_whitespace() {
        let normalized = word.trim_matches(|character: char| {
            matches!(character, ':' | ';' | ',' | '.' | '[' | ']' | '(' | ')')
        });
        if normalized == name {
            let count = previous?.trim_matches(|character: char| {
                matches!(character, ':' | ';' | ',' | '.' | '[' | ']' | '(' | ')')
            });
            return count.parse::<u32>().ok();
        }
        previous = Some(word);
    }
    None
}
