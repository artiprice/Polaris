//! Atomic ledger write (descriptor: specs/descriptors/rtk_atomic_ledger_write.json).

use std::error::Error;
use std::fs;
use std::time::Duration;

use jiff::Timestamp;
use polaris_storage_cas::atomic::RetryPolicy;
use polaris_tools_rtk::{Diagnostic, IssueId, Ledger, LedgerFile, LedgerLine, Level, RtkError};

type TestResult = Result<(), Box<dyn Error>>;

const T1: &str = "2026-09-22T11:04:00Z";

const FAST: RetryPolicy = RetryPolicy {
    attempts: 1,
    base_delay: Duration::from_millis(1),
    max_delay: Duration::from_millis(2),
};

fn error(message: &str) -> Diagnostic {
    Diagnostic::new(Level::Error, None, message)
}

fn ledger_with(message: &str) -> Result<Ledger, jiff::Error> {
    let mut ledger = Ledger::default();
    ledger.reconcile(&[error(message)], T1.parse::<Timestamp>()?);
    Ok(ledger)
}

#[test]
fn save_creates_directories_and_writes_lf() -> TestResult {
    let dir = tempfile::tempdir()?;
    let file = LedgerFile::new(dir.path().join(".polaris").join("issues.md"));
    let ledger = ledger_with("boom")?;
    file.save_atomic(&ledger)?;
    let on_disk = fs::read_to_string(file.path())?;
    assert_eq!(on_disk, ledger.render());
    assert!(!on_disk.contains('\r'));
    assert_eq!(file.load()?.render(), on_disk);
    Ok(())
}

#[cfg(windows)]
#[test]
fn interrupted_write_leaves_old_ledger() -> TestResult {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    let dir = tempfile::tempdir()?;
    let file = LedgerFile::new(dir.path().join("issues.md"));
    let old = ledger_with("old")?;
    file.save_atomic(&old)?;
    let new = ledger_with("new")?;

    let held = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(file.path())?;
    let attempt = file.save_atomic_with(&new, &FAST);
    drop(held);

    assert!(attempt.is_err(), "replacing a locked file must fail");
    assert_eq!(fs::read_to_string(file.path())?, old.render());
    assert_eq!(fs::read_dir(dir.path())?.count(), 1, "no temp file left");
    Ok(())
}

#[cfg(windows)]
#[test]
fn transient_permission_denied_is_retried() -> TestResult {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    use std::sync::mpsc;

    let dir = tempfile::tempdir()?;
    let file = LedgerFile::new(dir.path().join("issues.md"));
    file.save_atomic(&ledger_with("old")?)?;
    let new = ledger_with("new")?;

    let (locked_tx, locked_rx) = mpsc::channel();
    let path = file.path().to_path_buf();
    let holder = std::thread::spawn(move || -> std::io::Result<()> {
        let held = OpenOptions::new().read(true).share_mode(0).open(path)?;
        let _ = locked_tx.send(());
        std::thread::sleep(Duration::from_millis(150));
        drop(held);
        Ok(())
    });
    locked_rx.recv()?;
    let patient = RetryPolicy {
        attempts: 60,
        base_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(40),
    };
    file.save_atomic_with(&new, &patient)?;
    holder.join().map_err(|_| "lock holder panicked")??;
    assert_eq!(fs::read_to_string(file.path())?, new.render());
    Ok(())
}

#[test]
fn temp_file_removed_on_failure() -> TestResult {
    let dir = tempfile::tempdir()?;
    // A non-empty directory in the way: the rename can never succeed on any platform.
    let dest = dir.path().join("issues.md");
    fs::create_dir(&dest)?;
    fs::write(dest.join("inner"), b"x")?;
    let file = LedgerFile::new(&dest);
    let result = file.save_atomic_with(&ledger_with("boom")?, &FAST);
    assert!(result.is_err());
    let names: Vec<_> = fs::read_dir(dir.path())?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<Result<_, _>>()?;
    assert_eq!(names.len(), 1, "leftovers: {names:?}");
    Ok(())
}

