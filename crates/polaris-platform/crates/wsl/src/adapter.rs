//! WSL host adapter. All commands are launched as an argv vector without a shell.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use uuid::Uuid;

const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const DEFAULT_DISTRO: &str = "Ubuntu-24.04";
const RUNNER_SCRIPT: &str = "token=$1; shift; printf 'POLARIS_PGID=%s\\n' \"$$\"; RUSTUP_AUTO_INSTALL=0 CARGO_NET_OFFLINE=true \"$@\"; status=$?; printf '\\nPOLARIS_EXIT_%s=%s\\n' \"$token\" \"$status\" >&2; while :; do sleep 3600 & wait $!; done";
const SAFE_HOST_ENV: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "WINDIR",
    "TEMP",
    "TMP",
    "LOCALAPPDATA",
    "USERPROFILE",
];

/// Validated WSL distribution name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DistroName(String);

impl DistroName {
    /// Validates a distribution name accepted by `wsl.exe`.
    pub fn new(value: impl Into<String>) -> Result<Self, WslError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(WslError::InvalidDistroName);
        }
        Ok(Self(value))
    }

    /// Returns the validated name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Validated absolute POSIX path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LinuxPath(String);

impl LinuxPath {
    /// Validates and stores an absolute Linux path without NUL or parent traversal.
    pub fn new(value: impl Into<String>) -> Result<Self, WslError> {
        let value = value.into();
        if !value.starts_with('/')
            || value.contains('\0')
            || value.contains('\\')
            || value.split('/').any(|component| component == "..")
        {
            return Err(WslError::InvalidLinuxPath);
        }
        Ok(Self(value))
    }

    /// Returns the absolute path text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Windows host path used by WSL path conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinPath(PathBuf);

impl WinPath {
    /// Creates a Windows path wrapper.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }

    /// Returns the wrapped path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// WSL adapter bound to an absolute `wsl.exe` path.
#[derive(Debug, Clone)]
pub struct Wsl {
    executable: PathBuf,
    command_timeout: Duration,
}

/// Typed WSL command failure.
#[derive(Debug, Error)]
pub enum WslError {
    /// WSL could not be found on PATH.
    #[error("wsl.exe was not found")]
    NotInstalled,
    /// Distribution name failed validation.
    #[error("invalid WSL distribution name")]
    InvalidDistroName,
    /// Requested distribution was not present in `wsl.exe -l -v` output.
    #[error("WSL distribution is not installed: {0}")]
    DistroMissing(String),
    /// Windows path could not be translated safely.
    #[error("Windows path cannot be translated safely")]
    InvalidWindowsPath,
    /// Linux path failed absolute-path validation.
    #[error("Linux path must be absolute and contain no parent traversal")]
    InvalidLinuxPath,
    /// Command output encoding was invalid.
    #[error("WSL output could not be decoded")]
    Decode,
    /// Command exited unsuccessfully.
    #[error("WSL command exited with code {code}: {stderr}")]
    Exit { code: i32, stderr: String },
    /// Command exceeded its configured timeout.
    #[error("WSL command timed out")]
    Timeout,
    /// Captured stdout/stderr exceeded the bounded output limit.
    #[error("WSL command output exceeded the configured cap")]
    OutputTooLarge,
    /// KVM device is absent or unreadable in the requested distro.
    #[error("/dev/kvm is unavailable in WSL distribution")]
    KvmUnavailable,
    /// Configuration has a duplicate or malformed setting.
    #[error("invalid .wslconfig value for {key}: {detail}")]
    ConfigInvalid { key: String, detail: String },
    /// Underlying operating-system operation failed.
    #[error("WSL host operation failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Captured command output and exit status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecOutput {
    /// Exit code reported by `wsl.exe`.
    pub exit_code: i32,
    /// Raw stdout bytes.
    pub stdout: Vec<u8>,
    /// Raw stderr bytes.
    pub stderr: Vec<u8>,
}

/// Parsed distribution row from `wsl.exe -l -v`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DistroInfo {
    /// Validated distribution name.
    pub name: DistroName,
    /// State emitted by WSL, for example `Running` or `Stopped`.
    pub state: String,
    /// WSL generation when present.
    pub version: Option<u8>,
    /// Whether WSL marked this distribution as the default.
    pub is_default: bool,
}

/// Whether KVM can be used inside a distribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KvmStatus {
    /// `/dev/kvm` exists and is readable by the configured WSL user.
    Available,
    /// `/dev/kvm` is absent or unreadable.
    Unavailable,
    /// No Ubuntu distribution was available to probe.
    NotProbed,
}

