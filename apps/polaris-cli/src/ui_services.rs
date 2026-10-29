//! Isolated cold-path services used by the CLI UI.
//!
//! Native clipboard access always runs on its owned worker. Workspace snapshots delegate
//! traversal and path checks to `polaris-tools-builtin`; results are bounded and generation tagged.

#[cfg(target_os = "linux")]
use std::ffi::OsString;
use std::{
    env,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use polaris_config::{CliColorDepth, CliColorPolicy, CliPanelDefault, CliPreferences, CliTheme};
use polaris_tools_builtin::{GlobInput, ToolLimits, glob};

/// Clipboard text cap; images are never requested or written.
pub const MAX_CLIPBOARD_TEXT_BYTES: usize = 1_048_576;
const SERVICE_QUEUE_CAPACITY: usize = 8;
const MAX_SERVICE_TIMEOUT: Duration = Duration::from_secs(30);
const INDEX_SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

/// Runtime-only preference overrides. This type has no persistence operation by design.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PreferenceOverrides {
    /// Optional runtime canvas theme.
    pub theme: Option<CliTheme>,
    /// Optional runtime color policy.
    pub color: Option<CliColorPolicy>,
    /// Optional runtime color depth.
    pub color_depth: Option<CliColorDepth>,
    /// Optional runtime ASCII drawing preference.
    pub ascii: Option<bool>,
    /// Optional runtime motion preference.
    pub reduce_motion: Option<bool>,
    /// Optional runtime panel visibility.
    pub panel: Option<CliPanelDefault>,
}

impl PreferenceOverrides {
    /// Resolve runtime/session overrides over persisted CLI preferences in memory.
    #[must_use]
    pub fn apply(self, base: CliPreferences) -> CliPreferences {
        CliPreferences {
            theme: match self.theme {
                Some(value) => value,
                None => base.theme,
            },
            color: match self.color {
                Some(value) => value,
                None => base.color,
            },
            color_depth: match self.color_depth {
                Some(value) => value,
                None => base.color_depth,
            },
            ascii: match self.ascii {
                Some(value) => value,
                None => base.ascii,
            },
            reduce_motion: match self.reduce_motion {
                Some(value) => value,
                None => base.reduce_motion,
            },
            panel: match self.panel {
                Some(value) => value,
                None => base.panel,
            },
        }
    }
}

/// Failures reported by an isolated UI service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ServiceError {
    /// The worker could not be created or the native clipboard is unavailable.
    #[error("service is unavailable")]
    Unavailable,
    /// A request exceeded its configured wait bound.
    #[error("service request timed out")]
    TimedOut,
    /// The request was retired because its UI generation is no longer active.
    #[error("service request was cancelled")]
    Cancelled,
    /// Text exceeded the one MiB safety limit.
    #[error("clipboard text exceeds the configured limit")]
    TextTooLarge,
    /// A bounded service queue is full or has been shut down.
    #[error("service queue is full or closed")]
    QueueFull,
    /// The request or configured limits are invalid.
    #[error("invalid service request or limits")]
    InvalidInput,
}

/// Clipboard operation result delivered through polling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardResult {
    /// Text retrieved from the system clipboard.
    TextRead {
        /// UI generation that requested the operation.
        generation: u64,
        /// Clipboard text or a generic service failure.
        result: Result<String, ServiceError>,
    },
    /// Text copied to the system clipboard.
    TextCopied {
        /// UI generation that requested the operation.
        generation: u64,
        /// Success or a generic service failure.
        result: Result<(), ServiceError>,
    },
}

enum ClipboardCommand {
    Read {
        generation: u64,
        deadline: Instant,
    },
    Copy {
        generation: u64,
        deadline: Instant,
        text: String,
    },
    Shutdown,
}

