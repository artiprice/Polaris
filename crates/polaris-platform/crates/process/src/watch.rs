//! Debounced filesystem events with explicit overflow reporting.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use notify::{Event, RecursiveMode, Watcher};

/// Filesystem watcher configuration with bounded callback buffering.
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// Debounce interval used when coalescing event batches.
    pub debounce: Duration,
    /// Maximum callback events retained before an overflow forces a full rescan.
    pub capacity: usize,
    /// Directory names excluded from emitted changes.
    pub ignored_directories: Vec<String>,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(80),
            capacity: 4096,
            ignored_directories: vec![
                ".git".to_owned(),
                "target".to_owned(),
                ".polaris".to_owned(),
            ],
        }
    }
}

/// Why the watcher requires the consumer to reconcile from a fresh filesystem scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RescanReason {
    /// The operating-system backend indicated that events were lost.
    BackendOverflow,
    /// The bounded callback queue filled before the consumer could drain it.
    QueueOverflow,
    /// The backend reported an error, so incremental state is no longer trustworthy.
    BackendError,
}

/// Debounced path changes or an explicit rescan request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsEvent {
    /// Paths changed during one debounce interval.
    Changed(Vec<PathBuf>),
    /// At least one event was lost or invalidated.
    RescanRequired { reason: RescanReason },
}

/// Active watcher. Dropping it stops OS event delivery.
pub struct FsWatcher {
    _watcher: notify::RecommendedWatcher,
    receiver: mpsc::Receiver<notify::Result<Event>>,
    overflowed: Arc<AtomicBool>,
    root: PathBuf,
    config: WatchConfig,
}

impl std::fmt::Debug for FsWatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FsWatcher")
            .field("root", &self.root)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl FsWatcher {
    /// Waits for a debounced change batch, an overflow signal, or timeout.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<FsEvent>, WatchError> {
        if self.overflowed.swap(false, Ordering::AcqRel) {
            return Ok(Some(FsEvent::RescanRequired {
                reason: RescanReason::QueueOverflow,
            }));
        }
        let first = match self.receiver.recv_timeout(timeout) {
            Ok(event) => event,
            Err(mpsc::RecvTimeoutError::Timeout) => return Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(WatchError::Stopped),
        };
        let mut paths = Vec::new();
        let mut should_rescan = None;
        collect_event(
            first,
            &self.root,
            &self.config.ignored_directories,
            &mut paths,
            &mut should_rescan,
        );
        let deadline = Instant::now()
            .checked_add(self.config.debounce)
            .ok_or(WatchError::InvalidDuration)?;
        loop {
            if let Some(reason) = should_rescan {
                return Ok(Some(FsEvent::RescanRequired { reason }));
            }
            if self.overflowed.swap(false, Ordering::AcqRel) {
                return Ok(Some(FsEvent::RescanRequired {
                    reason: RescanReason::QueueOverflow,
                }));
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match self
                .receiver
                .recv_timeout(deadline.saturating_duration_since(now))
            {
                Ok(event) => collect_event(
                    event,
                    &self.root,
                    &self.config.ignored_directories,
                    &mut paths,
                    &mut should_rescan,
                ),
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(WatchError::Stopped),
            }
        }
        paths.sort();
        paths.dedup();
        if paths.is_empty() {
            Ok(None)
        } else {
            Ok(Some(FsEvent::Changed(paths)))
        }
    }
}

/// Watch setup and consumer errors.
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    /// The requested root does not exist or is not a directory.
    #[error("watch root must be an existing directory")]
    InvalidRoot,
    /// The backend could not be initialized or attached.
    #[error("filesystem watcher failed: {0}")]
    Backend(String),
    /// The watcher has stopped delivering events.
    #[error("filesystem watcher stopped")]
    Stopped,
    /// Debounce duration exceeded the monotonic clock range.
    #[error("debounce duration exceeded timer range")]
    InvalidDuration,
}

/// Watches a directory recursively; overflow and backend errors become rescan events.
pub fn watch(root: &Path, config: WatchConfig) -> Result<FsWatcher, WatchError> {
    if !root.is_dir() || config.capacity == 0 {
        return Err(WatchError::InvalidRoot);
    }
    let (sender, receiver) = mpsc::sync_channel(config.capacity);
    let overflowed = Arc::new(AtomicBool::new(false));
    let callback_overflow = Arc::clone(&overflowed);
    let watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
        enqueue_event(&sender, &callback_overflow, event);
    })
    .map_err(|error| WatchError::Backend(error.to_string()))?;
    let mut watcher = watcher;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|error| WatchError::Backend(error.to_string()))?;
    Ok(FsWatcher {
        _watcher: watcher,
        receiver,
        overflowed,
        root: root.to_path_buf(),
        config,
    })
}

fn enqueue_event(
    sender: &mpsc::SyncSender<notify::Result<Event>>,
    overflowed: &AtomicBool,
    event: notify::Result<Event>,
) {
    if sender.try_send(event).is_err() {
        overflowed.store(true, Ordering::Release);
    }
}

fn collect_event(
    event: notify::Result<Event>,
    root: &Path,
    ignored: &[String],
    paths: &mut Vec<PathBuf>,
    rescan: &mut Option<RescanReason>,
) {
    match event {
        Ok(event) if event.need_rescan() => *rescan = Some(RescanReason::BackendOverflow),
        Ok(event) => {
            for path in event.paths {
                if is_ignored(&path, root, ignored) {
                    continue;
                }
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
        Err(_) => *rescan = Some(RescanReason::BackendError),
    }
}

fn is_ignored(path: &Path, root: &Path, ignored: &[String]) -> bool {
    match path.strip_prefix(root) {
        Ok(relative) => relative.components().any(|component| {
            let name = component.as_os_str().to_string_lossy();
            ignored
                .iter()
                .any(|ignored_name| ignored_name == name.as_ref())
        }),
        Err(_) => false,
    }
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-platform/crates/process/src/watch.rs"]
mod tests;