/// Serializable health report consumed by the CLI doctor command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WslReport {
    /// Whether `wsl.exe` could be invoked.
    pub available: bool,
    /// First version line, when WSL supplied one.
    pub version: Option<String>,
    /// Installed WSL distributions.
    pub distributions: Vec<DistroInfo>,
    /// `Ubuntu-24.04` availability.
    pub ubuntu_24_04_installed: bool,
    /// KVM probe result.
    pub kvm: KvmStatus,
    /// Human-readable diagnostic details without credentials.
    pub diagnostics: Vec<String>,
}

/// Configuration validation results with warnings suitable for a doctor report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WslConfigReport {
    /// Parsed memory cap in MiB, if configured.
    pub memory_mib: Option<u64>,
    /// Whether nested virtualization was explicitly enabled.
    pub nested_virtualization: Option<bool>,
    /// Non-fatal observations.
    pub warnings: Vec<String>,
}

/// Plan for installing the recommended Ubuntu distro; producing a plan has no host side effects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvisionPlan {
    /// Distribution name to install.
    pub distro: DistroName,
    /// Exact argv that a separately confirmed onboarding action may execute.
    pub argv: Vec<String>,
    /// Administrator elevation and possible reboot may be required by Windows.
    pub may_require_elevation: bool,
    /// This adapter does not execute the plan automatically.
    pub requires_explicit_confirmation: bool,
}

/// A running keepalive process group, killed automatically when dropped.
#[derive(Debug)]
pub struct KeepAlive {
    child: Option<Box<dyn ChildWrapper>>,
    executable: PathBuf,
    distro: DistroName,
    process_group_id: u32,
    cleanup_completed: bool,
}

/// Cancellation guard for an acknowledged Linux process group. The shell supervisor remains
/// alive in this group until the host sends the cleanup signal, preventing PGID reuse.
struct LinuxGroupOwner {
    executable: PathBuf,
    distro: DistroName,
    process_group_id: u32,
    active: bool,
}

impl LinuxGroupOwner {
    fn new(executable: &Path, distro: &DistroName, process_group_id: u32) -> Self {
        Self {
            executable: executable.to_path_buf(),
            distro: distro.clone(),
            process_group_id,
            active: true,
        }
    }

    async fn terminate(&mut self) -> Result<(), WslError> {
        terminate_linux_group(&self.executable, &self.distro, self.process_group_id).await?;
        self.active = false;
        Ok(())
    }
}

impl Drop for LinuxGroupOwner {
    fn drop(&mut self) {
        if self.active {
            terminate_linux_group_sync(&self.executable, &self.distro, self.process_group_id);
        }
    }
}

impl KeepAlive {
    /// Returns whether the keepalive process is still running.
    pub async fn is_running(&mut self) -> Result<bool, WslError> {
        match self.child.as_mut() {
            Some(child) => Ok(child.try_wait()?.is_none()),
            None => Ok(false),
        }
    }

    /// Returns the Linux process-group identifier retained for cleanup.
    #[must_use]
    pub fn process_group_id(&self) -> u32 {
        self.process_group_id
    }

    /// Stops the Linux process group and its host-side WSL client explicitly.
    pub async fn shutdown(mut self) -> Result<(), WslError> {
        terminate_linux_group(&self.executable, &self.distro, self.process_group_id).await?;
        drop(self.child.take());
        self.cleanup_completed = true;
        Ok(())
    }
}

impl Drop for KeepAlive {
    fn drop(&mut self) {
        if self.cleanup_completed {
            return;
        }
        terminate_linux_group_sync(&self.executable, &self.distro, self.process_group_id);
        self.child.take();
    }
}