/// Nonblocking interface to the platform text clipboard.
///
/// The worker never accesses image clipboard data. Cancellation retires queued and future results
/// for the currently active generation; a native OS clipboard call already in progress cannot be
/// interrupted, but it cannot block the UI thread or deliver a stale result.
#[derive(Debug)]
pub struct ClipboardService {
    requests: SyncSender<ClipboardCommand>,
    results: Receiver<ClipboardResult>,
    active_generation: Arc<Mutex<u64>>,
    timeout: Duration,
    max_text_bytes: usize,
    closed: bool,
    worker: Option<JoinHandle<()>>,
}

impl ClipboardService {
    /// Start the owned native clipboard worker without reading or changing clipboard contents.
    ///
    /// # Errors
    /// Returns [`ServiceError::InvalidInput`] for zero, overlong, or over-limit configuration.
    pub fn new(timeout: Duration, max_text_bytes: usize) -> Result<Self, ServiceError> {
        if timeout.is_zero()
            || timeout > MAX_SERVICE_TIMEOUT
            || max_text_bytes == 0
            || max_text_bytes > MAX_CLIPBOARD_TEXT_BYTES
        {
            return Err(ServiceError::InvalidInput);
        }
        let (request_tx, request_rx) = mpsc::sync_channel(SERVICE_QUEUE_CAPACITY);
        let (result_tx, result_rx) = mpsc::sync_channel(SERVICE_QUEUE_CAPACITY);
        let active_generation = Arc::new(Mutex::new(0_u64));
        let worker_generation = Arc::clone(&active_generation);
        let worker_max_text_bytes = max_text_bytes;
        let worker = thread::Builder::new()
            .name("polaris-cli-clipboard".to_owned())
            .spawn(move || {
                clipboard_worker(
                    request_rx,
                    result_tx,
                    worker_generation,
                    worker_max_text_bytes,
                )
            })
            .map_err(|_| ServiceError::Unavailable)?;
        Ok(Self {
            requests: request_tx,
            results: result_rx,
            active_generation,
            timeout,
            max_text_bytes,
            closed: false,
            worker: Some(worker),
        })
    }

    /// Queue a system clipboard text read for `generation`.
    ///
    /// # Errors
    /// Returns invalid input, queue full, or closed-service errors. No clipboard work happens here.
    pub fn request_read(&mut self, generation: u64) -> Result<(), ServiceError> {
        let deadline = self.deadline()?;
        self.activate(generation)?;
        self.send(ClipboardCommand::Read {
            generation,
            deadline,
        })
    }

    /// Queue a text-only system clipboard write for `generation`.
    ///
    /// # Errors
    /// Returns invalid input, queue full, or closed-service errors. Text above one MiB is rejected.
    pub fn request_copy(&mut self, generation: u64, text: String) -> Result<(), ServiceError> {
        if text.is_empty() {
            return Err(ServiceError::InvalidInput);
        }
        if text.len() > self.max_text_bytes {
            return Err(ServiceError::TextTooLarge);
        }
        let deadline = self.deadline()?;
        self.activate(generation)?;
        self.send(ClipboardCommand::Copy {
            generation,
            deadline,
            text,
        })
    }

    /// Retire `generation`; a currently running native call may finish in the worker, but its
    /// result is discarded and no queued request for the retired generation will run.
    pub fn cancel(&mut self, generation: u64) {
        let mut active = lock_recover(&self.active_generation);
        if *active == generation {
            *active = 0;
        }
    }

    /// Poll one current-generation clipboard result without waiting.
    #[must_use]
    pub fn try_recv(&self) -> Option<ClipboardResult> {
        loop {
            match self.results.try_recv() {
                Ok(result) if self.result_is_current(&result) => return Some(result),
                Ok(_) => continue,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return None,
            }
        }
    }

