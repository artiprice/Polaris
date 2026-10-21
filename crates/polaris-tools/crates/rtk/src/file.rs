//! The ledger file: path resolution, load, atomic save, concurrent-writer protection.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use jiff::Timestamp;
use polaris_sdk::ids::Cid;
use polaris_storage_cas::atomic::{RetryPolicy, with_retry, write_atomic};
use polaris_storage_cas::content_id;

use crate::diag::Diagnostic;
use crate::error::RtkError;
use crate::ledger::{Ledger, LedgerSummary};

/// Environment variable that overrides the ledger location (proposed name; the reference engine
/// uses `HARNESS_ISSUES_FILE`).
pub const ISSUES_FILE_ENV: &str = "POLARIS_ISSUES_FILE";

/// Ledger path relative to the workspace root.
const DEFAULT_RELATIVE: [&str; 2] = [".polaris", "issues.md"];

/// How long [`LedgerFile::update`] waits for the advisory lock: `LOCK_ATTEMPTS` polls of
/// `LOCK_POLL` (about 5 seconds).
const LOCK_ATTEMPTS: u32 = 1000;
const LOCK_POLL: Duration = Duration::from_millis(5);

/// Identity of the ledger file's content at the time it was read: a BLAKE3 content id, or `None`
/// when the file did not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerStamp(Option<Cid>);

/// The nearest ancestor of `start` (inclusive) that contains `.git` or `.polaris`.
///
/// `ceiling` is exclusive: the search never looks at it or above it. A `.polaris` directory in a
/// user's home is the global store, not a workspace marker, so [`LedgerFile::resolve`] passes the
/// home directory (the rule `polaris-config` discovery uses). Both paths are compared in canonical
/// form (`dunce`), so 8.3 short names or `\\?\` prefixes cannot slip past the ceiling.
#[must_use]
pub fn find_workspace_root(start: &Path, ceiling: Option<&Path>) -> Option<PathBuf> {
    let canonical = |p: &Path| dunce::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let start = canonical(start);
    let ceiling = ceiling.map(canonical);
    start
        .ancestors()
        .take_while(|dir| Some(*dir) != ceiling.as_deref())
        .find(|dir| dir.join(".git").exists() || dir.join(".polaris").exists())
        .map(Path::to_path_buf)
}

fn text_of(bytes: &[u8]) -> String {
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(body).into_owned()
}

/// Where the ledger lives, and how to read and replace it safely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerFile {
    path: PathBuf,
}

impl LedgerFile {
    /// A ledger at exactly `path`.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Resolves the ledger path: `POLARIS_ISSUES_FILE` if set and non-empty, then `explicit`
    /// (`--issues-file`), then `<workspace root>/.polaris/issues.md`. The root search stops at the
    /// user's home directory (exclusive), so the global `~/.polaris` store is never mistaken for a
    /// workspace.
    #[must_use]
    pub fn resolve(explicit: Option<&Path>, workspace: &Path) -> Self {
        let env = std::env::var_os(ISSUES_FILE_ENV);
        let home = std::env::home_dir();
        Self::resolve_with(env.as_deref(), explicit, workspace, home.as_deref())
    }

    /// [`LedgerFile::resolve`] with the environment value and the search ceiling given explicitly.
    /// The workspace root is the nearest ancestor of `workspace` containing `.git` or `.polaris`
    /// (see [`find_workspace_root`]), or `workspace` itself when there is none.
    #[must_use]
    pub fn resolve_with(
        env: Option<&OsStr>,
        explicit: Option<&Path>,
        workspace: &Path,
        ceiling: Option<&Path>,
    ) -> Self {
        if let Some(value) = env.filter(|v| !v.is_empty()) {
            return Self::new(value);
        }
        if let Some(path) = explicit {
            return Self::new(path);
        }
        let root =
            find_workspace_root(workspace, ceiling).unwrap_or_else(|| workspace.to_path_buf());
        Self::new(
            DEFAULT_RELATIVE
                .iter()
                .fold(root, |acc, part| acc.join(part)),
        )
    }