impl Wsl {
    /// Locates `wsl.exe` through PATH and stores a bounded command timeout.
    pub fn locate() -> Result<Self, WslError> {
        let executable = which::which("wsl.exe")
            .or_else(|_| which::which("wsl"))
            .map_err(|_| WslError::NotInstalled)?;
        Ok(Self {
            executable,
            command_timeout: Duration::from_secs(30),
        })
    }

    /// Creates an adapter from a caller-supplied absolute executable path.
    pub fn from_executable(
        executable: PathBuf,
        command_timeout: Duration,
    ) -> Result<Self, WslError> {
        if !executable.is_absolute() || !executable.is_file() || command_timeout.is_zero() {
            return Err(WslError::NotInstalled);
        }
        Ok(Self {
            executable,
            command_timeout,
        })
    }

    /// Returns the resolved executable path.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Reads installed distributions from WSL's native list command.
    pub async fn list(&self) -> Result<Vec<DistroInfo>, WslError> {
        let output = self
            .run(&["--list", "--verbose"], self.command_timeout)
            .await?;
        ensure_success(output.clone())?;
        let text = decode_wsl_output(&output.stdout)?;
        Ok(parse_distro_list(&text))
    }

    /// Produces a serializable, secret-free health report for WSL and the optional Ubuntu distro.
    pub async fn doctor(&self) -> Result<WslReport, WslError> {
        let version_result = self.run(&["--version"], self.command_timeout).await;
        let (version, mut diagnostics) = match version_result {
            Ok(output) => {
                ensure_success(output.clone())?;
                let decoded = decode_wsl_output(&output.stdout)?;
                (decoded.lines().next().map(str::to_owned), Vec::new())
            }
            Err(error) => (None, vec![error.to_string()]),
        };
        let distributions = self.list().await?;
        let ubuntu = distributions
            .iter()
            .find(|item| item.name.as_str() == DEFAULT_DISTRO);
        let ubuntu_24_04_installed = ubuntu.is_some();
        let kvm = match ubuntu {
            Some(_) => match self
                .exec(
                    &DistroName::new(DEFAULT_DISTRO)?,
                    &["/usr/bin/test", "-r", "/dev/kvm"],
                    None,
                    self.command_timeout,
                )
                .await
            {
                Ok(output) if output.exit_code == 0 => KvmStatus::Available,
                Ok(_) | Err(WslError::Exit { .. }) | Err(WslError::KvmUnavailable) => {
                    KvmStatus::Unavailable
                }
                Err(error) => {
                    diagnostics.push(format!("KVM probe: {error}"));
                    KvmStatus::NotProbed
                }
            },
            None => KvmStatus::NotProbed,
        };
        if ubuntu.is_none() {
            diagnostics.push(format!("{DEFAULT_DISTRO} is not installed"));
        }
        Ok(WslReport {
            available: true,
            version,
            distributions,
            ubuntu_24_04_installed,
            kvm,
            diagnostics,
        })
    }

    /// Runs argv directly in a distribution with a timeout and a 4 MiB output cap per stream.
    pub async fn exec(
        &self,
        distro: &DistroName,
        argv: &[&str],
        cwd: Option<&LinuxPath>,
        timeout: Duration,
    ) -> Result<ExecOutput, WslError> {
        if argv.is_empty() || argv.iter().any(|item| item.contains('\0')) {
            return Err(WslError::InvalidLinuxPath);
        }
        self.run_distro(distro, argv, cwd, timeout).await
    }

    /// Checks `/dev/kvm` readability with a real argv command in the selected distribution.
    pub async fn kvm(&self, distro: &DistroName) -> Result<KvmStatus, WslError> {
        let output = self
            .run_distro(
                distro,
                &["/usr/bin/test", "-r", "/dev/kvm"],
                None,
                self.command_timeout,
            )
            .await?;
        Ok(if output.exit_code == 0 {
            KvmStatus::Available
        } else {
            KvmStatus::Unavailable
        })
    }

