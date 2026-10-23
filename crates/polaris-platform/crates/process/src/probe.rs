//! Bounded native argv execution for read-only setup probes.

use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

#[cfg(windows)]
use process_wrap::std::JobObject;
#[cfg(unix)]
use process_wrap::std::ProcessGroup;
use process_wrap::std::{ChildWrapper, CommandWrap};

use crate::{EnvPolicy, resolve_program};
use polaris_sdk::registry::PlatformError;

const SAFE_INHERITED_ENV: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "WINDIR",
    "TEMP",
    "TMP",
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "ProgramData",
    "LANG",
    "LC_ALL",
];

/// Captured output and lifecycle result for one native argv probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutput {
    /// Exit code when the process exited normally; `None` means signal termination.
    pub exit_code: Option<i32>,
    /// Retained stdout bytes, at most the requested per-stream cap.
    pub stdout: Vec<u8>,
    /// Retained stderr bytes, at most the requested per-stream cap.
    pub stderr: Vec<u8>,
    /// Whether stdout contained bytes beyond the cap.
    pub stdout_truncated: bool,
    /// Whether stderr contained bytes beyond the cap.
    pub stderr_truncated: bool,
    /// Whether the deadline expired and the owned process tree was terminated.
    pub timed_out: bool,
}

struct CapturedStream {
    bytes: Vec<u8>,
    truncated: bool,
}

struct OwnedProbeChild {
    child: Box<dyn ChildWrapper>,
    finished: bool,
}

impl Drop for OwnedProbeChild {
    fn drop(&mut self) {
        if !self.finished {
            let _kill_result = self.child.start_kill();
            let _wait_result = self.child.wait();
        }
    }
}

/// Runs an absolute executable with static argv, bounded captured output and an explicit
/// environment policy copied from the safe process variables needed by developer tools.
///
/// There is no shell involved. Descendants share a Unix process group or Windows Job Object, and
/// the owner kills the complete tree on timeout or early return. `cap` is applied independently to
/// stdout and stderr; excess bytes are drained to avoid blocking the child and marked as truncated.
///
/// # Errors
/// Returns an error for invalid paths, rejected batch scripts, setup failures or I/O errors.
pub fn run_probe(
    program: &Path,
    argv: &[String],
    cwd: &Path,
    deadline: Duration,
    cap: usize,
) -> Result<ExecOutput, PlatformError> {
    let environment = safe_probe_environment()?;
    run_probe_with_env(program, argv, cwd, deadline, cap, &environment)
}

/// Runs an absolute executable with a caller-supplied explicit environment allow-list.
///
/// # Errors
/// Returns an error for invalid paths, rejected batch scripts, setup failures or I/O errors.
pub fn run_probe_with_env(
    program: &Path,
    argv: &[String],
    cwd: &Path,
    deadline: Duration,
    cap: usize,
    environment: &EnvPolicy,
) -> Result<ExecOutput, PlatformError> {
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err(PlatformError::Policy(
            "probe working directory must be an existing absolute directory".to_owned(),
        ));
    }
    let resolution = resolve_program(program.as_os_str(), cwd)
        .map_err(|error| PlatformError::Policy(error.to_string()))?;
    if resolution.is_batch_script {
        return Err(PlatformError::Policy(
            "native probes do not execute batch scripts".to_owned(),
        ));
    }

    let mut command = CommandWrap::with_new(resolution.path.as_os_str(), |_| {});
    let raw = command.command_mut();
    raw.args(argv);
    raw.current_dir(cwd);
    raw.env_clear();
    raw.envs(
        environment
            .entries()
            .iter()
            .map(|(key, value)| (key, value)),
    );
    raw.stdin(Stdio::null());
    raw.stdout(Stdio::piped());
    raw.stderr(Stdio::piped());
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(JobObject);

    let child = command
        .spawn()
        .map_err(|error| PlatformError::Io(error.to_string()))?;
    let mut owner = OwnedProbeChild {
        child,
        finished: false,
    };
    let stdout = owner
        .child
        .stdout()
        .take()
        .ok_or_else(|| PlatformError::Io("probe stdout pipe was not created".to_owned()))?;
    let stderr = owner
        .child
        .stderr()
        .take()
        .ok_or_else(|| PlatformError::Io("probe stderr pipe was not created".to_owned()))?;
    let stdout_reader = std::thread::Builder::new()
        .name("polaris-probe-stdout".to_owned())
        .spawn(move || read_capped(stdout, cap))
        .map_err(|error| PlatformError::Io(error.to_string()))?;
    let stderr_reader = match std::thread::Builder::new()
        .name("polaris-probe-stderr".to_owned())
        .spawn(move || read_capped(stderr, cap))
    {
        Ok(reader) => reader,
        Err(error) => {
            let _kill_result = owner.child.start_kill();
            let _wait_result = owner.child.wait();
            owner.finished = true;
            let _stdout_result = stdout_reader.join();
            return Err(PlatformError::Io(error.to_string()));
        }
    };

    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match owner.child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                let _kill_result = owner.child.start_kill();
                let _wait_result = owner.child.wait();
                owner.finished = true;
                let _stdout_result = stdout_reader.join();
                let _stderr_result = stderr_reader.join();
                return Err(PlatformError::Io(error.to_string()));
            }
        }
        if started.elapsed() >= deadline {
            timed_out = true;
            owner
                .child
                .start_kill()
                .map_err(|error| PlatformError::Io(error.to_string()))?;
            break owner
                .child
                .wait()
                .map_err(|error| PlatformError::Io(error.to_string()))?;
        }
        std::thread::sleep(
            Duration::from_millis(5).min(deadline.saturating_sub(started.elapsed())),
        );
    };
    owner.finished = true;
    let stdout = join_reader(stdout_reader)?;
    let stderr = join_reader(stderr_reader)?;
    Ok(ExecOutput {
        exit_code: status.code(),
        stdout: stdout.bytes,
        stderr: stderr.bytes,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
        timed_out,
    })
}

fn safe_probe_environment() -> Result<EnvPolicy, PlatformError> {
    let mut entries: Vec<(OsString, OsString)> = SAFE_INHERITED_ENV
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
        .collect();
    entries.extend([
        (OsString::from("RUSTUP_AUTO_INSTALL"), OsString::from("0")),
        (OsString::from("CARGO_NET_OFFLINE"), OsString::from("true")),
    ]);
    EnvPolicy::new(entries)
}

fn read_capped<R: Read>(mut reader: R, cap: usize) -> std::io::Result<CapturedStream> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let available = cap.saturating_sub(bytes.len());
        let retained = count.min(available);
        if let Some(chunk) = buffer.get(..retained) {
            bytes.extend_from_slice(chunk);
        }
        truncated |= retained < count;
    }
    Ok(CapturedStream { bytes, truncated })
}

fn join_reader(
    reader: std::thread::JoinHandle<std::io::Result<CapturedStream>>,
) -> Result<CapturedStream, PlatformError> {
    match reader.join() {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(PlatformError::Io(error.to_string())),
        Err(_) => Err(PlatformError::Io(
            "probe output reader thread failed".to_owned(),
        )),
    }
}
