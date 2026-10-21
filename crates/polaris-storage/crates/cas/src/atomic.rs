//! Atomic writes (temp file in the same directory, fsync, rename) with a bounded retry for files
//! held by other processes — on Windows typically antivirus scanners or indexers (pitfall W5).

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

/// Exponential backoff budget for "file in use" errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts, including the first (0 behaves like 1).
    pub attempts: u32,
    /// Delay after the first failure; doubles each time.
    pub base_delay: Duration,
    /// Upper bound for one delay.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    /// 8 attempts, 10 ms doubling to at most 500 ms (about 1.8 s in total).
    fn default() -> Self {
        Self {
            attempts: 8,
            base_delay: Duration::from_millis(10),
            max_delay: Duration::from_millis(500),
        }
    }
}

/// `true` for "file is in use" errors worth retrying: Windows `ERROR_ACCESS_DENIED` (5),
/// `ERROR_SHARING_VIOLATION` (32), `ERROR_LOCK_VIOLATION` (33); `EBUSY` (16) and `ETXTBSY` (26)
/// elsewhere.
#[must_use]
pub fn is_retryable(error: &io::Error) -> bool {
    match error.raw_os_error() {
        Some(code) if cfg!(windows) => matches!(code, 5 | 32 | 33),
        Some(code) => matches!(code, 16 | 26),
        None => false,
    }
}

/// Backoff state shared by [`with_retry`] and [`write_atomic`].
struct Backoff {
    policy: RetryPolicy,
    attempt: u32,
    delay: Duration,
}

impl Backoff {
    const fn new(policy: &RetryPolicy) -> Self {
        Self {
            policy: *policy,
            attempt: 1,
            delay: policy.base_delay,
        }
    }

    /// Sleeps and returns `true` if `error` is retryable and the budget allows another attempt.
    fn retry_after(&mut self, error: &io::Error) -> bool {
        if !is_retryable(error) || self.attempt >= self.policy.attempts {
            return false;
        }
        std::thread::sleep(self.delay);
        self.delay = self.delay.saturating_mul(2).min(self.policy.max_delay);
        self.attempt = self.attempt.saturating_add(1);
        true
    }
}

/// Runs `op`, retrying retryable errors per `policy`.
///
/// # Errors
/// The last error of `op`: immediately for non-retryable errors, after the budget otherwise.
pub fn with_retry<T>(policy: &RetryPolicy, mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut backoff = Backoff::new(policy);
    loop {
        match op() {
            Ok(value) => return Ok(value),
            Err(error) if backoff.retry_after(&error) => {}
            Err(error) => return Err(error),
        }
    }
}

/// Writes `bytes` to `dest` atomically: readers see the old file or the complete new one, never
/// a partial file; on failure the temp file is removed.
///
/// # Errors
/// I/O errors creating, writing or syncing the temp file, or the final rename error after the
/// retry budget.
pub fn write_atomic(dest: &Path, bytes: &[u8], policy: &RetryPolicy) -> io::Result<()> {
    let dir = dest
        .parent()
        .ok_or_else(|| io::Error::other("destination has no parent directory"))?;
    let mut tmp = tempfile::Builder::new().prefix(".tmp-").tempfile_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    let mut backoff = Backoff::new(policy);
    let mut pending = tmp;
    loop {
        match pending.persist(dest) {
            Ok(_) => return Ok(()),
            Err(failed) if backoff.retry_after(&failed.error) => pending = failed.file,
            Err(failed) => return Err(failed.error),
        }
    }
}