    /// Starts a retained Linux process group to keep WSL alive; dropping the handle kills the
    /// group and its host-side WSL client.
    pub async fn keepalive(&self, distro: &DistroName) -> Result<KeepAlive, WslError> {
        let args = runner_args(distro, None, "keepalive", &["/bin/sleep", "infinity"]);
        let mut command = host_command(&self.executable, &args);
        let raw = command.command_mut();
        raw.stdin(Stdio::null());
        raw.stdout(Stdio::piped());
        raw.stderr(Stdio::null());
        wrap_host_tree(&mut command);
        let mut child = command.spawn().map_err(WslError::Io)?;
        let stdout = child
            .stdout()
            .take()
            .ok_or_else(|| std::io::Error::other("keepalive stdout pipe unavailable"))?;
        let mut stdout = BufReader::new(stdout);
        let mut line = String::new();
        let read_result =
            tokio::time::timeout(self.command_timeout, stdout.read_line(&mut line)).await;
        let process_group_id = match read_result {
            Ok(Ok(_)) => parse_group_handshake(&line),
            Ok(Err(error)) => {
                let _kill_result = child.start_kill();
                let _wait_result = child.wait().await;
                return Err(WslError::Io(error));
            }
            Err(_) => {
                let _kill_result = child.start_kill();
                let _wait_result = child.wait().await;
                return Err(WslError::Timeout);
            }
        };
        let Some(process_group_id) = process_group_id else {
            let _kill_result = child.start_kill();
            let _wait_result = child.wait().await;
            return Err(WslError::Decode);
        };
        drop(stdout);
        Ok(KeepAlive {
            child: Some(child),
            executable: self.executable.clone(),
            distro: distro.clone(),
            process_group_id,
            cleanup_completed: false,
        })
    }

    /// Returns a no-side-effect plan for installing Ubuntu 24.04.
    #[must_use]
    pub fn provision_ubuntu_plan() -> ProvisionPlan {
        ProvisionPlan {
            distro: DistroName(DEFAULT_DISTRO.to_owned()),
            argv: vec![
                "--install".to_owned(),
                "--distribution".to_owned(),
                DEFAULT_DISTRO.to_owned(),
            ],
            may_require_elevation: true,
            requires_explicit_confirmation: true,
        }
    }