#[test]
fn concurrent_writer_detected() -> TestResult {
    let dir = tempfile::tempdir()?;
    let file = LedgerFile::new(dir.path().join("issues.md"));
    file.save_atomic(&ledger_with("first")?)?;
    let (ledger, stamp) = file.load_stamped()?;

    fs::write(file.path(), "someone else edited this\n")?;
    let err = file
        .save_if_unchanged(&ledger, &stamp, &FAST)
        .err()
        .ok_or("expected a conflict")?;
    assert!(matches!(err, RtkError::ConcurrentWrite), "got {err:?}");
    assert_eq!(
        fs::read_to_string(file.path())?,
        "someone else edited this\n"
    );

    // Absent at read time, created by someone else before our write.
    let fresh = LedgerFile::new(dir.path().join("other.md"));
    let (ledger, stamp) = fresh.load_stamped()?;
    fs::write(fresh.path(), "appeared\n")?;
    let err = fresh
        .save_if_unchanged(&ledger, &stamp, &FAST)
        .err()
        .ok_or("expected a conflict")?;
    assert!(matches!(err, RtkError::ConcurrentWrite), "got {err:?}");
    Ok(())
}

#[test]
fn unchanged_file_passes_the_concurrency_check() -> TestResult {
    let dir = tempfile::tempdir()?;
    let file = LedgerFile::new(dir.path().join("issues.md"));
    let (empty, stamp) = file.load_stamped()?;
    file.save_if_unchanged(&empty, &stamp, &FAST)?;
    let (mut ledger, stamp) = file.load_stamped()?;
    ledger.reconcile(&[error("boom")], T1.parse::<Timestamp>()?);
    file.save_if_unchanged(&ledger, &stamp, &FAST)?;
    assert_eq!(file.load()?.render(), ledger.render());
    Ok(())
}

#[test]
fn stale_temp_from_a_crashed_run_is_ignored() -> TestResult {
    let dir = tempfile::tempdir()?;
    let stale = dir.path().join(".tmp-crashed");
    fs::write(&stale, b"half written garbage")?;
    let file = LedgerFile::new(dir.path().join("issues.md"));
    assert_eq!(file.load()?.render(), "");
    let ledger = ledger_with("boom")?;
    file.save_atomic(&ledger)?;
    assert_eq!(file.load()?.render(), ledger.render());
    assert_eq!(fs::read(&stale)?, b"half written garbage");
    Ok(())
}

#[test]
fn concurrent_updates_lose_no_history() -> TestResult {
    const THREADS: usize = 2;
    const ROUNDS: usize = 12;
    let dir = tempfile::tempdir()?;
    let file = LedgerFile::new(dir.path().join(".polaris").join("issues.md"));
    let now = T1.parse::<Timestamp>()?;

    std::thread::scope(|scope| -> TestResult {
        let handles: Vec<_> = (0..THREADS)
            .map(|t| {
                let file = &file;
                scope.spawn(move || -> Result<(), String> {
                    for i in 0..ROUNDS {
                        let d = error(&format!("thread {t} item {i}"));
                        file.update(&[d], now).map_err(|e| e.to_string())?;
                    }
                    Ok(())
                })
            })
            .collect();
        for h in handles {
            h.join().map_err(|_| "writer thread panicked")??;
        }
        Ok(())
    })?;

    let ledger = file.load()?;
    let ids: Vec<&IssueId> = ledger
        .lines()
        .iter()
        .filter_map(|l| match l {
            LedgerLine::Issue(i) => Some(&i.id),
            LedgerLine::Raw(_) => None,
        })
        .collect();
    for t in 0..THREADS {
        for i in 0..ROUNDS {
            let expected = IssueId::from_message(&format!("error: thread {t} item {i}"));
            assert!(ids.contains(&&expected), "lost issue thread {t} item {i}");
        }
    }
    assert_eq!(ledger.render().lines().count(), THREADS * ROUNDS);
    Ok(())
}