    /// Ask the worker to close. A native OS call already blocked in the OS cannot be joined safely.
    pub fn shutdown(&mut self) {
        if !self.closed {
            self.closed = true;
            *lock_recover(&self.active_generation) = 0;
            let _ = self.requests.send(ClipboardCommand::Shutdown);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn deadline(&self) -> Result<Instant, ServiceError> {
        Instant::now()
            .checked_add(self.timeout)
            .ok_or(ServiceError::InvalidInput)
    }

    fn activate(&self, generation: u64) -> Result<(), ServiceError> {
        if self.closed || generation == 0 {
            return Err(ServiceError::InvalidInput);
        }
        Ok(())
    }

    fn send(&self, command: ClipboardCommand) -> Result<(), ServiceError> {
        if self.closed {
            return Err(ServiceError::Unavailable);
        }
        let generation = match &command {
            ClipboardCommand::Read { generation, .. }
            | ClipboardCommand::Copy { generation, .. } => *generation,
            ClipboardCommand::Shutdown => return Err(ServiceError::InvalidInput),
        };
        let mut active_generation = lock_recover(&self.active_generation);
        let previous_generation = *active_generation;
        *active_generation = generation;
        match self.requests.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                *active_generation = previous_generation;
                Err(ServiceError::QueueFull)
            }
            Err(TrySendError::Disconnected(_)) => {
                *active_generation = previous_generation;
                Err(ServiceError::Unavailable)
            }
        }
    }

    fn result_is_current(&self, result: &ClipboardResult) -> bool {
        let generation = match result {
            ClipboardResult::TextRead { generation, .. }
            | ClipboardResult::TextCopied { generation, .. } => *generation,
        };
        *lock_recover(&self.active_generation) == generation
    }
}

impl Drop for ClipboardService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn clipboard_worker(
    requests: Receiver<ClipboardCommand>,
    results: SyncSender<ClipboardResult>,
    active_generation: Arc<Mutex<u64>>,
    max_text_bytes: usize,
) {
    while let Ok(command) = requests.recv() {
        match command {
            ClipboardCommand::Shutdown => break,
            ClipboardCommand::Read {
                generation,
                deadline,
            } => {
                if !is_generation_active(&active_generation, generation) {
                    continue;
                }
                let result = clipboard_process(
                    false,
                    None,
                    deadline,
                    generation,
                    max_text_bytes,
                    &active_generation,
                )
                .and_then(|bytes| String::from_utf8(bytes).map_err(|_| ServiceError::Unavailable));
                send_clipboard_result(
                    &results,
                    &active_generation,
                    ClipboardResult::TextRead { generation, result },
                );
            }
            ClipboardCommand::Copy {
                generation,
                deadline,
                text,
            } => {
                if !is_generation_active(&active_generation, generation) {
                    continue;
                }
                let result = clipboard_process(
                    true,
                    Some(text),
                    deadline,
                    generation,
                    max_text_bytes,
                    &active_generation,
                )
                .map(|_| ());
                send_clipboard_result(
                    &results,
                    &active_generation,
                    ClipboardResult::TextCopied { generation, result },
                );
            }
        }
    }
}

