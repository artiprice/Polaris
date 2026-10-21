//! File Rotation (descriptor: specs/descriptors/log_file_rotation.json).

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use polaris_log::{LOG_FILE_NAME, LogConfig, LogError, LogGuard, start};
use polaris_sdk::registry::{RedactStats, Redactor};
use tracing::Dispatch;

type TestResult = Result<(), Box<dyn Error>>;

const FLUSH: Duration = Duration::from_secs(20);

/// Passes text through, masking one fixed fake secret.
struct Mask(Vec<u8>);

impl Redactor for Mask {
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
        let mut rest = input;
        let mut replacements = 0u32;
        while let Some(pos) = rest
            .windows(self.0.len())
            .position(|w| w == self.0.as_slice())
        {
            out.extend_from_slice(rest.get(..pos).unwrap_or_default());
            out.extend_from_slice(b"[REDACTED]");
            rest = rest.get(pos + self.0.len()..).unwrap_or_default();
            replacements += 1;
        }
        out.extend_from_slice(rest);
        RedactStats {
            bytes_in: input.len(),
            replacements,
        }
    }
}

fn config(dir: &Path, rotate_bytes: u64, keep_files: u16) -> LogConfig {
    let mut cfg = LogConfig::new(dir);
    cfg.level = "info".to_string();
    cfg.rotate_bytes = rotate_bytes;
    cfg.keep_files = keep_files;
    cfg.tui_mode = true;
    cfg
}

fn begin(cfg: LogConfig) -> Result<(LogGuard, Dispatch), LogError> {
    start(cfg, Mask(b"sk-fake-rotation-secret".to_vec()))
}

/// Log files of `dir`, oldest first (`polaris_N.log` with the highest N first, `polaris.log` last).
fn log_files(dir: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files: Vec<(u32, PathBuf)> = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("non-utf8 name")?
            .to_string();
        if name == LOG_FILE_NAME {
            files.push((0, path));
        } else if let Some(n) = name
            .strip_prefix("polaris_")
            .and_then(|r| r.strip_suffix(".log"))
            .and_then(|n| n.parse::<u32>().ok())
        {
            files.push((n, path));
        }
    }
    files.sort_by_key(|a| std::cmp::Reverse(a.0));
    Ok(files.into_iter().map(|(_, p)| p).collect())
}

fn emit_numbered(dispatch: &Dispatch, count: usize) {
    tracing::dispatcher::with_default(dispatch, || {
        for i in 0..count {
            tracing::info!("line-{i:04} padding padding padding padding padding padding");
        }
    });
}

#[test]
fn rotates_when_size_exceeded() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (guard, dispatch) = begin(config(dir.path(), 400, 50))?;
    emit_numbered(&dispatch, 10);
    assert!(guard.flush(FLUSH));
    let files = log_files(dir.path())?;
    assert!(files.len() > 1, "expected several files, got {files:?}");
    for f in &files {
        assert!(fs::metadata(f)?.len() <= 400, "{f:?} exceeds the limit");
    }
    Ok(())
}

#[test]
fn keeps_exactly_keep_files() -> TestResult {
    for keep in [2u16, 3] {
        let dir = tempfile::tempdir()?;
        let (guard, dispatch) = begin(config(dir.path(), 300, keep))?;
        emit_numbered(&dispatch, 30);
        assert!(guard.flush(FLUSH));
        assert_eq!(
            log_files(dir.path())?.len(),
            usize::from(keep),
            "keep={keep}"
        );
        drop(guard);
        assert_eq!(
            log_files(dir.path())?.len(),
            usize::from(keep),
            "after drop"
        );
    }
    Ok(())
}

#[test]
fn lines_survive_in_order_across_files() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (guard, dispatch) = begin(config(dir.path(), 500, 100))?;
    emit_numbered(&dispatch, 40);
    drop(guard);
    let mut text = String::new();
    for f in log_files(dir.path())? {
        text.push_str(&fs::read_to_string(f)?);
    }
    let numbers: Vec<usize> = text
        .lines()
        .filter_map(|l| l.split("line-").nth(1))
        .filter_map(|r| r.get(..4))
        .filter_map(|n| n.parse().ok())
        .collect();
    assert_eq!(numbers, (0..40).collect::<Vec<_>>());
    Ok(())
}

