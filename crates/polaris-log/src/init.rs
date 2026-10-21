//! Configuration, filter resolution, `start` / `init` and the guard.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use polaris_sdk::registry::Redactor;
use tracing::Dispatch;
use tracing_subscriber::layer::{Layer, SubscriberExt};
use tracing_subscriber::{EnvFilter, Registry};

use crate::entry::MIN_LINE_BYTES;
use crate::error::LogError;
use crate::file::{build_logger, spawn_drain};
use crate::queue::DrainQueue;
use crate::redact::{LogCounters, RedactionLayer, Sinks};
use crate::ring::RingSink;

/// Set once a global subscriber has been installed (or is being installed). Never cleared after
/// success: `tracing` cannot replace a global subscriber, so neither can we.
static INITIALISED: AtomicBool = AtomicBool::new(false);

/// Logging configuration. Build it with [`LogConfig::new`] and adjust the public fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogConfig {
    /// Directory of the rotating log file (created if missing).
    pub dir: PathBuf,
    /// `RUST_LOG`-style directives. Empty means: use the `RUST_LOG` environment variable, and if
    /// that is unset or empty, `info`.
    pub level: String,
    /// Rotate to a new file before the active one would exceed this many bytes.
    pub rotate_bytes: u64,
    /// Number of log files kept on disk, the active one included (at least 1).
    pub keep_files: u16,
    /// Lines held by each of the TUI ring and the telemetry ring.
    pub ring_capacity: usize,
    /// Emit one JSON object per record instead of text (applies to the file, console and rings).
    pub json: bool,
    /// The TUI owns the terminal: no sink writes to stdout or stderr; only the rings and the file
    /// are active. When `false`, records are also written to stderr (stdout is never used).
    pub tui_mode: bool,
    /// Slots of the queue between producers and the drain thread (overflow drops the newest).
    pub queue_capacity: usize,
    /// Longest stored line in bytes (target and message together, at least 256). Longer lines are
    /// cut and flagged; this bounds memory because all slots are allocated up front.
    pub max_line_bytes: usize,
}

impl LogConfig {
    /// Defaults: `info`, 10 MiB files, 5 files kept, 1024-line rings, text, console on, 1024-slot
    /// queue, 2048-byte lines.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            level: "info".to_string(),
            rotate_bytes: 10 * 1024 * 1024,
            keep_files: 5,
            ring_capacity: 1024,
            json: false,
            tui_mode: false,
            queue_capacity: 1024,
            max_line_bytes: 2048,
        }
    }

    fn validate(&self) -> Result<(), LogError> {
        let bad = |what: &str| Err(LogError::BadConfig(what.to_string()));
        if self.dir.as_os_str().is_empty() {
            return bad("dir must not be empty");
        }
        if self.rotate_bytes == 0 {
            return bad("rotate_bytes must be at least 1");
        }
        if self.keep_files == 0 {
            return bad("keep_files must be at least 1");
        }
        if self.ring_capacity == 0 {
            return bad("ring_capacity must be at least 1");
        }
        if self.queue_capacity == 0 {
            return bad("queue_capacity must be at least 1");
        }
        if self.max_line_bytes < MIN_LINE_BYTES {
            return bad("max_line_bytes must be at least 256");
        }
        Ok(())
    }
}

/// Parses the filter: the configured `level` if it is not blank, else the `RUST_LOG` value `env`
/// (read by the caller with `var_os`), else `info`. A non-Unicode `RUST_LOG` is only an error when
/// it is the one being used.
///
/// # Errors
/// [`LogError::BadFilter`] when the directives do not parse.
pub fn resolve_filter(level: &str, env: Option<&OsStr>) -> Result<EnvFilter, LogError> {
    let configured = level.trim();
    let directives = if !configured.is_empty() {
        configured
    } else if let Some(value) = env {
        let text = value
            .to_str()
            .ok_or_else(|| LogError::BadFilter("RUST_LOG is not valid Unicode".to_string()))?
            .trim();
        if text.is_empty() { "info" } else { text }
    } else {
        "info"
    };
    EnvFilter::try_new(directives).map_err(|e| LogError::BadFilter(e.to_string()))
}

/// Counters of everything that was redacted, dropped or failed, for `/status` and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LogStats {
    /// Secret/PII spans replaced by the redactor.
    pub redactions: u64,
    /// Events dropped because the redactor failed (fail closed).
    pub redact_failures: u64,
    /// Events replaced by a stub because they were too large to scan.
    pub oversize_events: u64,
    /// Failed sink write attempts and sink errors.
    pub sink_errors: u64,
    /// Records accepted by the file queue.
    pub queue_accepted: u64,
    /// Records dropped because the file queue was full or closed.
    pub queue_dropped: u64,
    /// Lines overwritten in the TUI ring.
    pub tui_dropped: u64,
    /// Lines overwritten in the telemetry ring.
    pub telemetry_dropped: u64,
}