    /// Probes Windows-to-WSL localhost TCP forwarding by connecting to the requested address.
    pub async fn probe_gateway_reachability(
        &self,
        host: &str,
        port: u16,
        timeout: Duration,
    ) -> Result<bool, WslError> {
        let address = format!("{host}:{port}");
        match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(address)).await {
            Ok(Ok(_stream)) => Ok(true),
            Ok(Err(error))
                if error.kind() == std::io::ErrorKind::ConnectionRefused
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                Ok(false)
            }
            Ok(Err(error)) => Err(WslError::Io(error)),
            Err(_) => Ok(false),
        }
    }

    async fn run(
        &self,
        args: &[impl AsRef<std::ffi::OsStr>],
        timeout: Duration,
    ) -> Result<ExecOutput, WslError> {
        let args = args
            .iter()
            .map(|item| item.as_ref().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let mut command = host_command(&self.executable, &args);
        command
            .command_mut()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        wrap_host_tree(&mut command);
        let mut child = command.spawn().map_err(WslError::Io)?;
        let stdout = child
            .stdout()
            .take()
            .ok_or_else(|| std::io::Error::other("stdout pipe unavailable"))?;
        let stderr = child
            .stderr()
            .take()
            .ok_or_else(|| std::io::Error::other("stderr pipe unavailable"))?;
        let stdout_read = read_bounded(stdout, OUTPUT_LIMIT);
        let stderr_read = read_bounded(stderr, OUTPUT_LIMIT);
        let command_run = async {
            let (stdout_result, stderr_result, status_result) =
                tokio::join!(stdout_read, stderr_read, Box::pin(child.wait()));
            let stdout = stdout_result?;
            let stderr = stderr_result?;
            let status = status_result?;
            Ok::<ExecOutput, WslError>(ExecOutput {
                exit_code: status_code(status),
                stdout,
                stderr,
            })
        };
        match tokio::time::timeout(timeout, command_run).await {
            Ok(result) => result,
            Err(_) => {
                let _kill_result = child.start_kill();
                let _wait_result = child.wait().await;
                Err(WslError::Timeout)
            }
        }
    }

    async fn run_distro(
        &self,
        distro: &DistroName,
        argv: &[&str],
        cwd: Option<&LinuxPath>,
        timeout: Duration,
    ) -> Result<ExecOutput, WslError> {
        let marker = Uuid::now_v7().simple().to_string();
        let args = runner_args(distro, cwd, &marker, argv);
        let mut command = host_command(&self.executable, &args);
        command
            .command_mut()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        wrap_host_tree(&mut command);
        let mut child = command.spawn().map_err(WslError::Io)?;
        let stdout = child
            .stdout()
            .take()
            .ok_or_else(|| std::io::Error::other("stdout unavailable"))?;
        let stderr = child
            .stderr()
            .take()
            .ok_or_else(|| std::io::Error::other("stderr unavailable"))?;
        let mut stdout = BufReader::new(stdout);
        let mut handshake = String::new();
        tokio::time::timeout(timeout, stdout.read_line(&mut handshake))
            .await
            .map_err(|_| WslError::Timeout)??;
        let process_group_id = parse_group_handshake(&handshake).ok_or(WslError::Decode)?;
        let mut group_owner = LinuxGroupOwner::new(&self.executable, distro, process_group_id);
        let mut stdout_read = Box::pin(read_bounded(stdout, OUTPUT_LIMIT));
        let mut stderr_read = Box::pin(read_exit_marker(stderr, OUTPUT_LIMIT, marker));
        let marker_result = tokio::time::timeout(timeout, async {
            tokio::select! {
                marker = &mut stderr_read => marker,
                stdout_result = &mut stdout_read => {
                    stdout_result?;
                    Err(WslError::Decode)
                }
            }
        })
        .await;
        let (stderr, exit_code) = match marker_result {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => {
                let group_result = group_owner.terminate().await;
                let _host_kill = child.start_kill();
                let _host_wait = child.wait().await;
                group_result?;
                return Err(error);
            }
            Err(_) => {
                let group_result = group_owner.terminate().await;
                let _host_kill = child.start_kill();
                let _host_wait = child.wait().await;
                group_result?;
                return Err(WslError::Timeout);
            }
        };
        group_owner.terminate().await?;
        let status = child.wait().await?;
        let stdout = stdout_read.await?;
        if status.code().is_none() {
            return Err(WslError::Decode);
        }
        Ok(ExecOutput {
            exit_code,
            stdout,
            stderr,
        })
    }
}

fn status_code(status: std::process::ExitStatus) -> i32 {
    let mut code = status.code();
    *code.get_or_insert(-1)
}

async fn read_bounded<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    cap: usize,
) -> Result<Vec<u8>, WslError> {
    let mut reader = reader;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut total = 0_usize;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        total = total.checked_add(read).ok_or(WslError::OutputTooLarge)?;
        if total <= cap {
            bytes.extend_from_slice(buffer.get(..read).ok_or(WslError::OutputTooLarge)?);
        }
    }
    if total > cap {
        return Err(WslError::OutputTooLarge);
    }
    Ok(bytes)
}

async fn read_exit_marker<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    cap: usize,
    token: String,
) -> Result<(Vec<u8>, i32), WslError> {
    let mut reader = reader;
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    let marker = format!("\nPOLARIS_EXIT_{token}=");
    let marker = marker.as_bytes();
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return Err(WslError::Decode);
        }
        output.extend_from_slice(&buffer[..count]);
        if let Some(index) = output.windows(marker.len()).position(|part| part == marker) {
            let suffix_start = index + marker.len();
            if let Some(relative_end) = output
                .get(suffix_start..)
                .and_then(|tail| tail.iter().position(|byte| *byte == b'\n'))
            {
                let end = suffix_start + relative_end;
                let code = std::str::from_utf8(&output[suffix_start..end])
                    .map_err(|_| WslError::Decode)?;
                let code = code.parse::<i32>().map_err(|_| WslError::Decode)?;
                if index > cap {
                    return Err(WslError::OutputTooLarge);
                }
                output.truncate(index);
                return Ok((output, code));
            }
        }
        if output.len() > cap.saturating_add(64) {
            return Err(WslError::OutputTooLarge);
        }
    }
}

