//! # polaris-vcs
//!
//! Repository status uses gix. Diff and worktree operations use a bounded, shell-free git process
//! runner with fixed safety configuration. File snapshots are content-addressed in CAS and a local
//! journal supports compare-before-restore undo and checkpoint rollback.

mod git;
mod journal;
mod paths;

pub use git::{
    Clean, DiffSet, Dirty, Removed, Repo, RepoStatus, StatusEntry, StatusKind, Worktree,
};
pub use journal::{CheckpointSeq, FileSnapshot, RollbackPlan, UndoPlan, UndoReport};
pub use paths::{BranchName, RepoPath, RevSpec};

/// Failures from repository access, git, snapshots, and guarded restoration.
#[derive(Debug, thiserror::Error)]
pub enum VcsError {
    /// The path is not inside a git worktree.
    #[error("not a git repository: {0}")]
    NotARepo(String),
    /// User-controlled ref or path text was rejected.
    #[error("invalid git ref or repository path: {0}")]
    InvalidRef(String),
    /// A restore target changed after its snapshot was sealed.
    #[error("restore conflicts with intervening edits at {0}")]
    Conflict(String),
    /// A requested checkpoint does not exist.
    #[error("checkpoint {0} is unknown")]
    UnknownCheckpoint(u64),
    /// The content-addressed snapshot is missing or corrupt.
    #[error("CAS snapshot error: {0}")]
    Cas(#[from] polaris_storage_cas::CasError),
    /// Repository or journal I/O failed.
    #[error("repository I/O: {0}")]
    Io(#[from] std::io::Error),
    /// gix could not read the repository.
    #[error("gix: {0}")]
    Gix(String),
    /// The git process failed, timed out, or exceeded an output limit.
    #[error("git command failed: {0}")]
    Git(String),
    /// Journal data could not be decoded.
    #[error("journal: {0}")]
    Journal(String),
}
