//! Temp Dir Cleanup (descriptor: specs/descriptors/reaper_temp_dir_cleanup.json).

use std::error::Error;
use std::fs;
use std::path::Path;

use polaris_kernel_reaper::{Outcome, Reaper, ShutdownConfig, TEMP_DIR_PREFIX};

type TestResult = Result<(), Box<dyn Error>>;

fn temp_dir_with_files(parent: &Path) -> Result<tempfile::TempDir, Box<dyn Error>> {
    let dir = tempfile::tempdir_in(parent)?;
    fs::write(dir.path().join("top.txt"), b"top")?;
    let nested = dir.path().join("nested").join("deeper");
    fs::create_dir_all(&nested)?;
    fs::write(nested.join("leaf.bin"), b"leaf")?;
    Ok(dir)
}

#[tokio::test]
async fn registered_temp_dirs_are_deleted_on_shutdown() -> TestResult {
    let parent = tempfile::tempdir()?;
    let journal = tempfile::tempdir()?;
    let reaper = Reaper::new(journal.path())?;
    let mut registered = Vec::new();
    for _ in 0..5 {
        let dir = temp_dir_with_files(parent.path())?;
        let path = dir.path().to_path_buf();
        let id = reaper.register_temp_dir(dir)?;
        registered.push((id, path));
    }
    assert!(registered.iter().all(|(_, path)| path.exists()));

    let report = reaper.shutdown_all(&ShutdownConfig::default()).await?;

    for (id, path) in &registered {
        assert!(!path.exists(), "{} must be gone", path.display());
        assert_eq!(report.outcome(*id), Some(&Outcome::Released));
    }
    assert!(reaper.leak_report().entries.is_empty());
    Ok(())
}

#[tokio::test]
async fn created_temp_dir_has_the_reaper_prefix() -> TestResult {
    let parent = tempfile::tempdir()?;
    let journal = tempfile::tempdir()?;
    let reaper = Reaper::new(journal.path())?;

    let (id, path) = reaper.create_temp_dir(parent.path())?;

    assert!(path.is_dir());
    assert_eq!(path.parent(), Some(parent.path()));
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("temp dir name is not utf-8")?;
    assert!(name.starts_with(TEMP_DIR_PREFIX), "{name}");
    let report = reaper.shutdown_all(&ShutdownConfig::default()).await?;
    assert_eq!(report.outcome(id), Some(&Outcome::Released));
    assert!(!path.exists());
    Ok(())
}

#[test]
fn dropping_an_unshutdown_reaper_removes_registered_temp_dirs() -> TestResult {
    let parent = tempfile::tempdir()?;
    let journal = tempfile::tempdir()?;
    let dir = temp_dir_with_files(parent.path())?;
    let path = dir.path().to_path_buf();
    let reaper = Reaper::new(journal.path())?;
    reaper.register_temp_dir(dir)?;
    assert!(path.exists());

    drop(reaper);

    assert!(!path.exists());
    Ok(())
}

#[cfg(windows)]
#[tokio::test]
async fn temp_dir_with_a_held_file_is_reported_not_panicked() -> TestResult {
    use std::os::windows::fs::OpenOptionsExt;

    let parent = tempfile::tempdir()?;
    let journal = tempfile::tempdir()?;
    let reaper = Reaper::new(journal.path())?;

    let free_dir = temp_dir_with_files(parent.path())?;
    let free_path = free_dir.path().to_path_buf();
    let held_dir = temp_dir_with_files(parent.path())?;
    let held_path = held_dir.path().to_path_buf();
    let held_file = fs::OpenOptions::new()
        .write(true)
        .share_mode(0)
        .open(held_path.join("top.txt"))?;
    let free_id = reaper.register_temp_dir(free_dir)?;
    let held_id = reaper.register_temp_dir(held_dir)?;

    let report = reaper.shutdown_all(&ShutdownConfig::default()).await?;

    assert!(
        matches!(report.outcome(held_id), Some(Outcome::Failed(_))),
        "{:?}",
        report.outcome(held_id)
    );
    assert_eq!(report.outcome(free_id), Some(&Outcome::Released));
    assert!(held_path.exists());
    assert!(!free_path.exists());
    let leaked: Vec<_> = reaper.leak_report().entries.iter().map(|e| e.id).collect();
    assert_eq!(leaked, [held_id]);
    drop(held_file);
    Ok(())
}