fn runner_args(
    distro: &DistroName,
    cwd: Option<&LinuxPath>,
    token: &str,
    argv: &[&str],
) -> Vec<String> {
    let mut args = vec!["--distribution".to_owned(), distro.as_str().to_owned()];
    if let Some(cwd) = cwd {
        args.extend(["--cd".to_owned(), cwd.as_str().to_owned()]);
    }
    args.extend([
        "--exec".to_owned(),
        "/usr/bin/setsid".to_owned(),
        "--wait".to_owned(),
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        RUNNER_SCRIPT.to_owned(),
        "polaris-runner".to_owned(),
    ]);
    args.push(token.to_owned());
    args.extend(argv.iter().map(|argument| (*argument).to_owned()));
    args
}

fn parse_group_handshake(line: &str) -> Option<u32> {
    let value = line
        .trim()
        .strip_prefix("POLARIS_PGID=")?
        .parse::<u32>()
        .ok()?;
    (value > 1).then_some(value)
}

fn host_command(executable: &Path, args: &[String]) -> CommandWrap {
    let mut command = CommandWrap::with_new(executable.as_os_str(), |_| {});
    let raw = command.command_mut();
    raw.args(args);
    raw.env_clear();
    for key in SAFE_HOST_ENV {
        if let Some(value) = std::env::var_os(key) {
            raw.env(key, value);
        }
    }
    raw.stdin(Stdio::null());
    command
}

fn wrap_host_tree(command: &mut CommandWrap) {
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(JobObject);
    command.wrap(KillOnDrop);
}

async fn terminate_linux_group(
    executable: &Path,
    distro: &DistroName,
    process_group_id: u32,
) -> Result<(), WslError> {
    if process_group_id <= 1 {
        return Err(WslError::Decode);
    }
    let args = vec![
        "--distribution".to_owned(),
        distro.as_str().to_owned(),
        "--exec".to_owned(),
        "/bin/kill".to_owned(),
        "-KILL".to_owned(),
        "--".to_owned(),
        format!("-{process_group_id}"),
    ];
    let mut command = host_command(executable, &args);
    wrap_host_tree(&mut command);
    let mut child = command.spawn().map_err(WslError::Io)?;
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    match status {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(_)) => Err(WslError::Decode),
        Ok(Err(error)) => Err(WslError::Io(error)),
        Err(_) => {
            let _kill = child.start_kill();
            let _wait = child.wait().await;
            Err(WslError::Timeout)
        }
    }
}

fn terminate_linux_group_sync(executable: &Path, distro: &DistroName, process_group_id: u32) {
    if process_group_id <= 1 {
        return;
    }
    let mut command = std::process::Command::new(executable);
    command.args([
        "--distribution",
        distro.as_str(),
        "--exec",
        "/bin/kill",
        "-KILL",
        "--",
    ]);
    command.arg(format!("-{process_group_id}"));
    command.env_clear();
    for key in SAFE_HOST_ENV {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Ok(mut child) = command.spawn() {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if child.try_wait().ok().flatten().is_some() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                let _kill = child.kill();
                let _wait = child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

/// Decodes `wsl.exe` output, handling UTF-16LE BOM output and UTF-8 explicitly.
pub fn decode_wsl_output(bytes: &[u8]) -> Result<String, WslError> {
    if bytes.starts_with(&[0xff, 0xfe]) {
        let body = bytes.get(2..).ok_or(WslError::Decode)?;
        return decode_utf16le(body);
    }
    if bytes.starts_with(&[0xfe, 0xff]) {
        return Err(WslError::Decode);
    }
    if bytes.len().is_multiple_of(2) && bytes.iter().skip(1).step_by(2).any(|byte| *byte == 0) {
        return decode_utf16le(bytes);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| WslError::Decode)?;
    Ok(text
        .chars()
        .filter(|character| *character != '\0')
        .collect())
}

fn decode_utf16le(bytes: &[u8]) -> Result<String, WslError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(WslError::Decode);
    }
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let low = pair.first().copied().ok_or(WslError::Decode)?;
            let high = pair.get(1).copied().ok_or(WslError::Decode)?;
            Ok(u16::from_le_bytes([low, high]))
        })
        .collect::<Result<Vec<_>, WslError>>()?;
    let decoded = char::decode_utf16(units)
        .collect::<Result<String, _>>()
        .map_err(|_| WslError::Decode)?;
    Ok(decoded
        .chars()
        .filter(|character| *character != '\0')
        .collect())
}

