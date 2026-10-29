use polaris_runtime_research::{
    Budget, Decision, Direction, ExperimentWorkspace, ResearchState, Usage, VcsWorkspace,
};
use polaris_sdk::ids::Cid;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

fn temp_root() -> PathBuf {
    std::env::temp_dir().join(format!("polaris-vcs-research-{}", std::process::id()))
}

fn git(root: &Path, args: &[&str]) -> TestResult<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()?;
    if !status.success() {
        return Err(format!("git command failed: {args:?}").into());
    }
    Ok(())
}

fn repository(root: &Path) -> TestResult<()> {
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("src/value.txt"), "baseline\n")?;
    git(root, &["init", "--quiet"])?;
    git(root, &["config", "user.name", "Research Fixture"])?;
    git(
        root,
        &["config", "user.email", "research-fixture@example.invalid"],
    )?;
    git(root, &["add", "src/value.txt"])?;
    git(root, &["commit", "--quiet", "-m", "baseline"])?;
    Ok(())
}

fn experiment() -> TestResult<polaris_runtime_research::ExperimentId> {
    let mut state = ResearchState::new(
        Cid::from_bytes([1; 32]),
        Direction::Min,
        Budget::new(100, 100, 100, 20)?,
    )?;
    state.begin(Usage::new(5, 5, 5)).map_err(Into::into)
}

#[test]
fn vcs_keep_and_revert_return_distinct_receipts() -> TestResult<()> {
    let root = temp_root();
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    repository(&root)?;
    let workspace = VcsWorkspace::open(&root)?;
    let path = vec!["src/value.txt".to_owned()];

    let first = experiment()?;
    workspace.begin(first, &path)?;
    fs::write(root.join("src/value.txt"), "kept\n")?;
    let keep_receipt = workspace.settle(first, Decision::Keep)?;
    assert_eq!(
        polaris_storage_cas::content_id(&keep_receipt.payload),
        keep_receipt.cid
    );
    assert_eq!(fs::read_to_string(root.join("src/value.txt"))?, "kept\n");

    let second = experiment_with_ordinal(2)?;
    workspace.begin(second, &path)?;
    fs::write(root.join("src/value.txt"), "discarded\n")?;
    let revert_receipt = workspace.settle(second, Decision::Revert)?;
    assert_eq!(
        polaris_storage_cas::content_id(&revert_receipt.payload),
        revert_receipt.cid
    );
    assert_eq!(fs::read_to_string(root.join("src/value.txt"))?, "kept\n");
    assert_ne!(keep_receipt.cid, revert_receipt.cid);

    drop(workspace);
    fs::remove_dir_all(root)?;
    Ok(())
}

fn experiment_with_ordinal(ordinal: u32) -> TestResult<polaris_runtime_research::ExperimentId> {
    let mut state = ResearchState::new(
        Cid::from_bytes([1; 32]),
        Direction::Min,
        Budget::new(100, 100, 100, 20)?,
    )?;
    let mut id = state.begin(Usage::new(5, 5, 5))?;
    for _ in 1..ordinal {
        state.interrupt(id)?;
        state.recovered(id, Cid::from_bytes([2; 32]))?;
        id = state.begin(Usage::new(5, 5, 5))?;
    }
    Ok(id)
}
