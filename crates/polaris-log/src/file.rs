//! The drain side: a dedicated thread that turns queued lines into `spdlog-rs` records and writes
//! them to a size-rotated file (and, outside TUI mode, to stderr).
//!
//! `spdlog-rs` 0.5.3 is used for what it does well on every platform: the rotating file sink and
//! the std-stream sink. Its own async pool (`multi-thread` feature) is not used: it clones every
//! record into an owned allocation, which the closed zero-alloc hot-path set (ADR-003) forbids on
//! the producer side. Producers enqueue into [`DrainQueue`] instead and `spdlog-rs` runs on this
//! thread only.

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use spdlog::formatter::{Formatter, FormatterContext};
use spdlog::sink::{GetSinkProp, RotatingFileSink, RotationPolicy, Sink, SinkProp, StdStreamSink};
use spdlog::terminal_style::StyleMode;
use spdlog::{ErrorHandler, LevelFilter, Logger, Record, StringBuf};

use crate::entry::{Level, LogLine};
use crate::error::LogError;
use crate::init::LogConfig;
use crate::queue::DrainQueue;
use crate::redact::LogCounters;

/// Name of the active log file inside [`LogConfig::dir`]. Rotated files are `polaris_1.log`
/// (newest) to `polaris_<keep_files - 1>.log` (oldest).
pub const LOG_FILE_NAME: &str = "polaris.log";

/// Pauses before the 2nd and 3rd write attempt of a record (bounded manual retry for files held
/// by antivirus or indexers on Windows; `backon` is deliberately not used).
const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(1), Duration::from_millis(5)];

/// Writes the payload (already the complete line) and a newline: the line layout is decided by
/// this crate, so the event time survives the queue.
#[derive(Clone, Copy)]
struct PayloadFormatter;

impl Formatter for PayloadFormatter {
    fn format(
        &self,
        record: &Record,
        dest: &mut StringBuf,
        _ctx: &mut FormatterContext,
    ) -> spdlog::Result<()> {
        dest.write_str(record.payload())
            .and_then(|()| dest.write_char('\n'))
            .map_err(spdlog::Error::FormatRecord)
    }
}

/// Wraps the rotating file sink with a bounded retry. After a record fails every attempt the sink
/// is "degraded" and later records get a single attempt (no sleeping) until one succeeds, so a
/// dead disk costs the drain thread microseconds per record, not milliseconds.
struct RetryingSink {
    inner: Arc<RotatingFileSink>,
    prop: SinkProp,
    counters: Arc<LogCounters>,
    degraded: AtomicBool,
}

impl GetSinkProp for RetryingSink {
    fn prop(&self) -> &SinkProp {
        &self.prop
    }
}

impl Sink for RetryingSink {
    fn log(&self, record: &Record) -> spdlog::Result<()> {
        let mut delays = if self.degraded.load(Ordering::Relaxed) {
            [].iter()
        } else {
            RETRY_DELAYS.iter()
        };
        loop {
            match self.inner.log(record) {
                Ok(()) => {
                    self.degraded.store(false, Ordering::Relaxed);
                    return Ok(());
                }
                Err(err) => match delays.next() {
                    Some(delay) => {
                        self.counters.record_sink_error(err.to_string());
                        std::thread::sleep(*delay);
                    }
                    None => {
                        self.degraded.store(true, Ordering::Relaxed);
                        return Err(err);
                    }
                },
            }
        }
    }

    fn flush(&self) -> spdlog::Result<()> {
        self.inner.flush()
    }
}

fn sink_init(err: spdlog::Error) -> LogError {
    LogError::SinkInit(err.to_string())
}

fn error_handler(counters: &Arc<LogCounters>) -> ErrorHandler {
    // spdlog-rs would print unhandled errors to stderr, which must stay silent in TUI mode.
    let counters = Arc::clone(counters);
    ErrorHandler::new(move |err| counters.record_sink_error(err.to_string()))
}

/// Builds the `spdlog-rs` logger: the rotating file sink plus, unless `cfg.tui_mode`, stderr.
pub(crate) fn build_logger(
    cfg: &LogConfig,
    counters: &Arc<LogCounters>,
) -> Result<Logger, LogError> {
    let file = RotatingFileSink::builder()
        .base_path(cfg.dir.join(LOG_FILE_NAME))
        .rotation_policy(RotationPolicy::FileSize(cfg.rotate_bytes))
        .max_files(usize::from(cfg.keep_files))
        .formatter(PayloadFormatter)
        .error_handler(error_handler(counters))
        .build_arc()
        .map_err(sink_init)?;
    let retrying = RetryingSink {
        inner: file,
        prop: SinkProp::default(),
        counters: Arc::clone(counters),
        degraded: AtomicBool::new(false),
    };
    retrying.prop.set_error_handler(error_handler(counters));

    // dyn: spdlog-rs's `Logger` stores `Arc<dyn Sink>`; this runs on the drain thread only.
    let mut sinks: Vec<Arc<dyn Sink>> = vec![Arc::new(retrying)];
    if !cfg.tui_mode {
        let console = StdStreamSink::builder()
            .stderr()
            .style_mode(StyleMode::Never)
            .formatter(PayloadFormatter)
            .error_handler(error_handler(counters))
            .build_arc()
            .map_err(sink_init)?;
        sinks.push(console);
    }
    Logger::builder()
        .level_filter(LevelFilter::All)
        .sinks(sinks)
        .error_handler(error_handler(counters))
        .build()
        .map_err(sink_init)
}

fn spdlog_level(level: Level) -> spdlog::Level {
    if level == Level::ERROR {
        spdlog::Level::Error
    } else if level == Level::WARN {
        spdlog::Level::Warn
    } else if level == Level::INFO {
        spdlog::Level::Info
    } else if level == Level::DEBUG {
        spdlog::Level::Debug
    } else {
        spdlog::Level::Trace
    }
}

fn write_line(logger: &Logger, line: &LogLine, json: bool) {
    let text = if json {
        line.to_json()
    } else {
        line.to_string()
    };
    spdlog::log!(logger: logger, spdlog_level(line.level), "{}", text);
}

/// Starts the drain thread. It writes until the queue is closed and empty, flushing the sinks
/// whenever it runs out of work.
pub(crate) fn spawn_drain(
    queue: Arc<DrainQueue>,
    logger: Logger,
    json: bool,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("polaris-log-drain".to_string())
        .spawn(move || {
            loop {
                while let Some(line) = queue.try_recv() {
                    write_line(&logger, &line, json);
                }
                logger.flush();
                queue.mark_flushed();
                if !queue.wait_for_items() {
                    break;
                }
            }
        })
}
