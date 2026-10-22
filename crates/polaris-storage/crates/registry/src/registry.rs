//! The [`Registry`] handle, its configuration and the database-thread actor behind it.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::JoinHandle;
use std::time::Duration;

use polaris_sdk::contracts::Timestamp;
use polaris_sdk::ids::SessionId;
use polaris_sdk::registry::{ChatRole, Redactor};
use rusqlite::Connection;

use crate::error::RegistryError;
use crate::migrate::migrate;
use crate::search::{Hit, MAX_SEARCH_QUERY_BYTES};
use crate::sessions::{SessionFilter, SessionMeta, SessionStatus};
use crate::{history, search, sessions};

/// File name of the database inside the registry directory.
pub const DB_FILE_NAME: &str = "registry.db";

/// Tuning for lock contention. The defaults suit a CLI, a daemon and a desktop app sharing one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryConfig {
    /// How long SQLite itself waits for a lock held by another connection before reporting busy.
    pub busy_timeout: Duration,
    /// Extra attempts after a busy report before [`RegistryError::Busy`] is returned.
    pub busy_retries: u32,
    /// Pause before retry `n` is `retry_backoff * n`.
    pub retry_backoff: Duration,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            busy_timeout: Duration::from_secs(5),
            busy_retries: 3,
            retry_backoff: Duration::from_millis(25),
        }
    }
}

/// Opens (creating if needed) the registry database at `path` and prepares the connection:
/// busy timeout, WAL journal, `synchronous=NORMAL`, `foreign_keys=ON`, then schema migration.
///
/// WAL needs a local file system; on a network drive SQLite silently keeps another journal mode
/// and this function refuses with [`RegistryError::Open`] rather than continue without WAL.
///
/// # Errors
/// [`RegistryError::Open`], [`RegistryError::Corrupt`] (not a SQLite file), [`RegistryError::Busy`],
/// [`RegistryError::SchemaTooNew`], [`RegistryError::Migration`].
pub fn open_connection(path: &Path, cfg: &RegistryConfig) -> Result<Connection, RegistryError> {
    let mut conn = Connection::open(path).map_err(|e| match RegistryError::from(e) {
        RegistryError::Sql(message) => RegistryError::Open(message),
        other => other,
    })?;
    conn.busy_timeout(cfg.busy_timeout)?;
    let mode: String =
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(RegistryError::Open(format!(
            "WAL journal mode is unavailable here (got {mode:?}); is the registry on a network drive?"
        )));
    }
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate(&mut conn)?;
    Ok(conn)
}

type Reply<T> = Sender<Result<T, RegistryError>>;

/// A request to the database thread. Each carries its own reply channel.
enum Request {
    Upsert(SessionMeta, Reply<()>),
    Get(SessionId, Reply<Option<SessionMeta>>),
    List(SessionFilter, Reply<Vec<SessionMeta>>),
    SetStatus(SessionId, SessionStatus, Timestamp, Reply<bool>),
    Index(SessionId, ChatRole, String, Timestamp, Reply<()>),
    Search(String, u32, Reply<Vec<Hit>>),
    SearchForSession(SessionId, String, u32, Reply<Vec<Hit>>),
}

/// Runs `op`, retrying with a growing pause while it reports [`RegistryError::Busy`].
fn with_retry<T>(
    cfg: &RegistryConfig,
    mut op: impl FnMut() -> Result<T, RegistryError>,
) -> Result<T, RegistryError> {
    let mut attempt: u32 = 0;
    loop {
        match op() {
            Err(RegistryError::Busy) if attempt < cfg.busy_retries => {
                attempt = attempt.saturating_add(1);
                std::thread::sleep(cfg.retry_backoff.saturating_mul(attempt));
            }
            other => return other,
        }
    }
}

/// The database thread: owns the only `Connection`, answers requests until every sender is gone.
fn serve(conn: &Connection, requests: &Receiver<Request>, cfg: &RegistryConfig) {
    for request in requests {
        // A dropped reply receiver means the caller gave up; there is nobody to tell.
        match request {
            Request::Upsert(meta, reply) => {
                let _ = reply.send(with_retry(cfg, || sessions::upsert(conn, &meta)));
            }
            Request::Get(id, reply) => {
                let _ = reply.send(with_retry(cfg, || sessions::get(conn, id)));
            }
            Request::List(filter, reply) => {
                let _ = reply.send(with_retry(cfg, || sessions::list(conn, &filter)));
            }
            Request::SetStatus(id, status, at, reply) => {
                let _ = reply.send(with_retry(cfg, || {
                    sessions::set_status(conn, id, status, at)
                }));
            }
            Request::Index(session, role, text, at, reply) => {
                let _ = reply.send(with_retry(cfg, || {
                    history::index(conn, session, role, &text, at)
                }));
            }
            Request::Search(query, limit, reply) => {
                let _ = reply.send(with_retry(cfg, || search::search(conn, &query, limit)));
            }
            Request::SearchForSession(session, query, limit, reply) => {
                let _ = reply.send(with_retry(cfg, || {
                    search::search_for_session(conn, session, &query, limit)
                }));
            }
        }
    }
}