fn clipboard_process(
    write: bool,
    input: Option<String>,
    deadline: Instant,
    generation: u64,
    max_text_bytes: usize,
    active_generation: &Mutex<u64>,
) -> Result<Vec<u8>, ServiceError> {
    if Instant::now() >= deadline {
        return Err(ServiceError::TimedOut);
    }
    if !is_generation_active(active_generation, generation) {
        return Err(ServiceError::Cancelled);
    }
    let mut command = clipboard_command(write)?;
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if write { Stdio::null() } else { Stdio::piped() })
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|_| ServiceError::Unavailable)?;
    let writer = input.and_then(|text| {
        child
            .stdin
            .take()
            .map(|mut stdin| thread::spawn(move || stdin.write_all(text.as_bytes()).is_ok()))
    });
    let (output_tx, output_rx) = mpsc::sync_channel(1);
    let reader = if write {
        None
    } else {
        let stdout = take_child_stdout(&mut child)?;
        Some(thread::spawn(move || {
            let outcome = match u64::try_from(max_text_bytes.saturating_add(1)) {
                Ok(read_limit) => {
                    let mut stdout = stdout.take(read_limit);
                    let mut bytes = Vec::new();
                    stdout.read_to_end(&mut bytes).map(|_| bytes)
                }
                Err(_) => Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "clipboard text bound exceeds platform size",
                )),
            };
            let _ = output_tx.send(outcome);
        }))
    };
    let mut status = None;
    let mut output = None;
    let mut output_too_large = false;
    let result = loop {
        if !is_generation_active(active_generation, generation) {
            break Err(ServiceError::Cancelled);
        }
        if Instant::now() >= deadline {
            break Err(ServiceError::TimedOut);
        }
        if !write {
            match output_rx.try_recv() {
                Ok(Ok(bytes)) => {
                    if bytes.len() > max_text_bytes {
                        output_too_large = true;
                    } else {
                        output = Some(bytes);
                    }
                }
                Ok(Err(_)) => break Err(ServiceError::Unavailable),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
            }
        }
        match child.try_wait() {
            Ok(Some(exit_status)) => {
                status = Some(exit_status);
                break Ok(());
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => break Err(ServiceError::Unavailable),
        }
    };
    if result.is_err() || output_too_large {
        stop_child(&mut child);
    }
    if status.is_none() {
        status = child.wait().ok();
    }
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    if let Some(reader) = reader {
        let _ = reader.join();
    }
    result?;
    if output_too_large {
        return Err(ServiceError::TextTooLarge);
    }
    if !status.as_ref().is_some_and(ExitStatus::success) {
        return Err(ServiceError::Unavailable);
    }
    match output {
        Some(bytes) => Ok(bytes),
        None if write => Ok(Vec::new()),
        None => match output_rx.try_recv() {
            Ok(Ok(bytes)) if bytes.len() <= max_text_bytes => Ok(bytes),
            Ok(Ok(_)) => Err(ServiceError::TextTooLarge),
            Ok(Err(_)) | Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                Err(ServiceError::Unavailable)
            }
        },
    }
}

fn clipboard_command(write: bool) -> Result<Command, ServiceError> {
    #[cfg(windows)]
    {
        let system_root = env::var_os("SystemRoot").ok_or(ServiceError::Unavailable)?;
        let system_root = PathBuf::from(system_root);
        if !system_root.is_absolute() {
            return Err(ServiceError::Unavailable);
        }
        let executable = system_root
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        let mut command = Command::new(executable);
        command
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-STA")
            .arg("-Command");
        if write {
            command.arg("$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Windows.Forms; [Console]::InputEncoding = New-Object System.Text.UTF8Encoding($false); $text = [Console]::In.ReadToEnd(); [Windows.Forms.Clipboard]::SetText($text)");
        } else {
            command.arg("$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Windows.Forms; [Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false); [Console]::Out.Write([Windows.Forms.Clipboard]::GetText())");
        }
        Ok(command)
    }
    #[cfg(target_os = "linux")]
    {
        let wayland = env::var_os("WAYLAND_DISPLAY").is_some();
        let executable: OsString = if wayland {
            OsString::from(if write { "wl-copy" } else { "wl-paste" })
        } else {
            OsString::from("xclip")
        };
        let mut command = Command::new(executable);
        if wayland {
            if !write {
                command.arg("--no-newline");
            }
        } else {
            command.arg("-selection").arg("clipboard");
            command.arg(if write { "-i" } else { "-o" });
        }
        Ok(command)
    }
    #[cfg(target_os = "macos")]
    {
        Ok(Command::new(if write { "pbcopy" } else { "pbpaste" }))
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = write;
        Err(ServiceError::Unavailable)
    }
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn take_child_stdout(child: &mut Child) -> Result<std::process::ChildStdout, ServiceError> {
    match child.stdout.take() {
        Some(stdout) => Ok(stdout),
        None => {
            stop_child(child);
            Err(ServiceError::Unavailable)
        }
    }
}

fn is_generation_active(active_generation: &Mutex<u64>, generation: u64) -> bool {
    *lock_recover(active_generation) == generation
}

fn send_clipboard_result(
    results: &SyncSender<ClipboardResult>,
    active_generation: &Mutex<u64>,
    result: ClipboardResult,
) {
    let generation = match &result {
        ClipboardResult::TextRead { generation, .. }
        | ClipboardResult::TextCopied { generation, .. } => *generation,
    };
    if is_generation_active(active_generation, generation) {
        let _ = results.try_send(result);
    }
}

/// Bounds for one workspace path snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexLimits {
    /// Maximum number of returned paths.
    pub max_paths: usize,
    /// Maximum combined UTF-8 bytes across returned paths.
    pub max_snapshot_bytes: usize,
    /// Maximum traversal time.
    pub timeout: Duration,
}

