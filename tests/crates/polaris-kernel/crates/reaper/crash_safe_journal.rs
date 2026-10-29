//! Crash Safe Journal (descriptor: specs/descriptors/reaper_crash_safe_journal.json).
//!
//! A "crash" is simulated with `std::mem::forget`: the first reaper never runs `Drop`, so its
//! journal and its temp dirs are left behind exactly as a killed process would leave them.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use polaris_kernel_reaper::{
    Disposition, JOURNAL_FILE_NAME, Locator, OrphanAction, Outcome, Reaper, ReaperError,
    ResourceKind, ShutdownConfig, TEMP_DIR_PREFIX, Teardown, TeardownContext, TeardownError,
};

type TestResult = Result<(), Box<dyn Error>>;

const HEADER: &str = "polaris-reaper-journal v1";

struct Noop;

impl Teardown for Noop {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        Ok(Disposition::Clean)
    }
}

struct Stuck;

impl Teardown for Stuck {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        Err(TeardownError::Other("still busy".to_owned()))
    }
}

fn journal_path(dir: &Path) -> PathBuf {
    dir.join(JOURNAL_FILE_NAME)
}

fn read_journal(dir: &Path) -> Result<String, Box<dyn Error>> {
    Ok(fs::read_to_string(journal_path(dir))?)
}

/// The journal's text escaping, restated here so tests can hand-write records.
fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn temp_dir_record(id: u64, path: &Path) -> String {
    format!(
        "R\t{id}\ttemp_dir\tpath\t{}\n",
        escape(&path.to_string_lossy())
    )
}

#[test]
fn register_writes_and_release_removes_the_journal_record() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    assert!(!journal_path(dir.path()).exists());

    let id = reaper.register_journaled(
        ResourceKind::Sandbox,
        Locator::Label("sbx-0007".to_owned()),
        Noop,
    )?;
    let text = read_journal(dir.path())?;
    assert!(text.starts_with(HEADER), "{text}");
    assert!(text.contains("sandbox"), "{text}");
    assert!(text.contains("sbx-0007"), "{text}");

    reaper.release(id)?;
    assert!(!journal_path(dir.path()).exists());
    Ok(())
}

#[tokio::test]
async fn clean_shutdown_clears_the_journal() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    reaper.register_journaled(ResourceKind::Sandbox, Locator::Label("a".to_owned()), Noop)?;
    reaper.register_journaled(ResourceKind::Sandbox, Locator::Label("b".to_owned()), Noop)?;
    assert!(journal_path(dir.path()).exists());

    let report = reaper.shutdown_all(&ShutdownConfig::default()).await?;

    assert!(report.journal_error.is_none());
    assert!(!journal_path(dir.path()).exists());
    Ok(())
}

#[tokio::test]
async fn failed_resource_stays_journaled_after_shutdown() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    reaper.register_journaled(
        ResourceKind::Sandbox,
        Locator::Label("fine-one".to_owned()),
        Noop,
    )?;
    let stuck = reaper.register_journaled(
        ResourceKind::Sandbox,
        Locator::Label("stuck-one".to_owned()),
        Stuck,
    )?;

    let report = reaper.shutdown_all(&ShutdownConfig::default()).await?;

    assert!(matches!(report.outcome(stuck), Some(Outcome::Failed(_))));
    let text = read_journal(dir.path())?;
    assert!(text.contains("stuck-one"), "{text}");
    assert!(!text.contains("fine-one"), "{text}");
    Ok(())
}

#[tokio::test]
async fn orphaned_reaper_temp_dir_is_removed_by_the_next_start() -> TestResult {
    let parent = tempfile::tempdir()?;
    let journal = tempfile::tempdir()?;
    let crashed = Reaper::new(journal.path())?;
    let (_, leftover) = crashed.create_temp_dir(parent.path())?;
    fs::write(leftover.join("scratch.txt"), b"x")?;
    std::mem::forget(crashed);
    assert!(leftover.exists());

    let next = Reaper::new(journal.path())?;
    let sweep = next.sweep_orphans().await?;

    assert_eq!(sweep.orphans.len(), 1);
    let orphan = sweep.orphans.first().ok_or("no orphan reported")?;
    assert_eq!(orphan.kind, ResourceKind::TempDir);
    assert_eq!(orphan.locator, Locator::Path(leftover.clone()));
    assert_eq!(orphan.action, OrphanAction::Removed);
    assert_eq!(sweep.ignored, 0);
    assert!(!leftover.exists());
    assert!(!journal_path(journal.path()).exists());
    Ok(())
}

#[tokio::test]
async fn truncated_last_record_is_ignored() -> TestResult {
    let journal = tempfile::tempdir()?;
    let crashed = Reaper::new(journal.path())?;
    crashed.register_journaled(
        ResourceKind::Sandbox,
        Locator::Label("alpha".to_owned()),
        Noop,
    )?;
    crashed.register_journaled(
        ResourceKind::Sandbox,
        Locator::Label("beta".to_owned()),
        Noop,
    )?;
    std::mem::forget(crashed);
    let text = read_journal(journal.path())?;
    // Cut inside the last record: "\tbeta\n" are the final six bytes.
    let cut = text.len().saturating_sub("\tbeta\n".len());
    fs::write(
        journal_path(journal.path()),
        text.get(..cut).ok_or("bad cut")?,
    )?;

    let next = Reaper::new(journal.path())?;
    let sweep = next.sweep_orphans().await?;

    let locators: Vec<&Locator> = sweep.orphans.iter().map(|o| &o.locator).collect();
    assert_eq!(locators, [&Locator::Label("alpha".to_owned())]);
    assert_eq!(sweep.ignored, 1);
    Ok(())
}

