//! The drain side: a dedicated thread that turns queued lines into `spdlog-rs` records and writes
//! them to a UTC-daily and size-rotated file (and, outside TUI mode, to stderr).
//!
//! `spdlog-rs` 0.5.3 provides the std-stream sink. The in-house file sink combines daily and size
//! rotation with bounded retention. Its own async pool (`multi-thread` feature) is not used: it clones every
//! record into an owned allocation, which the closed zero-alloc hot-path set (ADR-003) forbids on
//! the producer side. Producers enqueue into [`DrainQueue`] instead and `spdlog-rs` runs on this
//! thread only.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::Write as IoWrite;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use spdlog::formatter::{Formatter, FormatterContext};
use spdlog::sink::{GetSinkProp, Sink, SinkProp, StdStreamSink};
use spdlog::terminal_style::StyleMode;
use spdlog::{ErrorHandler, LevelFilter, Logger, Record, StringBuf};

use crate::entry::{Level, LogLine};
use crate::error::LogError;
use crate::init::LogConfig;
use crate::queue::DrainQueue;
use crate::redact::LogCounters;
use crate::ring::lock;

/// Name of the active log file inside [`LogConfig::dir`]. Rotated files are `polaris_1.log`
/// (newest) to `polaris_<keep_files - 1>.log` (oldest).
pub const LOG_FILE_NAME: &str = "polaris.log";

#[derive(Debug)]
struct DailySizeState {
    file: Option<File>,
    size: u64,
    utc_day: u64,
}

/// A single-writer file sink that rotates at UTC midnight and before a size limit is crossed.
/// Rotation keeps the same numbered filenames for both causes, so the configured retention
/// bound applies across the current and archived files.
struct DailySizeSink {
    prop: SinkProp,
    dir: PathBuf,
    max_bytes: u64,
    keep_files: usize,
    state: Mutex<DailySizeState>,
}

impl DailySizeSink {
    fn new(cfg: &LogConfig) -> std::io::Result<Self> {
        fs::create_dir_all(&cfg.dir)?;
        let path = cfg.dir.join(LOG_FILE_NAME);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata()?.len();
        let day = if size == 0 {
            utc_day()
        } else {
            match file.metadata()?.modified()?.duration_since(UNIX_EPOCH) {
                Ok(elapsed) => elapsed.as_secs() / 86_400,
                Err(_) => 0,
            }
        };
        Ok(Self {
            prop: SinkProp::default(),
            dir: cfg.dir.clone(),
            max_bytes: cfg.rotate_bytes,
            keep_files: usize::from(cfg.keep_files),
            state: Mutex::new(DailySizeState {
                file: Some(file),
                size,
                utc_day: day,
            }),
        })
    }

    fn rotate(&self, state: &mut DailySizeState) -> std::io::Result<()> {
        if let Some(file) = state.file.as_mut() {
            file.flush()?;
        }
        drop(state.file.take());
        if self.keep_files > 1 {
            for index in (1..self.keep_files).rev() {
                let destination = self.dir.join(rotated_name(index));
                if destination.exists() {
                    fs::remove_file(&destination)?;
                }
                let source = if index == 1 {
                    self.dir.join(LOG_FILE_NAME)
                } else {
                    self.dir.join(rotated_name(index - 1))
                };
                if source.exists() {
                    fs::rename(source, destination)?;
                }
            }
        } else {
            let active = self.dir.join(LOG_FILE_NAME);
            if active.exists() {
                fs::remove_file(active)?;
            }
        }
        state.file = Some(
            OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(self.dir.join(LOG_FILE_NAME))?,
        );
        state.size = 0;
        state.utc_day = utc_day();
        Ok(())
    }
}

impl GetSinkProp for DailySizeSink {
    fn prop(&self) -> &SinkProp {
        &self.prop
    }
}

impl Sink for DailySizeSink {
    fn log(&self, record: &Record) -> spdlog::Result<()> {
        let mut state = lock(&self.state);
        let payload_bytes = u64::try_from(record.payload().len()).map_err(|_| {
            spdlog::Error::WriteRecord(std::io::Error::other(
                "log record length exceeds the file size representation",
            ))
        })?;
        let line_bytes = payload_bytes.saturating_add(1);
        if state.utc_day != utc_day()
            || (state.size > 0 && state.size.saturating_add(line_bytes) > self.max_bytes)
        {
            self.rotate(&mut state)
                .map_err(spdlog::Error::WriteRecord)?;
        }
        let file = state.file.as_mut().ok_or_else(|| {
            spdlog::Error::WriteRecord(std::io::Error::other("log file is not open"))
        })?;
        file.write_all(record.payload().as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .map_err(spdlog::Error::WriteRecord)?;
        state.size = state.size.saturating_add(line_bytes);
        Ok(())
    }

    fn flush(&self) -> spdlog::Result<()> {
        let mut state = lock(&self.state);
        match state.file.as_mut() {
            Some(file) => file.flush().map_err(spdlog::Error::FlushBuffer),
            None => Ok(()),
        }
    }
}

fn rotated_name(index: usize) -> String {
    format!("polaris_{index}.log")
}

fn utc_day() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs() / 86_400,
        Err(_) => 0,
    }
}

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

/// Wraps the daily-size file sink with a bounded retry. After a record fails every attempt the sink
/// is "degraded" and later records get a single attempt (no sleeping) until one succeeds, so a
/// dead disk costs the drain thread microseconds per record, not milliseconds.
struct RetryingSink {
    inner: Arc<DailySizeSink>,
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
    let file =
        Arc::new(DailySizeSink::new(cfg).map_err(|err| LogError::SinkInit(err.to_string()))?);
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

#[cfg(test)]
mod tests {
    use super::{DailySizeSink, utc_day};
    use crate::init::LogConfig;
    use crate::ring::lock;
    use std::io::Write as _;

    #[test]
    fn daily_rollover_archives_and_reopens_the_active_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let mut config = LogConfig::new(directory.path());
        config.keep_files = 3;
        let sink = DailySizeSink::new(&config)?;
        {
            let mut state = lock(&sink.state);
            if let Some(file) = state.file.as_mut() {
                file.write_all(b"previous day\n")?;
                state.size = 13;
                state.utc_day = utc_day().saturating_sub(1);
            }
            sink.rotate(&mut state)?;
        }
        assert_eq!(
            std::fs::read(directory.path().join("polaris_1.log"))?,
            b"previous day\n"
        );
        assert_eq!(
            std::fs::metadata(directory.path().join("polaris.log"))?.len(),
            0
        );
        Ok(())
    }
}