impl Default for IndexLimits {
    fn default() -> Self {
        Self {
            max_paths: 2048,
            max_snapshot_bytes: MAX_CLIPBOARD_TEXT_BYTES,
            timeout: Duration::from_secs(2),
        }
    }
}

/// A bounded workspace-relative path snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexSnapshot {
    /// Request generation.
    pub generation: u64,
    /// Sorted, normalized workspace-relative file paths.
    pub paths: Vec<String>,
    /// True when walker, count, byte, or time limits omitted entries.
    pub truncated: bool,
}

/// Result of a workspace snapshot request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexResult {
    /// Request generation.
    pub generation: u64,
    /// Snapshot, or a generic traversal failure.
    pub result: Result<IndexSnapshot, ServiceError>,
}

enum IndexCommand {
    Snapshot { generation: u64 },
}

/// Owns a bounded ignore-aware workspace path index worker.
#[derive(Debug)]
pub struct WorkspaceFileIndex {
    root: PathBuf,
    limits: IndexLimits,
    requests: Option<SyncSender<IndexCommand>>,
    results: Receiver<IndexResult>,
    finished: Receiver<()>,
    active_generation: Arc<Mutex<u64>>,
    closed: bool,
    worker: Option<JoinHandle<()>>,
}

impl WorkspaceFileIndex {
    /// Start the worker without scanning the workspace.
    ///
    /// # Errors
    /// Returns [`ServiceError::InvalidInput`] when bounds are zero or exceed service caps.
    pub fn new(root: PathBuf, limits: IndexLimits) -> Result<Self, ServiceError> {
        if root.as_os_str().is_empty()
            || limits.max_paths == 0
            || limits.max_paths > 16_384
            || limits.max_snapshot_bytes == 0
            || limits.max_snapshot_bytes > MAX_CLIPBOARD_TEXT_BYTES
            || limits.timeout.is_zero()
            || limits.timeout > MAX_SERVICE_TIMEOUT
        {
            return Err(ServiceError::InvalidInput);
        }
        let (request_tx, request_rx) = mpsc::sync_channel(SERVICE_QUEUE_CAPACITY);
        let (result_tx, result_rx) = mpsc::sync_channel(SERVICE_QUEUE_CAPACITY);
        let (finished_tx, finished_rx) = mpsc::sync_channel(1);
        let active_generation = Arc::new(Mutex::new(0_u64));
        let worker_generation = Arc::clone(&active_generation);
        let worker_root = root.clone();
        let worker = thread::Builder::new()
            .name("polaris-cli-file-index".to_owned())
            .spawn(move || {
                file_index_worker(
                    worker_root,
                    limits,
                    request_rx,
                    result_tx,
                    worker_generation,
                );
                let _ = finished_tx.try_send(());
            })
            .map_err(|_| ServiceError::Unavailable)?;
        Ok(Self {
            root,
            limits,
            requests: Some(request_tx),
            results: result_rx,
            finished: finished_rx,
            active_generation,
            closed: false,
            worker: Some(worker),
        })
    }

