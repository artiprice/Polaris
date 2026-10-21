//! Typestate task-completion witness.

use crate::proof::IndependentCheck;
use polaris_sdk::ids::{Cid, CommandId};
use std::marker::PhantomData;

/// A task whose completion has not passed the evidence gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pending;

/// A task whose run evidence passed [`crate::proof_of_done`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Proven;

/// Completion state. Fields stay private so callers cannot construct the `Proven` state directly.
#[derive(Debug, PartialEq, Eq)]
pub struct Completion<S> {
    command: CommandId,
    expected_tree: Cid,
    proof: Option<ProofRecord>,
    state: PhantomData<S>,
}

impl Completion<Pending> {
    /// Starts a completion record for a command and the tree expected at verification time.
    #[must_use]
    pub fn pending(command: CommandId, expected_tree: Cid) -> Self {
        Self {
            command,
            expected_tree,
            proof: None,
            state: PhantomData,
        }
    }
}

impl<S> Completion<S> {
    /// Command associated with this completion attempt.
    #[must_use]
    pub const fn command(&self) -> CommandId {
        self.command
    }

    /// Expected tree snapshot at the time the completion attempt was created.
    #[must_use]
    pub const fn expected_tree(&self) -> Cid {
        self.expected_tree
    }
}

impl Completion<Proven> {
    /// Evidence accepted by the completion gate.
    #[must_use]
    pub fn proof(&self) -> Option<&ProofRecord> {
        self.proof.as_ref()
    }
}

impl Completion<Pending> {
    pub(crate) fn proven(self, proof: ProofRecord) -> Completion<Proven> {
        Completion {
            command: self.command,
            expected_tree: self.expected_tree,
            proof: Some(proof),
            state: PhantomData,
        }
    }
}

/// Evidence references and counts accepted by the completion gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofRecord {
    command: CommandId,
    tree: Cid,
    stdout: Cid,
    stderr: Cid,
    tests_passed: u32,
    tests_failed: u32,
    open_issues: u32,
    independent_check: IndependentCheck,
    independent_report: Cid,
}

impl ProofRecord {
    pub(crate) fn new(
        command: CommandId,
        tree: Cid,
        stdout: Cid,
        stderr: Cid,
        tests_passed: u32,
        tests_failed: u32,
        open_issues: u32,
        independent_check: IndependentCheck,
        independent_report: Cid,
    ) -> Self {
        Self {
            command,
            tree,
            stdout,
            stderr,
            tests_passed,
            tests_failed,
            open_issues,
            independent_check,
            independent_report,
        }
    }

    /// Command associated with the accepted run.
    #[must_use]
    pub const fn command(&self) -> CommandId {
        self.command
    }

    /// Tree hash observed by the successful run.
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

    /// Number of tests reported as passed.
    #[must_use]
    pub const fn tests_passed(&self) -> u32 {
        self.tests_passed
    }

    /// Number of tests reported as failed.
    #[must_use]
    pub const fn tests_failed(&self) -> u32 {
        self.tests_failed
    }

    /// Number of open RTK issues at reconciliation time.
    #[must_use]
    pub const fn open_issues(&self) -> u32 {
        self.open_issues
    }

    /// Independent mutation or falsifier evidence accepted by Proof-of-Done.
    #[must_use]
    pub const fn independent_check(&self) -> IndependentCheck {
        self.independent_check
    }

    /// Content id of the independent postflight report.
    #[must_use]
    pub const fn independent_report(&self) -> Cid {
        self.independent_report
    }
}