    /// The resolved ledger path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn parent_dir(&self) -> io::Result<&Path> {
        match self.path.parent() {
            Some(parent) if parent.as_os_str().is_empty() => Ok(Path::new(".")),
            Some(parent) => Ok(parent),
            None => Err(io::Error::other("ledger path has no parent directory")),
        }
    }

    fn read_bytes(&self) -> Result<Option<Vec<u8>>, RtkError> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Reads and parses the ledger; a missing file is an empty ledger.
    ///
    /// # Errors
    /// [`RtkError::Io`] if the file exists but cannot be read.
    pub fn load(&self) -> Result<Ledger, RtkError> {
        self.load_stamped().map(|(ledger, _)| ledger)
    }

    /// Like [`LedgerFile::load`], also returning the [`LedgerStamp`] of the bytes that were read,
    /// for [`LedgerFile::save_if_unchanged`].
    ///
    /// # Errors
    /// [`RtkError::Io`] if the file exists but cannot be read.
    pub fn load_stamped(&self) -> Result<(Ledger, LedgerStamp), RtkError> {
        match self.read_bytes()? {
            Some(bytes) => Ok((
                Ledger::parse(&text_of(&bytes)),
                LedgerStamp(Some(content_id(&bytes))),
            )),
            None => Ok((Ledger::default(), LedgerStamp(None))),
        }
    }

    fn current_stamp(&self) -> Result<LedgerStamp, RtkError> {
        Ok(LedgerStamp(self.read_bytes()?.map(|b| content_id(&b))))
    }

    /// Replaces the ledger atomically with the default [`RetryPolicy`]; see
    /// [`LedgerFile::save_atomic_with`].
    ///
    /// # Errors
    /// [`RtkError::Io`] on filesystem failure.
    pub fn save_atomic(&self, ledger: &Ledger) -> Result<(), RtkError> {
        self.save_atomic_with(ledger, &RetryPolicy::default())
    }

    /// Writes the rendered ledger (`LF`) to a unique temporary file in the ledger's directory,
    /// fsyncs it and renames it over the ledger. Readers see the old file or the new one, never a
    /// partial one. Transient "file in use" errors (antivirus, indexers on Windows) are retried per
    /// `policy`; on failure the temporary file is removed and the old ledger is left in place.
    /// Missing parent directories are created.
    ///
    /// # Errors
    /// [`RtkError::Io`] on filesystem failure, after the retry budget for in-use files.
    pub fn save_atomic_with(&self, ledger: &Ledger, policy: &RetryPolicy) -> Result<(), RtkError> {
        fs::create_dir_all(self.parent_dir()?)?;
        write_atomic(&self.path, ledger.render().as_bytes(), policy)?;
        Ok(())
    }

    /// [`LedgerFile::save_atomic_with`], but only if the file still has the content identified by
    /// `expected` (as returned by [`LedgerFile::load_stamped`]).
    ///
    /// The check and the rename are not one atomic step, so this detects a writer that finished
    /// in between, not one racing within microseconds; cooperating RTK processes are fully
    /// excluded by the lock taken in [`LedgerFile::update`].
    ///
    /// # Errors
    /// [`RtkError::ConcurrentWrite`] if the file changed; [`RtkError::Io`] on filesystem failure.
    pub fn save_if_unchanged(
        &self,
        ledger: &Ledger,
        expected: &LedgerStamp,
        policy: &RetryPolicy,
    ) -> Result<(), RtkError> {
        if self.current_stamp()? != *expected {
            return Err(RtkError::ConcurrentWrite);
        }
        self.save_atomic_with(ledger, policy)
    }

    /// Takes an exclusive advisory lock on `<ledger>.lock` (kept on disk, empty) so concurrent RTK
    /// processes serialise their read-modify-write cycles. Released when the returned file drops.
    fn lock(&self) -> Result<File, RtkError> {
        let parent = self.parent_dir()?;
        fs::create_dir_all(parent)?;
        let mut name = self
            .path
            .file_name()
            .ok_or_else(|| io::Error::other("ledger path has no file name"))?
            .to_os_string();
        name.push(".lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(parent.join(name))?;
        for _ in 0..LOCK_ATTEMPTS {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(TryLockError::WouldBlock) => std::thread::sleep(LOCK_POLL),
                Err(TryLockError::Error(e)) => return Err(e.into()),
            }
        }
        Err(RtkError::LockTimeout)
    }

    /// Reads the ledger, reconciles it with `diagnostics` and saves it, under the advisory lock.
    /// If a writer that ignores the lock changes the file in between, the cycle is repeated once.
    /// Nothing is written when reconciliation changes nothing (so a missing ledger stays missing
    /// for a clean build).
    ///
    /// # Errors
    /// [`RtkError::LockTimeout`], [`RtkError::ConcurrentWrite`] (after the one retry) or
    /// [`RtkError::Io`].
    pub fn update(
        &self,
        diagnostics: &[Diagnostic],
        now: Timestamp,
    ) -> Result<LedgerSummary, RtkError> {
        let _guard = self.lock()?;
        let policy = RetryPolicy::default();
        let mut retried = false;
        loop {
            let (mut ledger, stamp) = self.load_stamped()?;
            let summary = ledger.reconcile(diagnostics, now);
            if summary.newly_opened == 0 && summary.newly_closed == 0 {
                return Ok(summary);
            }
            match self.save_if_unchanged(&ledger, &stamp, &policy) {
                Ok(()) => return Ok(summary),
                Err(RtkError::ConcurrentWrite) if !retried => retried = true,
                Err(e) => return Err(e),
            }
        }
    }

    /// Moves a legacy `ISSUES.md` from `workspace` to this ledger's path, once: only if the ledger
    /// does not exist yet and the legacy file does. Returns whether a move happened (the caller
    /// should print it).
    ///
    /// # Errors
    /// [`RtkError::Io`] on filesystem failure.
    pub fn migrate_legacy(&self, workspace: &Path) -> Result<bool, RtkError> {
        let legacy = workspace.join("ISSUES.md");
        if self.path.exists() || !legacy.is_file() {
            return Ok(false);
        }
        fs::create_dir_all(self.parent_dir()?)?;
        with_retry(&RetryPolicy::default(), || fs::rename(&legacy, &self.path))?;
        Ok(true)
    }
}