    /// Queue a bounded path snapshot for `generation`.
    ///
    /// # Errors
    /// Returns invalid input, queue full, or closed-service errors.
    pub fn request(&mut self, generation: u64) -> Result<(), ServiceError> {
        if self.closed || generation == 0 {
            return Err(ServiceError::InvalidInput);
        }
        let mut active_generation = lock_recover(&self.active_generation);
        let previous_generation = *active_generation;
        *active_generation = generation;
        let Some(requests) = self.requests.as_ref() else {
            return Err(ServiceError::Unavailable);
        };
        match requests.try_send(IndexCommand::Snapshot { generation }) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                *active_generation = previous_generation;
                Err(ServiceError::QueueFull)
            }
            Err(TrySendError::Disconnected(_)) => {
                *active_generation = previous_generation;
                Err(ServiceError::Unavailable)
            }
        }
    }

    /// Retire a generation. A scan already in progress remains bounded by its timeout; its result
    /// is discarded before it reaches the UI.
    pub fn cancel(&mut self, generation: u64) {
        let mut active = lock_recover(&self.active_generation);
        if *active == generation {
            *active = 0;
        }
    }

    /// Poll one current-generation index result without waiting.
    #[must_use]
    pub fn try_recv(&self) -> Option<IndexResult> {
        loop {
            match self.results.try_recv() {
                Ok(result) if *lock_recover(&self.active_generation) == result.generation => {
                    return Some(result);
                }
                Ok(_) => continue,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return None,
            }
        }
    }

    /// Root being indexed.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Configured snapshot bounds.
    #[must_use]
    pub const fn limits(&self) -> IndexLimits {
        self.limits
    }

    /// Ask the worker to close. A bounded in-progress filesystem scan may finish before exit.
    pub fn shutdown(&mut self) {
        if !self.closed {
            self.closed = true;
            *lock_recover(&self.active_generation) = 0;
            self.requests.take();
            if let Some(worker) = self.worker.take()
                && matches!(self.finished.recv_timeout(INDEX_SHUTDOWN_WAIT), Ok(()))
            {
                let _ = worker.join();
            }
        }
    }
}

impl Drop for WorkspaceFileIndex {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn file_index_worker(
    root: PathBuf,
    limits: IndexLimits,
    requests: Receiver<IndexCommand>,
    results: SyncSender<IndexResult>,
    active_generation: Arc<Mutex<u64>>,
) {
    while let Ok(command) = requests.recv() {
        match command {
            IndexCommand::Snapshot { generation } => {
                if !is_generation_active(&active_generation, generation) {
                    continue;
                }
                let result = build_snapshot(&root, limits, generation);
                if is_generation_active(&active_generation, generation) {
                    let _ = results.try_send(IndexResult { generation, result });
                }
            }
        }
    }
}

fn build_snapshot(
    root: &Path,
    limits: IndexLimits,
    generation: u64,
) -> Result<IndexSnapshot, ServiceError> {
    let tool_limits = ToolLimits {
        max_read_bytes: 1,
        max_results: limits.max_paths.saturating_add(1),
        max_output_bytes: match u64::try_from(limits.max_snapshot_bytes) {
            Ok(value) => value,
            Err(_) => return Err(ServiceError::InvalidInput),
        },
        max_input_bytes: 1,
        regex_size_limit: 1,
        timeout: limits.timeout,
    };
    let mut output = glob(
        root,
        &GlobInput {
            pattern: "**/*".to_owned(),
        },
        &tool_limits,
    )
    .map_err(|_| ServiceError::Unavailable)?;
    output
        .rows
        .sort_by(|left, right| left.path.cmp(&right.path));
    let mut paths = Vec::with_capacity(output.rows.len().min(limits.max_paths));
    let mut bytes = 0_usize;
    let mut truncated = output.truncated;
    for row in output.rows {
        let path_bytes = row.path.len();
        let Some(next_bytes) = bytes.checked_add(path_bytes) else {
            truncated = true;
            break;
        };
        if paths.len() >= limits.max_paths || next_bytes > limits.max_snapshot_bytes {
            truncated = true;
            break;
        }
        bytes = next_bytes;
        paths.push(row.path);
    }
    Ok(IndexSnapshot {
        generation,
        paths,
        truncated,
    })
}

fn lock_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
#[path = "../../../tests/apps/polaris-cli/src/ui_services.rs"]
mod tests;
