//! Real temporary-repository coverage for gix status, bounded git diff, worktrees, and CAS undo.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;

use polaris_vcs::{BranchName, CheckpointSeq, Repo, RepoPath, RevSpec};

type TestResult = Result<(), Box<dyn Error>>;

fn git(root: &Path, args: &[&str]) -> Result<(), Box<dyn Error>> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    if !output.status.success() {
        return Err(format!("git {:?}: {}", args, String::from_utf8_lossy(&output.stderr)).into());
    }
    Ok(())
}

fn repository() -> Result<tempfile::TempDir, Box<dyn Error>> {
    let tmp = tempfile::tempdir()?;
    git(tmp.path(), &["init", "-b", "main"])?;
    git(tmp.path(), &["config", "user.name", "Polaris Test"])?;
    git(tmp.path(), &["config", "user.email", "polaris-test@example.invalid"])?;
    fs::write(tmp.path().join("notes.txt"), b"first\r\nsecond\r\n")?;
    git(tmp.path(), &["add", "--", "notes.txt"])?;
    git(tmp.path(), &["commit", "-m", "initial"])?;
    Ok(tmp)
}

#[test]
fn gix_status_and_git_diff_report_real_worktree_changes() -> TestResult {
    let tmp = repository()?;
    fs::write(tmp.path().join("notes.txt"), b"first\r\nchanged\r\n")?;
    fs::write(tmp.path().join("new.txt"), b"untracked\n")?;
    let repo = Repo::open(tmp.path())?;
    let status = repo.status()?;
    let paths: Vec<&str> = status.entries.iter().map(|entry| entry.path.as_str()).collect();
    assert!(paths.contains(&"notes.txt"), "{paths:?}");
    assert!(paths.contains(&"new.txt"), "{paths:?}");
    assert!(!paths.iter().any(|path| path.starts_with(".polaris")), "{paths:?}");
    let diff = tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(repo.diff(&[]))?;
    assert!(diff.text().is_some_and(|text| text.contains("changed")));
    Ok(())
}

#[test]
fn undo_restores_crlf_bytes_and_refuses_intervening_edits() -> TestResult {
    let tmp = repository()?;
    let repo = Repo::open(tmp.path())?;
    let path = RepoPath::parse("notes.txt")?;
    let seq = CheckpointSeq::new(1);
    repo.snapshot_before_edit(seq, std::slice::from_ref(&path))?;
    fs::write(tmp.path().join("notes.txt"), b"agent edit\r\n")?;
    repo.seal_edit(seq)?;
    let undone = repo.undo()?;
    assert_eq!(undone.restored, vec![path.clone()]);
    assert_eq!(fs::read(tmp.path().join("notes.txt"))?, b"first\r\nsecond\r\n");

    let next = CheckpointSeq::new(2);
    repo.snapshot_before_edit(next, std::slice::from_ref(&path))?;
    fs::write(tmp.path().join("notes.txt"), b"agent second edit\r\n")?;
    repo.seal_edit(next)?;
    fs::write(tmp.path().join("notes.txt"), b"user intervening edit\r\n")?;
    let result = repo.undo();
    assert!(matches!(result, Err(polaris_vcs::VcsError::Conflict(_))));
    assert_eq!(fs::read(tmp.path().join("notes.txt"))?, b"user intervening edit\r\n");
    Ok(())
}

#[tokio::test]
async fn worktree_lifecycle_uses_validated_refs_and_isolated_checkout() -> TestResult {
    let tmp = repository()?;
    let repo = Repo::open(tmp.path())?;
    let branch = BranchName::parse("agent/frame-1")?;
    let base = RevSpec::parse("main")?;
    let worktree = repo.worktree_add(&branch, &base).await?;
    assert!(worktree.path().join("notes.txt").is_file());
    let dirty = worktree.mark_dirty();
    fs::write(dirty.path().join("agent.txt"), b"change")?;
    let removed = dirty.remove(true).await?;
    assert!(!removed.path().exists());
    Ok(())
}

#[test]
fn ref_validators_reject_options_traversal_and_windows_devices() -> TestResult {
    assert!(BranchName::parse("feature/cleanup").is_ok());
    for invalid in ["-c", "../escape", "x..y", "NUL", "feature/name.lock", "a b"] {
        assert!(BranchName::parse(invalid).is_err(), "{invalid}");
    }
    for invalid in ["-x", "HEAD..main", "C:\\escape", " ", "refs/heads/CON"] {
        assert!(RevSpec::parse(invalid).is_err(), "{invalid}");
    }
    assert!(RevSpec::parse("HEAD~2").is_ok());
    Ok(())
}