#[tokio::test]
async fn duplicated_record_is_reported_once() -> TestResult {
    let journal = tempfile::tempdir()?;
    let record = "R\t5\tsandbox\tlabel\tsbx-dup\n";
    fs::write(
        journal_path(journal.path()),
        format!("{HEADER}\n{record}{record}"),
    )?;

    let next = Reaper::new(journal.path())?;
    let sweep = next.sweep_orphans().await?;

    assert_eq!(sweep.orphans.len(), 1);
    assert_eq!(sweep.ignored, 0);
    Ok(())
}

#[tokio::test]
async fn path_outside_the_reaper_prefix_is_never_removed() -> TestResult {
    let parent = tempfile::tempdir()?;
    let journal = tempfile::tempdir()?;
    let victim = parent.path().join("precious-user-data");
    fs::create_dir(&victim)?;
    fs::write(victim.join("keep.txt"), b"keep")?;
    assert!(!TEMP_DIR_PREFIX.is_empty());
    fs::write(
        journal_path(journal.path()),
        format!("{HEADER}\n{}", temp_dir_record(1, &victim)),
    )?;

    let next = Reaper::new(journal.path())?;
    let sweep = next.sweep_orphans().await?;

    let orphan = sweep.orphans.first().ok_or("no orphan reported")?;
    assert_eq!(orphan.action, OrphanAction::NeedsOperator);
    assert!(victim.join("keep.txt").exists());
    Ok(())
}

#[tokio::test]
async fn separators_in_labels_cannot_forge_records() -> TestResult {
    let parent = tempfile::tempdir()?;
    let journal = tempfile::tempdir()?;
    let victim = parent.path().join(format!("{TEMP_DIR_PREFIX}victim"));
    fs::create_dir(&victim)?;
    let hostile = format!(
        "x\nR\t99\ttemp_dir\tpath\t{}\r\n\\",
        victim.to_string_lossy()
    );
    let crashed = Reaper::new(journal.path())?;
    crashed.register_journaled(ResourceKind::Sandbox, Locator::Label(hostile.clone()), Noop)?;
    std::mem::forget(crashed);
    let text = read_journal(journal.path())?;
    assert_eq!(
        text.lines().count(),
        2,
        "header + exactly one record: {text}"
    );

    let next = Reaper::new(journal.path())?;
    let sweep = next.sweep_orphans().await?;

    assert_eq!(sweep.orphans.len(), 1);
    let orphan = sweep.orphans.first().ok_or("no orphan reported")?;
    assert_eq!(orphan.locator, Locator::Label(hostile));
    assert!(
        victim.exists(),
        "the forged record must not delete anything"
    );
    Ok(())
}

#[tokio::test]
async fn pid_orphans_are_listed_never_killed() -> TestResult {
    let journal = tempfile::tempdir()?;
    fs::write(
        journal_path(journal.path()),
        format!("{HEADER}\nR\t3\tprocess\tpid\t4242\nR\t4\tsandbox\tlabel\tsbx-9\n"),
    )?;

    let next = Reaper::new(journal.path())?;
    let sweep = next.sweep_orphans().await?;

    assert_eq!(sweep.orphans.len(), 2);
    assert!(
        sweep
            .orphans
            .iter()
            .all(|o| o.action == OrphanAction::NeedsOperator)
    );
    let pid_orphan = sweep
        .orphans
        .iter()
        .find(|o| o.locator == Locator::Pid(4242))
        .ok_or("pid orphan missing")?;
    assert_eq!(pid_orphan.kind, ResourceKind::Process);
    assert!(read_journal(journal.path())?.contains("4242"));
    // New registrations must not reuse the ids of unresolved orphans.
    let fresh = next.register_journaled(
        ResourceKind::Sandbox,
        Locator::Label("fresh".to_owned()),
        Noop,
    )?;
    assert!(sweep.orphans.iter().all(|o| o.id != fresh));

    for orphan in &sweep.orphans {
        next.acknowledge_orphan(orphan.id)?;
    }
    let text = read_journal(journal.path())?;
    assert!(!text.contains("4242"), "{text}");
    assert!(!text.contains("sbx-9"), "{text}");
    assert!(text.contains("fresh"), "{text}");
    assert!(matches!(
        next.acknowledge_orphan(pid_orphan.id),
        Err(ReaperError::UnknownResource)
    ));
    Ok(())
}

#[test]
fn unknown_journal_version_is_refused() -> TestResult {
    let journal = tempfile::tempdir()?;
    let original = "polaris-reaper-journal v9\nR\t1\tsandbox\tlabel\tx\n";
    fs::write(journal_path(journal.path()), original)?;

    let refused = Reaper::new(journal.path());

    assert!(matches!(refused, Err(ReaperError::JournalFormat(_))));
    assert_eq!(read_journal(journal.path())?, original);
    Ok(())
}
