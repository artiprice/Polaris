// Unit tests of apps/polaris-cli/src/ui_services.rs (`tests`); compiled into that module via #[path].
use super::{
    ClipboardService, IndexLimits, MAX_CLIPBOARD_TEXT_BYTES, ServiceError, WorkspaceFileIndex,
    take_child_stdout,
};
use std::{
    fs,
    path::PathBuf,
    process::Stdio,
    thread,
    time::{Duration, Instant},
};

#[test]
fn clipboard_text_limit_is_enforced_without_touching_the_system_clipboard()
-> Result<(), ServiceError> {
    let too_large = "x".repeat(MAX_CLIPBOARD_TEXT_BYTES.saturating_add(1));
    let mut service = ClipboardService::new(Duration::from_secs(1), MAX_CLIPBOARD_TEXT_BYTES)?;
    assert_eq!(
        service.request_copy(10, too_large),
        Err(ServiceError::TextTooLarge),
    );
    service.shutdown();
    Ok(())
}

#[test]
fn empty_clipboard_copy_is_rejected_without_native_access() -> Result<(), ServiceError> {
    let mut service = ClipboardService::new(Duration::from_secs(1), MAX_CLIPBOARD_TEXT_BYTES)?;
    assert_eq!(
        service.request_copy(14, String::new()),
        Err(ServiceError::InvalidInput)
    );
    assert!(service.try_recv().is_none());
    service.shutdown();
    Ok(())
}

#[test]
fn missing_clipboard_child_pipe_is_killed_and_reaped() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    let mut command = {
        let system_root = std::env::var_os("SystemRoot").ok_or(ServiceError::Unavailable)?;
        let executable = PathBuf::from(system_root).join("System32").join("ping.exe");
        let mut command = std::process::Command::new(executable);
        command.args(["-n", "60", "127.0.0.1"]);
        command
    };
    #[cfg(unix)]
    let mut command = {
        let mut command = std::process::Command::new("sleep");
        command.arg("60");
        command
    };
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let _pipe = child.stdout.take();
    assert!(matches!(
        take_child_stdout(&mut child),
        Err(ServiceError::Unavailable)
    ));
    assert!(child.try_wait()?.is_some());
    Ok(())
}

#[test]
fn clipboard_generation_can_be_cancelled_without_native_access() -> Result<(), ServiceError> {
    let mut service = ClipboardService::new(Duration::from_secs(1), MAX_CLIPBOARD_TEXT_BYTES)?;
    service.cancel(11);
    assert_eq!(service.try_recv(), None);
    service.shutdown();
    Ok(())
}

#[test]
fn file_index_respects_builtin_ignore_and_secret_path_rules()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    fs::create_dir_all(workspace.path().join("src"))?;
    fs::create_dir_all(workspace.path().join(".aws"))?;
    fs::create_dir_all(workspace.path().join(".git"))?;
    fs::write(workspace.path().join(".gitignore"), "ignored.txt\n")?;
    fs::write(
        workspace.path().join(".polarisignore"),
        "ignored-local.txt\n",
    )?;
    fs::write(workspace.path().join("src/main.rs"), "fn main() {}")?;
    fs::write(workspace.path().join("ignored.txt"), "private")?;
    fs::write(workspace.path().join("ignored-local.txt"), "private")?;
    fs::write(workspace.path().join(".aws/credentials"), "private")?;
    let mut service = WorkspaceFileIndex::new(
        workspace.path().to_path_buf(),
        IndexLimits {
            max_paths: 8,
            max_snapshot_bytes: 1024,
            timeout: Duration::from_secs(2),
        },
    )?;
    service.request(12)?;
    let started = Instant::now();
    loop {
        if let Some(result) = service.try_recv() {
            let snapshot = result.result?;
            assert_eq!(snapshot.generation, 12);
            assert!(snapshot.paths.iter().any(|path| path == "src/main.rs"));
            assert!(!snapshot.paths.iter().any(|path| path == "ignored.txt"));
            assert!(
                !snapshot
                    .paths
                    .iter()
                    .any(|path| path == "ignored-local.txt")
            );
            assert!(!snapshot.paths.iter().any(|path| path.starts_with(".aws/")));
            break;
        }
        if started.elapsed() > Duration::from_secs(3) {
            return Err("file index timed out".into());
        }
        thread::sleep(Duration::from_millis(5));
    }
    service.shutdown();
    Ok(())
}

#[test]
fn file_index_enforces_path_count_bound() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = tempfile::tempdir()?;
    for name in ["a.txt", "b.txt", "c.txt"] {
        fs::write(workspace.path().join(name), "x")?;
    }
    let mut service = WorkspaceFileIndex::new(
        workspace.path().to_path_buf(),
        IndexLimits {
            max_paths: 1,
            max_snapshot_bytes: 1024,
            timeout: Duration::from_secs(2),
        },
    )?;
    service.request(13)?;
    let started = Instant::now();
    loop {
        if let Some(result) = service.try_recv() {
            let snapshot = result.result?;
            assert_eq!(snapshot.paths.len(), 1);
            assert!(snapshot.truncated);
            break;
        }
        if started.elapsed() > Duration::from_secs(3) {
            return Err("file index timed out".into());
        }
        thread::sleep(Duration::from_millis(5));
    }
    service.shutdown();
    Ok(())
}

#[test]
fn runtime_preference_override_is_memory_only() {
    let base = polaris_config::CliPreferences::default();
    let overrides = super::PreferenceOverrides {
        ascii: Some(true),
        ..super::PreferenceOverrides::default()
    };
    let effective = overrides.apply(base);
    assert!(effective.ascii);
    assert!(!base.ascii);
}