#[test]
fn file_lines_are_redacted() -> TestResult {
    let dir = tempfile::tempdir()?;
    let (guard, dispatch) = begin(config(dir.path(), 10_000, 3))?;
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(
            key = "sk-fake-rotation-secret",
            "seeded sk-fake-rotation-secret"
        );
    });
    drop(guard);
    let text = fs::read_to_string(dir.path().join(LOG_FILE_NAME))?;
    assert!(text.contains("[REDACTED]"));
    assert!(!text.contains("sk-fake-rotation-secret"));
    Ok(())
}

#[test]
fn json_mode_writes_one_object_per_line() -> TestResult {
    let dir = tempfile::tempdir()?;
    let mut cfg = config(dir.path(), 10_000, 3);
    cfg.json = true;
    let (guard, dispatch) = begin(cfg)?;
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(user = "alice", "first");
        tracing::warn!("second");
    });
    drop(guard);
    let text = fs::read_to_string(dir.path().join(LOG_FILE_NAME))?;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    for l in &lines {
        assert!(l.starts_with(r#"{"ts":""#) && l.ends_with("}}"), "{l}");
    }
    assert!(
        lines
            .first()
            .ok_or("none")?
            .contains(r#""fields":{"message":"first","user":"alice"}"#)
    );
    assert!(lines.get(1).ok_or("none")?.contains(r#""level":"WARN""#));
    Ok(())
}

#[test]
fn unusable_log_dir_is_a_sink_init_error() -> TestResult {
    let dir = tempfile::tempdir()?;
    let blocker = dir.path().join("not-a-dir");
    fs::write(&blocker, b"file in the way")?;
    let result = begin(config(&blocker.join("logs"), 1000, 3));
    assert!(
        matches!(result, Err(LogError::SinkInit(_))),
        "{:?}",
        result.map(|_| ())
    );
    Ok(())
}

#[test]
fn rotation_failure_is_counted_and_producers_continue() -> TestResult {
    let dir = tempfile::tempdir()?;
    // A directory squats on the first rotated name, so every rotation fails.
    fs::create_dir(dir.path().join("polaris_1.log"))?;
    let (guard, dispatch) = begin(config(dir.path(), 200, 2))?;
    let start = Instant::now();
    emit_numbered(&dispatch, 200);
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "producers were slowed"
    );
    assert!(guard.flush(Duration::from_secs(60)));
    let stats = guard.stats();
    assert!(stats.sink_errors > 0, "{stats:?}");
    assert!(guard.last_sink_error().is_some());
    assert_eq!(guard.tui_ring().len(), 200.min(guard.tui_ring().capacity()));
    drop(guard);
    Ok(())
}

#[test]
fn overflow_policy_accounts_every_record() -> TestResult {
    let dir = tempfile::tempdir()?;
    let mut cfg = config(dir.path(), 50_000_000, 2);
    cfg.queue_capacity = 16;
    let (guard, dispatch) = begin(cfg)?;
    let dispatch = Arc::new(dispatch);
    let threads = 8usize;
    let per_thread = 2_000usize;
    let mut handles = Vec::new();
    for t in 0..threads {
        let dispatch = Arc::clone(&dispatch);
        handles.push(std::thread::spawn(move || {
            let mut worst = Duration::ZERO;
            tracing::dispatcher::with_default(&dispatch, || {
                for i in 0..per_thread {
                    let begin = Instant::now();
                    tracing::info!(t, i, "load line");
                    worst = worst.max(begin.elapsed());
                }
            });
            worst
        }));
    }
    let mut worst = Duration::ZERO;
    for h in handles {
        worst = worst.max(h.join().map_err(|_| "producer panicked")?);
    }
    assert!(guard.flush(Duration::from_secs(60)));
    let stats = guard.stats();
    let total = (threads * per_thread) as u64;
    assert_eq!(
        stats.queue_accepted + stats.queue_dropped,
        total,
        "{stats:?}"
    );
    let written = fs::read_to_string(dir.path().join(LOG_FILE_NAME))?
        .lines()
        .count() as u64;
    assert_eq!(
        written, stats.queue_accepted,
        "every accepted record reached the file"
    );
    eprintln!(
        "load: worst single-call latency {worst:?}, dropped {}",
        stats.queue_dropped
    );
    drop(guard);
    Ok(())
}
