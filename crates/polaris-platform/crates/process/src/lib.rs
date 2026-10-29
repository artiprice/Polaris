//! Native process-tree operations implementing the SDK's `PlatformOps` contract.
//!
//! The adapter retains each spawned child wrapper under an opaque handle. Runtime operations
//! resolve that handle and retain the OS process-group or Job Object wrapper, so they do not
//! re-resolve a PID that may have been reused. Inherited environment variables are cleared;
//! `SpawnSpec::env` is the complete child environment allowlist, and secret-like names are
//! rejected. Memory pressure uses available RAM ratios of 5% and 15% for Critical and Elevated.

mod probe;
mod pty;
mod resolve;
mod watch;

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-platform/crates/process/src/bootstrap_tests.rs"]
mod bootstrap_tests;

mod endpoint;

pub use probe::{ExecOutput, run_probe, run_probe_with_env};
pub use pty::{PtyOutput, PtySession, PtySize, spawn_pty};
pub use resolve::{ProgramResolution, ResolveError, resolve_program};
pub use watch::{FsEvent, FsWatcher, RescanReason, WatchConfig, watch};

use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use polaris_sdk::registry::{
    LocalEndpointOwnership, MemoryPressure, PlatformError, PlatformOps, ProcessExit, ProcessTree,
    SpawnSpec, StopOutcome, WorkerBootstrapPlatform,
};
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use secrecy::ExposeSecret;
use sysinfo::System;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{Instant, sleep, timeout_at};

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;

#[derive(Debug)]
struct ProcessRecord {
    root_pid: u32,
    child: Arc<AsyncMutex<Box<dyn ChildWrapper>>>,
    state: Arc<ProcessState>,
}

type RetainedProcess = Arc<AsyncMutex<Box<dyn ChildWrapper>>>;
type ProcessRecordHandle = (RetainedProcess, Arc<ProcessState>);

#[derive(Debug, Default)]
struct ProcessState {
    bootstrap_attempted: AtomicBool,
    force_stop_requested: AtomicBool,
}

/// Native process-tree adapter for Windows and Unix platforms.
#[derive(Debug)]
pub struct ProcessPlatform {
    next_handle: Mutex<u64>,
    children: Mutex<HashMap<u64, ProcessRecord>>,
}

impl Default for ProcessPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessPlatform {
    /// Creates a process adapter with no live child trees.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_handle: Mutex::new(1),
            children: Mutex::new(HashMap::new()),
        }
    }

    fn allocate_handle(&self) -> Result<u64, PlatformError> {
        let mut next = self
            .next_handle
            .lock()
            .map_err(|_| PlatformError::Io("process handle counter lock poisoned".to_owned()))?;
        let handle = *next;
        *next = next
            .checked_add(1)
            .ok_or_else(|| PlatformError::Io("process handle space exhausted".to_owned()))?;
        Ok(handle)
    }

    fn record(&self, tree: &ProcessTree) -> Result<ProcessRecordHandle, PlatformError> {
        let records = self
            .children
            .lock()
            .map_err(|_| PlatformError::Io("process registry lock poisoned".to_owned()))?;
        let record = records
            .get(&tree.handle_id)
            .ok_or_else(|| PlatformError::NotFound(format!("process handle {}", tree.handle_id)))?;
        if record.root_pid != tree.root_pid {
            return Err(PlatformError::Policy(
                "process handle root PID mismatch".to_owned(),
            ));
        }
        Ok((Arc::clone(&record.child), Arc::clone(&record.state)))
    }

    fn remove_record(&self, tree: &ProcessTree) -> Result<(), PlatformError> {
        let mut records = self
            .children
            .lock()
            .map_err(|_| PlatformError::Io("process registry lock poisoned".to_owned()))?;
        if let Some(record) = records.get(&tree.handle_id)
            && record.root_pid != tree.root_pid
        {
            return Err(PlatformError::Policy(
                "process handle root PID mismatch".to_owned(),
            ));
        }
        records.remove(&tree.handle_id);
        Ok(())
    }
}

