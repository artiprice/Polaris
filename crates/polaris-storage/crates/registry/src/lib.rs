//! # polaris-storage-registry
//!
//! The machine-wide session catalogue (`registry.db`) on SQLite, plus a full-text index over
//! prompts and responses for `/sessions` and `polaris_recall`.
//!
//! - **Catalogue**: [`SessionMeta`] rows (workspace, branch, base commit, token counters, sandbox
//!   checkpoint, status) via [`Registry::upsert_session`], [`Registry::get_session`],
//!   [`Registry::list_sessions`] (newest `updated_at` first), [`Registry::archive`] /
//!   [`Registry::unarchive`].
//! - **Search**: [`Registry::index_message`] feeds an FTS5 table with the `trigram` tokenizer;
//!   [`Registry::search_history`] finds any substring of three or more characters (shorter queries
//!   use `LIKE`). Queries are quoted ([`fts_phrase`]) so user text is never FTS5 syntax.
//! - **Masked text only**: the registry stores what it is given. Use
//!   [`Registry::index_message_redacted`] (or mask first) so secrets never reach the file.
//! - **Concurrency**: each [`Registry`] owns a database thread; WAL lets the CLI, daemon and
//!   desktop app open the same directory at once. Lock contention is retried a bounded number of
//!   times, then reported as [`RegistryError::Busy`].
//! - **Schema**: `PRAGMA user_version` with forward-only migrations ([`migrate()`]); a file from a
//!   newer Polaris is refused ([`RegistryError::SchemaTooNew`]).
//! - **Windows**: paths with spaces and non-ASCII characters work; `-wal` and `-shm` side files sit
//!   next to `registry.db`; network drives are unsupported (no WAL) and refused at open.
//! - The API is blocking: async callers use `spawn_blocking`.

mod error;
mod history;
mod migrate;
mod registry;
mod search;
mod sessions;

pub use error::RegistryError;
pub use migrate::{SCHEMA_VERSION, migrate};
pub use registry::{DB_FILE_NAME, Registry, RegistryConfig, open_connection};
pub use search::{Hit, MAX_SEARCH_LIMIT, SNIPPET_CHARS, fts_phrase, like_pattern};
pub use sessions::{SessionFilter, SessionMeta, SessionStatus};
