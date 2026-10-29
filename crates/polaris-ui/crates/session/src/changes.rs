//! `/diff` and `/undo` over the changes a session applied. Every reviewed change the kernel
//! commits is a `DiffApproved` event whose diff replaces the whole file in one hunk, so the
//! earlier and the applied content of each change can be read back from the session's log.
//!
//! `/undo` restores the newest change whose file still holds exactly what it wrote; a change
//! already undone is skipped, and a file edited since is left alone. The restore is written by
//! the host, outside the review flow: the session log keeps the original change.

use std::path::{Component, Path};
use std::sync::Arc;

use polaris_kernel::KernelHandle;
use polaris_sdk::contracts::Event;
use polaris_sdk::ids::{Cid, Seq};

use crate::SessionHostError;

/// Changes `/diff` lists.
const LISTED: usize = 20;

/// One applied change, read back from its diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedChange {
    /// Workspace-relative path with `/` separators.
    pub path: String,
    /// Content before the change (empty for a created file).
    pub old: String,
    /// Content the change wrote (empty for a deleted file).
    pub new: String,
    /// The change created the file.
    pub created: bool,
    /// The change deleted the file.
    pub deleted: bool,
}

/// The earlier and the applied content of a kernel diff (one whole-file hunk), with its path.
#[must_use]
pub fn parse_whole_file_diff(text: &str) -> Option<AppliedChange> {
    let path = text
        .lines()
        .find_map(|line| line.strip_prefix("+++ b/"))?
        .to_owned();
    let (header, body) = text.split_once("@@\n")?;
    let hunk = header.rsplit_once("@@ ")?.1;
    let created = hunk.starts_with("-0,0 ");
    let deleted = hunk.trim_end().ends_with("+0,0");
    let (mut old, mut new) = (String::new(), String::new());
    let mut last = None;
    for line in body.split_inclusive('\n') {
        if line.starts_with("\\ ") {
            let target = match last {
                Some('-') => &mut old,
                Some('+') => &mut new,
                _ => continue,
            };
            if target.ends_with('\n') {
                target.pop();
            }
            continue;
        }
        let mut chars = line.chars();
        let marker = chars.next()?;
        match marker {
            '-' => old.push_str(chars.as_str()),
            '+' => new.push_str(chars.as_str()),
            _ => return None,
        }
        last = Some(marker);
    }
    Some(AppliedChange {
        path,
        old,
        new,
        created,
        deleted,
    })
}

/// Every change the session applied, oldest first.
/// # Errors
/// The event log or the content store cannot be read.
pub async fn applied_changes(
    kernel: &KernelHandle,
) -> Result<Vec<AppliedChange>, SessionHostError> {
    let context = Arc::clone(kernel.context());
    tokio::task::spawn_blocking(move || {
        use polaris_kernel::verifier::EvidenceStore;
        let records = context
            .records(Seq::new(1), Seq::new(u64::MAX))
            .map_err(|_| SessionHostError::Unavailable("the session log cannot be read"))?;
        let diffs: Vec<Cid> = records
            .iter()
            .filter_map(|record| match record.event() {
                Event::DiffApproved { diff, .. } => Some(*diff),
                _ => None,
            })
            .collect();
        let store = polaris_kernel::KernelEvidence::new(context);
        let mut changes = Vec::with_capacity(diffs.len());
        for diff in diffs {
            let Ok(Some(bytes)) = store.get(&diff) else {
                continue;
            };
            if let Some(change) = String::from_utf8(bytes)
                .ok()
                .and_then(|text| parse_whole_file_diff(&text))
            {
                changes.push(change);
            }
        }
        Ok(changes)
    })
    .await
    .map_err(|_| SessionHostError::Unavailable("change history task failed"))?
}

/// `/diff`: the session's applied changes, newest first, with their line counts.
/// # Errors
/// The event log or the content store cannot be read.
pub async fn diff_lines(kernel: &KernelHandle) -> Result<Vec<String>, SessionHostError> {
    let changes = applied_changes(kernel).await?;
    if changes.is_empty() {
        return Ok(vec![
            "No change has been applied in this session; each proposed change waits for your review with its diff."
                .to_owned(),
        ]);
    }
    let mut lines = vec![format!(
        "{} change{} applied in this session (newest first):",
        changes.len(),
        if changes.len() == 1 { "" } else { "s" }
    )];
    for change in changes.iter().rev().take(LISTED) {
        let what = if change.created {
            "created".to_owned()
        } else if change.deleted {
            "deleted".to_owned()
        } else {
            format!(
                "-{} +{} lines",
                change.old.lines().count(),
                change.new.lines().count()
            )
        };
        lines.push(format!("  {} ({what})", change.path));
    }
    if changes.len() > LISTED {
        lines.push(format!("  …and {} earlier.", changes.len() - LISTED));
    }
    lines.push("/undo restores the newest one whose file is unchanged since.".to_owned());
    Ok(lines)
}

/// `/undo`: restore the newest applied change of `workspace` whose file still holds what it
/// wrote.
/// # Errors
/// The event log cannot be read, or the restore cannot be written.
pub async fn undo_lines(
    kernel: &KernelHandle,
    workspace: &Path,
) -> Result<Vec<String>, SessionHostError> {
    let changes = applied_changes(kernel).await?;
    let root = workspace.to_owned();
    tokio::task::spawn_blocking(move || undo_newest(&root, &changes))
        .await
        .map_err(|_| SessionHostError::Unavailable("undo task failed"))?
}

fn undo_newest(root: &Path, changes: &[AppliedChange]) -> Result<Vec<String>, SessionHostError> {
    for change in changes.iter().rev() {
        let relative = Path::new(&change.path);
        if relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            continue;
        }
        let target = root.join(relative);
        let current = match std::fs::read(&target) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => {
                return Err(SessionHostError::Ui(format!(
                    "{} cannot be read; nothing was undone",
                    change.path
                )));
            }
        };
        let holds_new = if change.deleted {
            current.is_none()
        } else {
            current.as_deref() == Some(change.new.as_bytes())
        };
        let holds_old = if change.created {
            current.is_none()
        } else {
            current.as_deref() == Some(change.old.as_bytes())
        };
        if holds_old && !holds_new {
            // Already undone; look at the change before it.
            continue;
        }
        if !holds_new {
            return Ok(vec![format!(
                "{} changed after Polaris wrote it; nothing was undone (the newest change is not overwritten).",
                change.path
            )]);
        }
        let done = if change.created {
            std::fs::remove_file(&target)
                .map(|()| format!("Removed {} (it was created by the change).", change.path))
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|_| {
                    SessionHostError::Ui(format!("{} cannot be restored", change.path))
                })?;
            }
            std::fs::write(&target, change.old.as_bytes()).map(|()| {
                if change.deleted {
                    format!("Restored {} (it was deleted by the change).", change.path)
                } else {
                    format!("Restored {} to its content before the change.", change.path)
                }
            })
        }
        .map_err(|_| SessionHostError::Ui(format!("{} cannot be restored", change.path)))?;
        return Ok(vec![
            done,
            "Written by Polaris outside the review flow; the session log keeps the original change."
                .to_owned(),
        ]);
    }
    Ok(vec![
        "Nothing to undo: no applied change of this session is still in place.".to_owned(),
    ])
}