/// Handle to the machine-wide session registry (`registry.db`).
///
/// The handle owns a dedicated database thread that holds the only SQLite connection of this
/// handle; calls send a request over a channel and block until the answer arrives, so the handle
/// is `Send + Sync` and needs no `Mutex<Connection>`. Async callers run the (short) calls on
/// `spawn_blocking`. Several handles, in one process or in several (CLI, daemon, desktop), may open
/// the same directory: WAL gives them concurrent readers and one writer at a time, and writes that
/// hit a lock held by another connection are retried a bounded number of times before
/// [`RegistryError::Busy`].
///
/// Dropping the handle stops the thread, which closes the connection.
#[derive(Debug)]
pub struct Registry {
    requests: Option<SyncSender<Request>>,
    thread: Option<JoinHandle<()>>,
    path: PathBuf,
}

impl Registry {
    /// Opens or creates `dir/registry.db` with the default [`RegistryConfig`].
    ///
    /// # Errors
    /// As [`Registry::open_with`].
    pub fn open(dir: &Path) -> Result<Self, RegistryError> {
        Self::open_with(dir, &RegistryConfig::default())
    }

    /// Opens or creates `dir/registry.db` (creating `dir`), applies WAL and the pragmas, migrates
    /// the schema and starts the database thread.
    ///
    /// # Errors
    /// Everything [`open_connection`] reports, plus [`RegistryError::Open`] when the directory or
    /// the thread cannot be created.
    pub fn open_with(dir: &Path, cfg: &RegistryConfig) -> Result<Self, RegistryError> {
        std::fs::create_dir_all(dir)
            .map_err(|e| RegistryError::Open(format!("cannot create {}: {e}", dir.display())))?;
        let path = dir.join(DB_FILE_NAME);
        let conn = open_connection(&path, cfg)?;
        let (requests, inbox) = mpsc::sync_channel(256);
        let cfg = *cfg;
        let thread = std::thread::Builder::new()
            .name("polaris-registry".to_owned())
            .spawn(move || serve(&conn, &inbox, &cfg))
            .map_err(|e| RegistryError::Open(format!("cannot start database thread: {e}")))?;
        Ok(Self {
            requests: Some(requests),
            thread: Some(thread),
            path,
        })
    }

    /// Path of the database file.
    #[must_use]
    pub fn db_path(&self) -> &Path {
        &self.path
    }

    fn call<T>(&self, build: impl FnOnce(Reply<T>) -> Request) -> Result<T, RegistryError> {
        let requests = self.requests.as_ref().ok_or(RegistryError::Closed)?;
        let (reply, answer) = mpsc::channel();
        requests
            .send(build(reply))
            .map_err(|_| RegistryError::Closed)?;
        answer.recv().map_err(|_| RegistryError::Closed)?
    }

    /// Inserts the session, or updates every field except `created_at` when the id exists.
    /// Idempotent.
    ///
    /// # Errors
    /// [`RegistryError::BadValue`] for a counter above `i64::MAX` or a non-UTF-8 workspace path;
    /// [`RegistryError::Busy`] / [`RegistryError::Sql`] from SQLite.
    pub fn upsert_session(&self, meta: &SessionMeta) -> Result<(), RegistryError> {
        self.call(|reply| Request::Upsert(meta.clone(), reply))
    }

    /// The session with `id`, or `None` when it was never written.
    ///
    /// # Errors
    /// [`RegistryError::Corrupt`] for an undecodable row; SQLite errors.
    pub fn get_session(&self, id: SessionId) -> Result<Option<SessionMeta>, RegistryError> {
        self.call(|reply| Request::Get(id, reply))
    }

    /// Sessions matching `filter`, newest `updated_at` first (`/sessions` order).
    ///
    /// # Errors
    /// As [`Registry::get_session`], plus [`RegistryError::BadValue`] for a non-UTF-8 workspace.
    pub fn list_sessions(&self, filter: &SessionFilter) -> Result<Vec<SessionMeta>, RegistryError> {
        self.call(|reply| Request::List(filter.clone(), reply))
    }

