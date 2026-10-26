//! Mutation receipts backed by the workspace's durable VCS snapshot and undo journal.
use crate::{Decision, ExperimentId};
use polaris_sdk::ids::Cid;
use polaris_storage_cas::content_id;
use polaris_vcs::{CheckpointSeq, Repo, RepoPath, UndoPlan, VcsError};
use std::path::Path;

/// A VCS-backed owner of research mutations.
#[derive(Debug)]
pub struct VcsWorkspace {
    repo: Repo,
}

/// Workspace acknowledgment with the exact receipt bytes needed for durable evidence retrieval.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceReceipt {
    /// Canonical content ID over `payload`.
    pub cid: Cid,
    /// Exact VCS plan and restoration result bytes.
    pub payload: Vec<u8>,
}

/// Requested experiment mutation boundary.
pub trait ExperimentWorkspace {
    /// Snapshot exact touched paths before tools can modify them.
    ///
    /// # Errors
    /// Returns [`WorkspaceError`] if a path is invalid or the snapshot cannot be persisted.
    fn begin(&self, id: ExperimentId, paths: &[String]) -> Result<(), WorkspaceError>;

    /// Seal and acknowledge a keep or revert mutation.
    ///
    /// # Errors
    /// Returns [`WorkspaceError`] when VCS cannot prove the requested disposition.
    fn settle(
        &self,
        id: ExperimentId,
        decision: Decision,
    ) -> Result<WorkspaceReceipt, WorkspaceError>;

    /// Restore the last acknowledged baseline after interruption.
    ///
    /// # Errors
    /// Returns [`WorkspaceError`] when restoration cannot be proven safe and complete.
    fn recover(&self, id: ExperimentId) -> Result<WorkspaceReceipt, WorkspaceError>;
}

impl VcsWorkspace {
    /// Open a repository and its persistent Polaris snapshot journal.
    ///
    /// # Errors
    /// Returns [`VcsError`] if the repository or its VCS state cannot be opened.
    pub fn open(root: &Path) -> Result<Self, VcsError> {
        Ok(Self {
            repo: Repo::open(root)?,
        })
    }

    /// The canonical repository root used by the VCS adapter.
    #[must_use]
    pub fn repo(&self) -> &Repo {
        &self.repo
    }
}

impl ExperimentWorkspace for VcsWorkspace {
    fn begin(&self, id: ExperimentId, paths: &[String]) -> Result<(), WorkspaceError> {
        if paths.is_empty() || paths.len() > 100_000 {
            return Err(WorkspaceError::Paths);
        }
        let mut validated = paths
            .iter()
            .map(|path| RepoPath::parse(path).map_err(WorkspaceError::Vcs))
            .collect::<Result<Vec<_>, _>>()?;
        validated.sort();
        if validated.windows(2).any(|pair| pair.first() == pair.get(1)) {
            return Err(WorkspaceError::Paths);
        }
        self.repo
            .snapshot_before_edit(sequence(id), &validated)
            .map_err(WorkspaceError::Vcs)
    }

    fn settle(
        &self,
        id: ExperimentId,
        decision: Decision,
    ) -> Result<WorkspaceReceipt, WorkspaceError> {
        let sequence = sequence(id);
        self.repo.seal_edit(sequence).map_err(WorkspaceError::Vcs)?;
        let plan = self.repo.undo_plan().map_err(WorkspaceError::Vcs)?;
        verify_plan(&plan, sequence)?;
        let mut payload = receipt_bytes(id, decision, &plan)?;
        if decision == Decision::Revert {
            let restored = self.repo.undo().map_err(WorkspaceError::Vcs)?;
            for path in restored.restored {
                append_text(&mut payload, path.as_str())?;
            }
            for path in restored.already_restored {
                append_text(&mut payload, path.as_str())?;
            }
        }
        Ok(workspace_receipt(payload))
    }

    fn recover(&self, id: ExperimentId) -> Result<WorkspaceReceipt, WorkspaceError> {
        let sequence = sequence(id);
        let plan = self.repo.undo_plan().map_err(WorkspaceError::Vcs)?;
        if plan.sequence != Some(sequence) {
            self.repo.seal_edit(sequence).map_err(WorkspaceError::Vcs)?;
        }
        let plan = self.repo.undo_plan().map_err(WorkspaceError::Vcs)?;
        verify_plan(&plan, sequence)?;
        let mut payload = receipt_bytes(id, Decision::Revert, &plan)?;
        let restored = self.repo.undo().map_err(WorkspaceError::Vcs)?;
        for path in restored.restored {
            append_text(&mut payload, path.as_str())?;
        }
        for path in restored.already_restored {
            append_text(&mut payload, path.as_str())?;
        }
        Ok(workspace_receipt(payload))
    }
}

fn workspace_receipt(payload: Vec<u8>) -> WorkspaceReceipt {
    let cid = content_id(&payload);
    WorkspaceReceipt { cid, payload }
}

fn sequence(id: ExperimentId) -> CheckpointSeq {
    CheckpointSeq::new(u64::from(id.get()))
}

fn verify_plan(plan: &UndoPlan, expected: CheckpointSeq) -> Result<(), WorkspaceError> {
    if plan.sequence != Some(expected) || !plan.conflicts.is_empty() || plan.files.is_empty() {
        return Err(WorkspaceError::Receipt);
    }
    Ok(())
}

fn receipt_bytes(
    id: ExperimentId,
    decision: Decision,
    plan: &UndoPlan,
) -> Result<Vec<u8>, WorkspaceError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"POLARIS-RESEARCH-MUTATION\x01");
    bytes.extend_from_slice(&id.get().to_le_bytes());
    bytes.push(match decision {
        Decision::Keep => 0,
        Decision::Revert => 1,
    });
    for file in &plan.files {
        append_text(&mut bytes, &file.path)?;
        append_cid(&mut bytes, file.before);
        bytes.push(u8::from(file.before_executable));
        append_cid(&mut bytes, file.after);
        bytes.push(u8::from(file.after_executable));
    }
    Ok(bytes)
}

fn append_text(bytes: &mut Vec<u8>, value: &str) -> Result<(), WorkspaceError> {
    let len = u64::try_from(value.len()).map_err(|_| WorkspaceError::Overflow)?;
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

fn append_cid(bytes: &mut Vec<u8>, cid: Option<Cid>) {
    match cid {
        Some(cid) => {
            bytes.push(1);
            bytes.extend_from_slice(cid.as_bytes());
        }
        None => bytes.push(0),
    }
}

/// VCS-backed research mutation could not be validated or acknowledged.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    /// A path is missing, duplicated, or outside the VCS path contract.
    #[error("invalid research mutation paths")]
    Paths,
    /// VCS rejected the repository operation.
    #[error("research VCS operation failed: {0}")]
    Vcs(#[source] VcsError),
    /// VCS did not report the exact mutation or reported conflicts.
    #[error("research mutation receipt is missing or conflicted")]
    Receipt,
    /// A path length could not be represented in the receipt format.
    #[error("research mutation receipt length overflow")]
    Overflow,
}
