//! Atomic Write With Retry (descriptor: specs/descriptors/cas_atomic_write_with_retry.json).

use std::error::Error;
use std::fs;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use polaris_storage_cas::atomic::{RetryPolicy, with_retry, write_atomic};

type TestResult = Result<(), Box<dyn Error>>;

/// The platform's "file in use" error: ERROR_SHARING_VIOLATION (32) on Windows, EBUSY (16) elsewhere.
fn busy() -> io::Error {
    io::Error::from_raw_os_error(if cfg!(windows) { 32 } else { 16 })
}

const FAST: RetryPolicy = RetryPolicy {
    attempts: 5,
    base_delay: Duration::from_millis(1),
    max_delay: Duration::from_millis(4),
};

#[test]
fn rename_race_is_success() -> TestResult {
    let dir = tempfile::tempdir()?;
    let dest = Arc::new(dir.path().join("blob"));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let dest = Arc::clone(&dest);
            std::thread::spawn(move || {
                write_atomic(&dest, b"identical content", &RetryPolicy::default())
            })
        })
        .collect();
    for h in handles {
        h.join().map_err(|_| "writer thread panicked")??;
    }
    assert_eq!(fs::read(dest.as_ref())?, b"identical content");
    let leftovers = fs::read_dir(dir.path())?.count();
    assert_eq!(leftovers, 1, "only the destination remains");
    Ok(())
}

#[test]
fn retry_on_sharing_violation() -> TestResult {
    let mut calls = 0;
    let got = with_retry(&FAST, || {
        calls += 1;
        if calls < 3 { Err(busy()) } else { Ok(calls) }
    })?;
    assert_eq!(got, 3);
    Ok(())
}

#[test]
fn gives_up_after_budget() {
    let mut calls = 0;
    let got: io::Result<()> = with_retry(&FAST, || {
        calls += 1;
        Err(busy())
    });
    assert!(got.is_err());
    assert_eq!(calls, FAST.attempts);
}

#[test]
fn non_retryable_error_is_immediate() {
    let mut calls = 0;
    let got: io::Result<()> = with_retry(&FAST, || {
        calls += 1;
        Err(io::Error::from(io::ErrorKind::NotFound))
    });
    assert!(got.is_err());
    assert_eq!(calls, 1);
}

#[test]
fn no_partial_file_on_failure() -> TestResult {
    let dir = tempfile::tempdir()?;
    let dest = dir.path().join("occupied");
    fs::create_dir(&dest)?;
    fs::write(dest.join("keep"), b"x")?;
    assert!(write_atomic(&dest, b"payload", &FAST).is_err());
    let names: Vec<_> = fs::read_dir(dir.path())?
        .filter_map(Result::ok)
        .map(|e| e.file_name())
        .collect();
    assert_eq!(
        names,
        vec![std::ffi::OsString::from("occupied")],
        "no temp file left behind"
    );
    Ok(())
}

#[cfg(windows)]
#[test]
fn windows_held_handle_is_retried() -> TestResult {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir()?;
    let dest = dir.path().join("held");
    fs::write(&dest, b"old")?;
    let held = fs::File::options().read(true).share_mode(0).open(&dest)?;
    // The lock is real: without retries the replace fails while the handle is held.
    let single = RetryPolicy {
        attempts: 1,
        ..FAST
    };
    assert!(
        write_atomic(&dest, b"blocked", &single).is_err(),
        "a held handle must block the rename"
    );
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        drop(held);
    });
    let policy = RetryPolicy {
        attempts: 40,
        base_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(50),
    };
    write_atomic(&dest, b"new", &policy)?;
    release.join().map_err(|_| "release thread panicked")?;
    assert_eq!(fs::read(&dest)?, b"new");
    Ok(())
}
