//! Contract tests for executable resolution, environment policy, PTY and filesystem watching.

use std::ffi::OsString;
use std::time::Duration;

use polaris_platform_process::{
    EnvPolicy, FsEvent, PtyOutput, PtySize, WatchConfig, resolve_program, spawn_pty, watch,
};

#[test]
fn absolute_program_resolution_canonicalizes_and_flags_batch_targets()
-> Result<(), Box<dyn std::error::Error>> {
    let current = std::env::current_exe()?;
    let parent = current
        .parent()
        .ok_or("test executable has no parent directory")?;
    let resolved = resolve_program(current.as_os_str(), parent)?;
    assert!(resolved.path.is_absolute());
    assert!(!resolved.is_batch_script);
    assert!(resolve_program(std::ffi::OsStr::new("relative/path.exe"), parent).is_err());
    Ok(())
}

#[test]
fn environment_policy_rejects_secrets_and_duplicate_names() {
    assert!(
        EnvPolicy::new(vec![(
            OsString::from("SERVICE_TOKEN"),
            OsString::from("secret")
        )])
        .is_err()
    );
    assert!(
        EnvPolicy::new(vec![
            (OsString::from("Worker_Mode"), OsString::from("a")),
            (OsString::from("worker_mode"), OsString::from("b")),
        ])
        .is_err()
    );
    assert!(EnvPolicy::new(vec![(OsString::from("LANG"), OsString::from("C.UTF-8"))]).is_ok());
}

#[test]
fn watcher_emits_real_create_event_and_ignores_default_noise()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let mut config = WatchConfig::default();
    config.debounce = Duration::from_millis(20);
    let watcher = watch(directory.path(), config)?;
    std::fs::write(directory.path().join("visible.txt"), b"created")?;
    let event = watcher.recv_timeout(Duration::from_secs(3))?;
    assert!(
        matches!(event, Some(FsEvent::Changed(paths)) if paths.iter().any(|path| path.ends_with("visible.txt")))
    );
    std::fs::create_dir(directory.path().join(".git"))?;
    std::fs::write(directory.path().join(".git").join("hidden"), b"ignored")?;
    let ignored = watcher.recv_timeout(Duration::from_millis(200))?;
    assert!(ignored.is_none());
    Ok(())
}

#[cfg(windows)]
#[test]
fn conpty_starts_cmd_and_streams_output_with_bounded_records()
-> Result<(), Box<dyn std::error::Error>> {
    let comspec = std::env::var_os("COMSPEC").ok_or("COMSPEC is missing")?;
    let executable = std::path::PathBuf::from(comspec);
    let environment = EnvPolicy::new(
        ["SystemRoot", "WINDIR", "ComSpec"]
            .into_iter()
            .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
            .collect(),
    )?;
    let session = spawn_pty(
        &executable,
        &[OsString::from("/C"), OsString::from("echo pty-probe")],
        None,
        &environment,
        PtySize {
            rows: 24,
            columns: 80,
            pixel_width: 0,
            pixel_height: 0,
        },
    )?;
    let mut all = Vec::new();
    match session.recv_timeout(Duration::from_secs(3))? {
        PtyOutput::Data(chunk) => {
            let probe = String::from_utf8_lossy(&chunk);
            if probe.contains("\u{1b}[6n") {
                session.write(b"\x1b[1;1R")?;
            }
            all.extend_from_slice(&chunk);
        }
        PtyOutput::Closed => return Err("PTY closed before writing output".into()),
        PtyOutput::Failed(error) => return Err(error.into()),
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let text = String::from_utf8_lossy(&all);
        if text.contains("pty-probe") {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("PTY output did not contain command output: {text:?}").into());
        }
        match session.recv_timeout(Duration::from_millis(100)) {
            Ok(PtyOutput::Data(chunk)) => all.extend_from_slice(&chunk),
            Ok(PtyOutput::Closed) => break,
            Ok(PtyOutput::Failed(error)) => return Err(error.into()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if session.try_wait()?.is_some() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err("PTY child did not exit after echo".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}

#[test]
fn invalid_pty_dimensions_are_rejected_before_native_allocation() {
    let result = spawn_pty(
        std::path::Path::new("relative"),
        &[],
        None,
        &EnvPolicy::default(),
        PtySize {
            rows: 0,
            columns: 80,
            pixel_width: 0,
            pixel_height: 0,
        },
    );
    assert!(result.is_err());
}