/// Parses WSL's table output; unrecognized rows are ignored rather than fabricated.
pub fn parse_distro_list(text: &str) -> Vec<DistroInfo> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let is_default = trimmed.starts_with('*');
            let row = match trimmed.strip_prefix('*') {
                Some(without_marker) => without_marker.trim(),
                None => trimmed,
            };
            let mut columns = row.split_whitespace();
            let name = columns.next()?;
            if name.eq_ignore_ascii_case("NAME") {
                return None;
            }
            let state = columns.next()?.to_owned();
            let version = columns.next().and_then(|value| value.parse::<u8>().ok());
            let name = DistroName::new(name.to_owned()).ok()?;
            Some(DistroInfo {
                name,
                state,
                version,
                is_default,
            })
        })
        .collect()
}

/// Converts a drive-qualified Windows path to the corresponding `/mnt/<drive>` path.
pub fn to_linux_path(path: &WinPath) -> Result<LinuxPath, WslError> {
    let text = path.0.to_string_lossy();
    if text.starts_with("\\\\?\\") || text.starts_with("\\.\\") || text.contains('\0') {
        return Err(WslError::InvalidWindowsPath);
    }
    let normalized = text.replace('/', "\\");
    let prefix = "\\\\wsl.localhost\\";
    if let Some(unc_tail) = normalized.strip_prefix(prefix) {
        let mut components = unc_tail.split('\\');
        let distro = components.next().ok_or(WslError::InvalidWindowsPath)?;
        DistroName::new(distro.to_owned())?;
        let suffix = components.collect::<Vec<_>>().join("/");
        if suffix
            .split('/')
            .any(|part| part == ".." || part == "." || part.ends_with(' ') || part.ends_with('.'))
        {
            return Err(WslError::InvalidWindowsPath);
        }
        return LinuxPath::new(format!("/{suffix}"));
    }
    let chars = normalized.chars().collect::<Vec<_>>();
    let (drive, tail) = match (chars.first(), chars.get(1), chars.get(2)) {
        (Some(drive), Some(':'), Some('\\')) if drive.is_ascii_alphabetic() => {
            (*drive, chars.get(3..).ok_or(WslError::InvalidWindowsPath)?)
        }
        _ => return Err(WslError::InvalidWindowsPath),
    };
    let suffix = tail.iter().collect::<String>();
    let mut linux = format!(
        "/mnt/{}/{}",
        drive.to_ascii_lowercase(),
        suffix.replace('\\', "/")
    );
    while linux.ends_with('/') && linux.len() > "/mnt/c".len() {
        linux.pop();
    }
    if linux
        .split('/')
        .any(|part| part == ".." || part.ends_with(' ') || part.ends_with('.'))
        || linux.contains(":")
    {
        return Err(WslError::InvalidWindowsPath);
    }
    LinuxPath::new(linux)
}

/// Converts an absolute Linux path to `\\wsl.localhost\<distro>\...`.
pub fn to_windows_path(path: &LinuxPath, distro: &DistroName) -> Result<WinPath, WslError> {
    let linux = LinuxPath::new(path.0.clone())?;
    let suffix = linux.0.trim_start_matches('/').replace('/', "\\");
    let unc = if suffix.is_empty() {
        format!("\\\\wsl.localhost\\{}", distro.0)
    } else {
        format!("\\\\wsl.localhost\\{}\\{}", distro.0, suffix)
    };
    Ok(WinPath(PathBuf::from(unc)))
}

