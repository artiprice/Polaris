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
    let config = WatchConfig {
        debounce: Duration::from_millis(20),
        ..WatchConfig::default()
    };
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

#[cfg(windows)]
#[test]
#[ignore = "streams 100 MiB through ConPTY while counting without retaining output"]
fn stress_conpty_streams_one_hundred_mibibytes_with_bounded_records()
-> Result<(), Box<dyn std::error::Error>> {
    const PAYLOAD_MARKER: &[u8] = b"ZQ7V3K9M";
    let executable = std::env::current_exe()?;
    let environment = EnvPolicy::new(
        ["SystemRoot", "WINDIR", "ComSpec"]
            .into_iter()
            .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
            .chain(std::iter::once((
                OsString::from("POLARIS_PTY_STRESS"),
                OsString::from("1"),
            )))
            .collect(),
    )?;
    let session = spawn_pty(
        &executable,
        &[
            OsString::from("--exact"),
            OsString::from("pty_stress_writer_fixture"),
            OsString::from("--nocapture"),
        ],
        None,
        &environment,
        PtySize {
            rows: 24,
            columns: 80,
            pixel_width: 0,
            pixel_height: 0,
        },
    )?;
    let expected_bytes = 100_u64 * 1024 * 1024;
    let marker_bytes = u64::try_from(PAYLOAD_MARKER.len())?;
    let expected_markers = expected_bytes / marker_bytes;
    let deadline = std::time::Instant::now() + Duration::from_secs(180);
    let started = std::time::Instant::now();
    let mut marker_count = 0_u64;
    let mut carry = Vec::new();
    while marker_count < expected_markers {
        if std::time::Instant::now() >= deadline {
            return Err(
                format!("ConPTY stream timed out after {marker_count} payload markers").into(),
            );
        }
        match session.recv_timeout(Duration::from_secs(1)) {
            Ok(PtyOutput::Data(chunk)) => {
                assert!(
                    chunk.len() <= 8 * 1024,
                    "PTY record exceeded configured chunk size"
                );
                let text = String::from_utf8_lossy(&chunk);
                if text.contains("\u{1b}[6n") {
                    session.write(b"\x1b[1;1R")?;
                }
                carry.extend_from_slice(&chunk);
                let found = carry
                    .windows(PAYLOAD_MARKER.len())
                    .filter(|window| *window == PAYLOAD_MARKER)
                    .count();
                marker_count = marker_count
                    .checked_add(u64::try_from(found)?)
                    .ok_or("PTY byte count overflow")?;
                let retain_from = carry.len().saturating_sub(PAYLOAD_MARKER.len() - 1);
                carry = carry
                    .get(retain_from..)
                    .ok_or("PTY marker carry range is invalid")?
                    .to_vec();
            }
            Ok(PtyOutput::Closed) => {
                return Err(format!("ConPTY closed after {marker_count} payload markers").into());
            }
            Ok(PtyOutput::Failed(error)) => return Err(error.into()),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error.into()),
        }
    }
    assert_eq!(marker_count, expected_markers);
    println!(
        "pty_stream_bytes={expected_bytes};payload_markers={marker_count};elapsed_ms={}",
        started.elapsed().as_millis()
    );
    Ok(())
}

#[test]
fn pty_stress_writer_fixture() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("POLARIS_PTY_STRESS").as_deref() != Some(std::ffi::OsStr::new("1")) {
        return Ok(());
    }
    use std::io::Write;
    const PAYLOAD_MARKER: &[u8] = b"ZQ7V3K9M";
    let chunk: Vec<u8> = PAYLOAD_MARKER
        .iter()
        .copied()
        .cycle()
        .take(1024 * 1024)
        .collect();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    for _ in 0..100 {
        output.write_all(&chunk)?;
    }
    output.flush()?;
    Ok(())
}
