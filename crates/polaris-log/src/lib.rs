//! # polaris-log
//!
//! `tracing` facade for Polaris (L1, P1): structured events are redacted, then fanned out to
//! non-blocking sinks. The emitting thread never waits for disk or terminal I/O.
//!
//! ```text
//! tracing::info!(..)
//!   -> RedactionLayer   format into reused thread-local buffers, Redactor::redact_into, escape
//!        |-> RingSink (TUI log pane)          overwrite-oldest, drop counter, zero-alloc push
//!        |-> RingSink (telemetry)             same
//!        `-> DrainQueue -> drain thread -> spdlog-rs RotatingFileSink (+ stderr unless TUI mode)
//!                          drop-newest when full, drop counter, producer never waits
//! ```
//!
//! - **Redaction first.** Every record passes the sdk [`Redactor`](polaris_sdk::registry::Redactor)
//!   before any sink sees it. The trait is infallible, so a redactor that panics or reports an
//!   incomplete scan fails closed: the record is dropped and counted ([`LogStats`]).
//! - **Overflow policy.** Rings overwrite the oldest line; the file queue drops the newest record.
//!   Both count what they lose. Lines are cut to a fixed limit so all memory is allocated up front.
//! - **Zero-alloc enqueue.** After warm-up, `tracing` event to enqueue does not allocate
//!   (`tests/log_enqueue_zero_alloc.rs`, ADR-003); `spdlog-rs` allocates on the drain thread only.
//! - **TUI mode.** With [`LogConfig::tui_mode`] nothing is written to stdout or stderr.
//! - **Process-wide state.** [`init`] installs the global subscriber once; [`start`] builds the
//!   same pipeline without installing it (tests, embedding).
//!
//! Daily and size rotation share retention. The application flushes [`LogGuard::flush`] from its
//! panic/signal paths. Span context and a telemetry-specific record format are later extensions.
//!
//! Test-first descriptors: `specs/descriptors/log_*.json`.

mod entry;
mod error;
mod file;
mod init;
mod queue;
mod redact;
mod ring;

pub use entry::{Level, LogLine, LogRecordRef, MAX_TARGET_BYTES, MIN_LINE_BYTES, format_timestamp};
pub use error::LogError;
pub use file::LOG_FILE_NAME;
pub use init::{LogConfig, LogGuard, LogStats, init, resolve_filter, start};
pub use queue::DrainQueue;
pub use redact::{LogCounters, RAW_LIMIT, RedactionLayer, Sinks};
pub use ring::{DEFAULT_RING_LINE_BYTES, RingSink};