    /// Sets the session's status and `updated_at`; `false` when the id is unknown.
    ///
    /// # Errors
    /// SQLite errors.
    pub fn set_status(
        &self,
        id: SessionId,
        status: SessionStatus,
        at: Timestamp,
    ) -> Result<bool, RegistryError> {
        self.call(|reply| Request::SetStatus(id, status, at, reply))
    }

    /// Marks the session [`SessionStatus::Archived`]; `false` when the id is unknown.
    ///
    /// # Errors
    /// As [`Registry::set_status`].
    pub fn archive(&self, id: SessionId, at: Timestamp) -> Result<bool, RegistryError> {
        self.set_status(id, SessionStatus::Archived, at)
    }

    /// Marks the session [`SessionStatus::Active`] again (a previous `Paused` is not restored);
    /// `false` when the id is unknown.
    ///
    /// # Errors
    /// As [`Registry::set_status`].
    pub fn unarchive(&self, id: SessionId, at: Timestamp) -> Result<bool, RegistryError> {
        self.set_status(id, SessionStatus::Active, at)
    }

    /// Adds a message to the searchable history. The text is stored **verbatim**: it must already
    /// be PII-masked (see [`Registry::index_message_redacted`]). The session need not exist yet.
    ///
    /// # Errors
    /// SQLite errors ([`RegistryError::Busy`], [`RegistryError::Sql`]).
    pub fn index_message(
        &self,
        session: SessionId,
        role: ChatRole,
        text: &str,
        at: Timestamp,
    ) -> Result<(), RegistryError> {
        self.call(|reply| Request::Index(session, role, text.to_owned(), at, reply))
    }

    /// Masks `text` with `redactor` and indexes only the masked result, so secrets matched by the
    /// redactor are never written to the registry files. Invalid UTF-8 produced by the redactor is
    /// replaced with U+FFFD.
    ///
    /// # Errors
    /// As [`Registry::index_message`].
    pub fn index_message_redacted<R: Redactor + ?Sized>(
        &self,
        session: SessionId,
        role: ChatRole,
        redactor: &R,
        text: &str,
        at: Timestamp,
    ) -> Result<(), RegistryError> {
        let mut masked = Vec::with_capacity(text.len());
        redactor.redact_into(text.as_bytes(), &mut masked);
        let masked = String::from_utf8_lossy(&masked);
        self.index_message(session, role, &masked, at)
    }

    /// Finds messages containing `query` as a literal substring (case-insensitive), newest first.
    ///
    /// Queries of three or more characters use the FTS5 trigram index; shorter ones fall back to
    /// `LIKE`. The query is untrusted: it is quoted/escaped and never interpreted as FTS5 or SQL
    /// syntax, so any string within the byte bound is accepted (an empty one, or `limit == 0`,
    /// yields no hits). At most `limit` hits, capped at [`crate::MAX_SEARCH_LIMIT`], are returned.
    /// Queries above [`crate::MAX_SEARCH_QUERY_BYTES`] bytes return [`RegistryError::BadValue`]
    /// before being copied into the bounded database-thread queue.
    ///
    /// # Errors
    /// SQLite errors; [`RegistryError::Corrupt`] for an undecodable row.
    pub fn search_history(&self, query: &str, limit: u32) -> Result<Vec<Hit>, RegistryError> {
        check_search_query_size(query)?;
        self.call(|reply| Request::Search(query.to_owned(), limit, reply))
    }

    /// Finds literal matches in exactly one session. Session filtering happens in SQLite before
    /// sorting and applying `limit`, so messages from other sessions cannot crowd out results.
    ///
    /// Query syntax, bounds, ordering and errors match [`Registry::search_history`].
    pub fn search_history_for_session(
        &self,
        session: SessionId,
        query: &str,
        limit: u32,
    ) -> Result<Vec<Hit>, RegistryError> {
        check_search_query_size(query)?;
        self.call(|reply| Request::SearchForSession(session, query.to_owned(), limit, reply))
    }
}

fn check_search_query_size(query: &str) -> Result<(), RegistryError> {
    if query.len() > MAX_SEARCH_QUERY_BYTES {
        return Err(RegistryError::BadValue(format!(
            "search query exceeds the {MAX_SEARCH_QUERY_BYTES}-byte limit"
        )));
    }
    Ok(())
}

impl Drop for Registry {
    fn drop(&mut self) {
        // Closing the channel ends the thread's loop; joining makes sure the connection is closed
        // (and the WAL checkpointed) before the caller can reopen the directory.
        self.requests.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