/// Keeps logging alive. Dropping it closes the queue, lets the drain thread write and flush
/// everything already accepted, and joins it. The thread blocks on file I/O, so a hung disk can
/// delay the drop; producers are never affected.
#[derive(Debug)]
pub struct LogGuard {
    queue: Arc<DrainQueue>,
    drain: Option<JoinHandle<()>>,
    tui: Arc<RingSink>,
    telemetry: Arc<RingSink>,
    counters: Arc<LogCounters>,
    log_file: PathBuf,
}

impl LogGuard {
    /// The ring backing the TUI log pane.
    #[must_use]
    pub fn tui_ring(&self) -> Arc<RingSink> {
        Arc::clone(&self.tui)
    }

    /// The ring read by telemetry.
    #[must_use]
    pub fn telemetry_ring(&self) -> Arc<RingSink> {
        Arc::clone(&self.telemetry)
    }

    /// Waits until every record accepted so far is written and flushed (for panic hooks and
    /// shutdown paths). `false` if `timeout` elapsed first.
    #[must_use]
    pub fn flush(&self, timeout: Duration) -> bool {
        self.queue.flush(timeout)
    }

    /// Current counters.
    #[must_use]
    pub fn stats(&self) -> LogStats {
        LogStats {
            redactions: self.counters.redactions(),
            redact_failures: self.counters.redact_failures(),
            oversize_events: self.counters.oversize_events(),
            sink_errors: self.counters.sink_errors(),
            queue_accepted: self.queue.accepted(),
            queue_dropped: self.queue.dropped(),
            tui_dropped: self.tui.dropped(),
            telemetry_dropped: self.telemetry.dropped(),
        }
    }

    /// The most recent sink error, as text.
    #[must_use]
    pub fn last_sink_error(&self) -> Option<String> {
        self.counters.last_sink_error()
    }

    /// Path of the active log file.
    #[must_use]
    pub fn log_file(&self) -> &Path {
        &self.log_file
    }
}

impl Drop for LogGuard {
    fn drop(&mut self) {
        self.queue.close();
        if let Some(handle) = self.drain.take() {
            // A panicked drain thread has nothing left to report here.
            let _ = handle.join();
        }
    }
}

/// Builds the whole pipeline (rings, queue, drain thread, layer) and returns the guard plus the
/// `tracing` dispatcher, **without installing anything globally**. Use it for tests and for
/// embedding; [`init`] is `start` plus installation.
///
/// # Errors
/// [`LogError::BadConfig`], [`LogError::BadFilter`] (both before anything touches the disk),
/// [`LogError::SinkInit`] (log directory or file unusable) or [`LogError::Io`] (thread spawn).
pub fn start<R: Redactor + 'static>(
    cfg: LogConfig,
    redactor: R,
) -> Result<(LogGuard, Dispatch), LogError> {
    cfg.validate()?;
    let rust_log = std::env::var_os("RUST_LOG");
    let filter = resolve_filter(&cfg.level, rust_log.as_deref())?;

    let counters = LogCounters::new();
    let logger = build_logger(&cfg, &counters)?;
    let tui = Arc::new(RingSink::with_line_limit(
        cfg.ring_capacity,
        cfg.max_line_bytes,
    ));
    let telemetry = Arc::new(RingSink::with_line_limit(
        cfg.ring_capacity,
        cfg.max_line_bytes,
    ));
    let queue = Arc::new(DrainQueue::new(cfg.queue_capacity, cfg.max_line_bytes));
    let drain = spawn_drain(Arc::clone(&queue), logger, cfg.json)?;

    let sinks = Sinks::new(Arc::clone(&counters))
        .with_tui(Arc::clone(&tui))
        .with_telemetry(Arc::clone(&telemetry))
        .with_file(Arc::clone(&queue));
    let layer = RedactionLayer::new(redactor, sinks).with_json(cfg.json);
    let dispatch = Dispatch::new(Registry::default().with(layer.with_filter(filter)));

    let guard = LogGuard {
        queue,
        drain: Some(drain),
        tui,
        telemetry,
        counters,
        log_file: cfg.dir.join(crate::file::LOG_FILE_NAME),
    };
    Ok((guard, dispatch))
}

/// Installs logging for the whole process: [`start`] plus `tracing`'s global default. Only the
/// first call can succeed; a failed attempt (bad config, unusable directory) does not use up the
/// slot.
///
/// # Errors
/// Everything [`start`] returns, plus [`LogError::AlreadyInitialised`] when a global subscriber
/// exists already (also after the first guard was dropped, or if another library installed one).
pub fn init<R: Redactor + 'static>(cfg: LogConfig, redactor: R) -> Result<LogGuard, LogError> {
    if INITIALISED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(LogError::AlreadyInitialised);
    }
    let (guard, dispatch) = match start(cfg, redactor) {
        Ok(started) => started,
        Err(err) => {
            INITIALISED.store(false, Ordering::SeqCst);
            return Err(err);
        }
    };
    if tracing::dispatcher::set_global_default(dispatch).is_err() {
        // Someone else owns the global slot; it stays taken, so the flag stays set.
        return Err(LogError::AlreadyInitialised);
    }
    Ok(guard)
}