/// Validates a `.wslconfig` file without modifying it.
pub fn validate_wslconfig(text: &str) -> Result<WslConfigReport, WslError> {
    let mut section = String::new();
    let mut sections = HashSet::new();
    let mut keys = HashSet::new();
    let mut report = WslConfigReport {
        memory_mib: None,
        nested_virtualization: None,
        warnings: Vec::new(),
    };
    for raw_line in text.lines() {
        let line = match raw_line.split(['#', ';']).next() {
            Some(value) => value.trim(),
            None => "",
        };
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line
                .get(1..line.len().saturating_sub(1))
                .ok_or_else(|| config_error("section", "malformed header".to_owned()))?
                .trim()
                .to_ascii_lowercase();
            if section != "wsl2" {
                return Err(config_error(
                    "section",
                    format!("unsupported section {section}"),
                ));
            }
            if !sections.insert(section.clone()) {
                return Err(config_error(
                    "section",
                    "duplicate [wsl2] section".to_owned(),
                ));
            }
            continue;
        }
        if section != "wsl2" {
            return Err(config_error(
                "section",
                "setting appears before [wsl2]".to_owned(),
            ));
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| config_error("line", "expected key=value".to_owned()))?;
        let key = key.trim();
        let value = value.trim();
        let normalized = key.to_ascii_lowercase();
        if !keys.insert(normalized.clone()) {
            return Err(config_error(key, "duplicate setting".to_owned()));
        }
        match normalized.as_str() {
            "memory" => report.memory_mib = Some(parse_memory_mib(key, value)?),
            "nestedvirtualization" => report.nested_virtualization = Some(parse_bool(key, value)?),
            "processors"
            | "swap"
            | "swapfile"
            | "localhostforwarding"
            | "networkingmode"
            | "vmidletimeout"
            | "kernel"
            | "kernelcommandline"
            | "guiapplications"
            | "debugconsole"
            | "safemode" => report.warnings.push(format!(
                "recognized setting {key} is validated for syntax only"
            )),
            _ => return Err(config_error(key, "unknown setting".to_owned())),
        }
    }
    if !text.is_empty() && report.nested_virtualization != Some(true) {
        report.warnings.push(
            "nestedVirtualization=true is recommended for the OpenShell vm driver".to_owned(),
        );
    }
    if report.memory_mib.is_none() {
        report
            .warnings
            .push("memory is not capped in .wslconfig".to_owned());
    }
    if sections.is_empty() {
        report
            .warnings
            .push(".wslconfig has no [wsl2] section".to_owned());
    }
    Ok(report)
}

fn parse_memory_mib(key: &str, value: &str) -> Result<u64, WslError> {
    let upper = value.to_ascii_uppercase();
    let (number, multiplier) = if let Some(number) = upper.strip_suffix("GB") {
        (number.trim(), 1024_u64)
    } else if let Some(number) = upper.strip_suffix("MB") {
        (number.trim(), 1_u64)
    } else {
        return Err(config_error(
            key,
            "memory requires MB or GB units".to_owned(),
        ));
    };
    let number = number
        .parse::<u64>()
        .map_err(|_| config_error(key, "invalid memory amount".to_owned()))?;
    number
        .checked_mul(multiplier)
        .filter(|amount| *amount > 0)
        .ok_or_else(|| config_error(key, "memory amount is out of range".to_owned()))
}

fn parse_bool(key: &str, value: &str) -> Result<bool, WslError> {
    match value.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(config_error(key, "expected true or false".to_owned())),
    }
}

fn config_error(key: &str, detail: String) -> WslError {
    WslError::ConfigInvalid {
        key: key.to_owned(),
        detail,
    }
}

fn ensure_success(output: ExecOutput) -> Result<ExecOutput, WslError> {
    if output.exit_code == 0 {
        Ok(output)
    } else {
        let stderr = match decode_wsl_output(&output.stderr) {
            Ok(decoded) => decoded,
            Err(_) => String::from_utf8_lossy(&output.stderr).into_owned(),
        };
        Err(WslError::Exit {
            code: output.exit_code,
            stderr,
        })
    }
}