impl PlatformOps for ProcessPlatform {
    fn spawn_tree(&self, spec: &SpawnSpec) -> Result<ProcessTree, PlatformError> {
        if !spec.program.is_absolute() {
            return Err(PlatformError::Policy(
                "worker program path must be absolute".to_owned(),
            ));
        }
        #[cfg(windows)]
        if matches!(
            spec.program
                .extension()
                .and_then(OsStr::to_str)
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("bat" | "cmd")
        ) {
            return Err(PlatformError::Policy(
                "batch scripts require an explicit trusted command policy".to_owned(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for (key, value) in &spec.env {
            let name = key.to_str().ok_or_else(|| {
                PlatformError::Policy("worker environment key must be Unicode".to_owned())
            })?;
            if name.is_empty()
                || name.contains(['=', '\0'])
                || value.to_string_lossy().contains('\0')
            {
                return Err(PlatformError::Policy(
                    "worker environment contains a forbidden NUL or equals sign".to_owned(),
                ));
            }
            let normalized = name.to_ascii_uppercase();
            if is_secret_name(&normalized) {
                return Err(PlatformError::Policy(
                    "worker environment includes a secret-like variable".to_owned(),
                ));
            }
            if !seen.insert(normalized) {
                return Err(PlatformError::Policy(
                    "worker environment contains duplicate keys".to_owned(),
                ));
            }
        }
        let handle_id = self.allocate_handle()?;

        let mut command = CommandWrap::with_new(spec.program.as_os_str(), |_| {});
        let raw = command.command_mut();
        raw.args(&spec.args);
        raw.env_clear();
        raw.envs(spec.env.iter().map(|(key, value)| (key, value)));
        raw.stdin(std::process::Stdio::piped());
        raw.stdout(std::process::Stdio::null());
        raw.stderr(std::process::Stdio::null());
        if let Some(cwd) = &spec.cwd {
            raw.current_dir(cwd);
        }

        // The wrapper registration communicates this setting to JobObject; setting only the
        // raw Tokio command flag does not enable JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.
        command.wrap(KillOnDrop);
        #[cfg(unix)]
        command.wrap(ProcessGroup::leader());
        #[cfg(windows)]
        command.wrap(JobObject);
        let child = command
            .spawn()
            .map_err(|error| classify_io(error, spec.program.as_os_str()))?;
        let root_pid = child
            .id()
            .ok_or_else(|| PlatformError::Io("spawned process has no OS identifier".to_owned()))?;
        let child = Arc::new(AsyncMutex::new(child));
        let mut records = self
            .children
            .lock()
            .map_err(|_| PlatformError::Io("process registry lock poisoned".to_owned()))?;
        records.insert(
            handle_id,
            ProcessRecord {
                root_pid,
                child,
                state: Arc::new(ProcessState::default()),
            },
        );
        Ok(ProcessTree {
            root_pid,
            handle_id,
        })
    }

    fn graceful_stop(
        &self,
        tree: &ProcessTree,
        grace: Duration,
    ) -> impl std::future::Future<Output = Result<StopOutcome, PlatformError>> + Send {
        let record = self.record(tree);
        let tree = *tree;
        async move {
            let (child_ref, state) = record?;
            let mut child = child_ref.lock().await;
            if state.force_stop_requested.load(Ordering::Acquire) {
                apply_force_stop(&mut child, tree.root_pid)?;
                let _status = child.wait().await.map_err(platform_io)?;
                drop(child);
                self.remove_record(&tree)?;
                return Ok(StopOutcome::Killed);
            }
            if let Some(status) = child.try_wait().map_err(platform_io)? {
                let outcome = StopOutcome::Exited {
                    code: status.code(),
                };
                drop(child);
                self.remove_record(&tree)?;
                return Ok(outcome);
            }

            #[cfg(unix)]
            child.signal(15).map_err(platform_io)?;
            #[cfg(windows)]
            drop(child.stdin().take());

            let deadline = Instant::now().checked_add(grace).ok_or_else(|| {
                PlatformError::Policy("grace duration exceeds timer range".to_owned())
            })?;
            loop {
                if state.force_stop_requested.load(Ordering::Acquire) {
                    apply_force_stop(&mut child, tree.root_pid)?;
                    let _status = child.wait().await.map_err(platform_io)?;
                    drop(child);
                    self.remove_record(&tree)?;
                    return Ok(StopOutcome::Killed);
                }
                if let Some(status) = child.try_wait().map_err(platform_io)? {
                    let outcome = StopOutcome::Exited {
                        code: status.code(),
                    };
                    drop(child);
                    self.remove_record(&tree)?;
                    return Ok(outcome);
                }
                if Instant::now() >= deadline {
                    break;
                }
                sleep(Duration::from_millis(5)).await;
            }
            child.start_kill().map_err(platform_io)?;
            child.wait().await.map_err(platform_io)?;
            drop(child);
            self.remove_record(&tree)?;
            Ok(StopOutcome::Killed)
        }
    }

    fn kill_tree(&self, tree: &ProcessTree) -> Result<(), PlatformError> {
        let (child_ref, state) = self.record(tree)?;
        state.force_stop_requested.store(true, Ordering::Release);
        match child_ref.try_lock() {
            Ok(mut child) => {
                apply_force_stop(&mut child, tree.root_pid)?;
                drop(child);
                self.remove_record(tree)
            }
            Err(_) if schedule_force_stop(Arc::clone(&child_ref), state, tree.root_pid) => {
                Err(PlatformError::Io(
                    "process tree force-stop is pending; OS termination is not confirmed"
                        .to_owned(),
                ))
            }
            Err(_) => Err(PlatformError::Io(
                "process tree force-stop is pending but no async cleanup runtime is available"
                    .to_owned(),
            )),
        }
    }

    fn try_wait_tree(
        &self,
        tree: &ProcessTree,
    ) -> impl std::future::Future<Output = Result<Option<ProcessExit>, PlatformError>> + Send {
        let record = self.record(tree);
        let tree = *tree;
        async move {
            let (child_ref, state) = record?;
            let mut child = child_ref.lock().await;
            if state.force_stop_requested.load(Ordering::Acquire) {
                apply_force_stop(&mut child, tree.root_pid)?;
            }
            let status = child.try_wait().map_err(platform_io)?;
            if state.force_stop_requested.load(Ordering::Acquire) && status.is_none() {
                apply_force_stop(&mut child, tree.root_pid)?;
            }
            drop(child);
            match status {
                Some(status) => {
                    self.remove_record(&tree)?;
                    Ok(Some(ProcessExit {
                        code: status.code(),
                    }))
                }
                None => Ok(None),
            }
        }
    }

    fn memory_pressure(&self) -> Result<MemoryPressure, PlatformError> {
        let mut system = System::new();
        system.refresh_memory();
        pressure_from_memory(system.available_memory(), system.total_memory())
    }
}

impl WorkerBootstrapPlatform for ProcessPlatform {
    fn bootstrap_tree(
        &self,
        tree: &ProcessTree,
        secret: secrecy::SecretString,
        timeout: Duration,
    ) -> impl std::future::Future<Output = Result<(), PlatformError>> + Send {
        let tree = *tree;
        async move {
            let start = Instant::now();
            let deadline = start.checked_add(timeout).ok_or_else(|| {
                PlatformError::Policy("worker bootstrap timeout exceeds timer range".to_owned())
            })?;
            if timeout.is_zero() || deadline <= start {
                return Err(PlatformError::Policy(
                    "worker bootstrap timeout must be positive".to_owned(),
                ));
            }

            let (child_ref, state) = self.record(&tree)?;
            let secret_bytes = secret.expose_secret().as_bytes();
            if secret_bytes.is_empty() || secret_bytes.len() > 4096 {
                return Err(PlatformError::Policy(
                    "worker bootstrap secret length must be between 1 and 4096 bytes".to_owned(),
                ));
            }
            if state
                .bootstrap_attempted
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return Err(PlatformError::Policy(
                    "worker process bootstrap was already attempted".to_owned(),
                ));
            }

            let mut cancellation = BootstrapCancellation {
                child: Arc::clone(&child_ref),
                state: Arc::clone(&state),
                root_pid: tree.root_pid,
                armed: true,
            };
            let write_result = timeout_at(deadline, async {
                let mut child = child_ref.lock().await;
                if state.force_stop_requested.load(Ordering::Acquire) {
                    apply_force_stop(&mut child, tree.root_pid)?;
                    return Err(PlatformError::Policy(
                        "worker process has a pending force-stop request".to_owned(),
                    ));
                }
                if child.try_wait().map_err(platform_io)?.is_some() {
                    return Err(PlatformError::NotFound(
                        "worker process exited before bootstrap".to_owned(),
                    ));
                }
                let stdin = child.stdin().as_mut().ok_or(PlatformError::Unsupported(
                    "worker process stdin is not retained",
                ))?;
                stdin.write_all(secret_bytes).await.map_err(platform_io)
            })
            .await;

            match write_result {
                Ok(Ok(())) => {
                    cancellation.armed = false;
                    Ok(())
                }
                Ok(Err(error)) => Err(error),
                Err(_) => Err(PlatformError::Policy(
                    "worker bootstrap deadline elapsed".to_owned(),
                )),
            }
        }
    }
}

impl LocalEndpointOwnership for ProcessPlatform {
    fn owns_loopback_tcp_listener(
        &self,
        tree: &ProcessTree,
        address: std::net::SocketAddr,
    ) -> impl std::future::Future<Output = Result<bool, PlatformError>> + Send {
        let tree = *tree;
        async move {
            if !address.ip().is_loopback() {
                return Ok(false);
            }
            if self.try_wait_tree(&tree).await?.is_some() {
                return Ok(false);
            }
            let root_pid = tree.root_pid;
            let observed = tokio::task::spawn_blocking(move || {
                endpoint::owns_exact_root_listener(root_pid, address)
            })
            .await
            .map_err(|_| PlatformError::Io("listener ownership query task failed".to_owned()))??;
            if self.try_wait_tree(&tree).await?.is_some() {
                return Ok(false);
            }
            Ok(observed)
        }
    }
}

struct BootstrapCancellation {
    child: Arc<AsyncMutex<Box<dyn ChildWrapper>>>,
    state: Arc<ProcessState>,
    root_pid: u32,
    armed: bool,
}

impl Drop for BootstrapCancellation {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.state
            .force_stop_requested
            .store(true, Ordering::Release);
        match self.child.try_lock() {
            Ok(mut child) => {
                if let Err(error) = apply_force_stop(&mut child, self.root_pid) {
                    tracing::error!(pid = self.root_pid, error = %error, "failed to terminate worker after bootstrap failure");
                }
            }
            Err(_) => {
                let scheduled = schedule_force_stop(
                    Arc::clone(&self.child),
                    Arc::clone(&self.state),
                    self.root_pid,
                );
                if !scheduled {
                    tracing::error!(
                        pid = self.root_pid,
                        "worker bootstrap force-stop remains pending because no async cleanup runtime is available"
                    );
                }
            }
        }
    }
}

fn apply_force_stop(child: &mut Box<dyn ChildWrapper>, root_pid: u32) -> Result<(), PlatformError> {
    match child.start_kill() {
        Ok(()) => Ok(()),
        Err(error) => match child.try_wait().map_err(platform_io)? {
            Some(_) => Ok(()),
            None => {
                tracing::error!(pid = root_pid, error = %error, "failed to terminate owned worker process");
                Err(platform_io(error))
            }
        },
    }
}

fn schedule_force_stop(
    child: Arc<AsyncMutex<Box<dyn ChildWrapper>>>,
    state: Arc<ProcessState>,
    root_pid: u32,
) -> bool {
    // Runtime shutdown may abort this detached cleanup before it acquires the child lock. The
    // persistent flag is still observed by later tree operations; dropping the final retained
    // ChildWrapper and the supervisor's owner guard remain the synchronous backstops.
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return false;
    };
    let cleanup = runtime.spawn(async move {
        let mut child = child.lock().await;
        if state.force_stop_requested.load(Ordering::Acquire)
            && let Err(error) = apply_force_stop(&mut child, root_pid)
        {
            tracing::error!(pid = root_pid, error = %error, "async worker force-stop cleanup failed");
        }
    });
    drop(cleanup);
    true
}

impl Drop for ProcessPlatform {
    fn drop(&mut self) {
        let records = match self.children.get_mut() {
            Ok(records) => records,
            Err(poisoned) => poisoned.into_inner(),
        };
        for (_, record) in records.drain() {
            record
                .state
                .force_stop_requested
                .store(true, Ordering::Release);
            if let Ok(mut child) = record.child.try_lock() {
                if let Err(error) = child.start_kill() {
                    tracing::error!(pid = record.root_pid, error = %error, "failed to terminate process tree while dropping adapter");
                }
            } else {
                tracing::error!(
                    pid = record.root_pid,
                    "could not lock process tree while dropping adapter"
                );
            }
        }
    }
}

fn classify_io(error: std::io::Error, program: &OsStr) -> PlatformError {
    match error.kind() {
        std::io::ErrorKind::NotFound => {
            PlatformError::NotFound(program.to_string_lossy().into_owned())
        }
        std::io::ErrorKind::PermissionDenied => {
            PlatformError::PermissionDenied(program.to_string_lossy().into_owned())
        }
        _ => PlatformError::Io(error.to_string()),
    }
}

fn is_secret_name(name: &str) -> bool {
    matches!(
        name,
        "NEBIUS_API_KEY"
            | "TAVILY_API_KEY"
            | "OPENAI_API_KEY"
            | "ANTHROPIC_API_KEY"
            | "AWS_ACCESS_KEY_ID"
            | "AWS_SECRET_ACCESS_KEY"
            | "GOOGLE_APPLICATION_CREDENTIALS"
            | "AZURE_CLIENT_SECRET"
    ) || name.ends_with("_API_KEY")
        || name.ends_with("_TOKEN")
        || name.ends_with("_SECRET")
        || name.ends_with("_PASSWORD")
        || name.ends_with("_CREDENTIALS")
        || name.ends_with("_PRIVATE_KEY")
}

fn pressure_from_memory(available: u64, total: u64) -> Result<MemoryPressure, PlatformError> {
    if total == 0 {
        return Err(PlatformError::Io(
            "system returned zero total memory".to_owned(),
        ));
    }
    let available_percent = available
        .checked_mul(100)
        .and_then(|value| value.checked_div(total))
        .ok_or_else(|| PlatformError::Io("memory pressure ratio overflow".to_owned()))?;
    Ok(if available_percent <= 5 {
        MemoryPressure::Critical
    } else if available_percent <= 15 {
        MemoryPressure::Elevated
    } else {
        MemoryPressure::Normal
    })
}

fn platform_io(error: std::io::Error) -> PlatformError {
    match error.kind() {
        std::io::ErrorKind::NotFound => PlatformError::NotFound(error.to_string()),
        std::io::ErrorKind::PermissionDenied => PlatformError::PermissionDenied(error.to_string()),
        _ => PlatformError::Io(error.to_string()),
    }
}

/// Explicit child environment policy. Values are copied from a caller-provided allow-list;
/// this type never reads the host environment.
#[derive(Debug, Clone, Default)]
pub struct EnvPolicy {
    entries: Vec<(std::ffi::OsString, std::ffi::OsString)>,
}

impl EnvPolicy {
    /// Builds a policy after rejecting secret-like names and duplicate keys.
    pub fn new(
        entries: Vec<(std::ffi::OsString, std::ffi::OsString)>,
    ) -> Result<Self, PlatformError> {
        let mut seen = std::collections::HashSet::new();
        for (key, value) in &entries {
            let name = key.to_str().ok_or_else(|| {
                PlatformError::Policy("environment key must be Unicode".to_owned())
            })?;
            if name.is_empty()
                || name.contains(['=', '\0'])
                || value.to_string_lossy().contains('\0')
            {
                return Err(PlatformError::Policy(
                    "environment contains a NUL or an invalid key".to_owned(),
                ));
            }
            let normalized = name.to_ascii_uppercase();
            if is_secret_name(&normalized) {
                return Err(PlatformError::Policy(
                    "environment includes a secret-like variable".to_owned(),
                ));
            }
            if !seen.insert(normalized) {
                return Err(PlatformError::Policy(
                    "environment contains duplicate keys".to_owned(),
                ));
            }
        }
        Ok(Self { entries })
    }

    /// Returns the exact allow-list to install into a child process.
    #[must_use]
    pub fn entries(&self) -> &[(std::ffi::OsString, std::ffi::OsString)] {
        &self.entries
    }
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-platform/crates/process/src/lib.rs"]
mod tests;
