//! Explicit-argv shell execution through a caller-owned sandbox handle only.

use core::future::Future;
use std::sync::Arc;
use std::time::Duration;

use polaris_sdk::registry::{ExecRequest, SandboxBackend, SandboxHandle, ToolError};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ToolLimits;

/// Supported command grammars for the advisory classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellFlavor {
    /// POSIX-like shell commands.
    Posix,
    /// PowerShell commands.
    PowerShell,
    /// Windows command prompt commands.
    Cmd,
}

/// Advisory classification returned with shell results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandClass {
    /// Build, compile, or test command that may need a longer budget.
    Heavy,
    /// Command with a recognized destructive operation.
    Destructive,
    /// Command did not match a known classification.
    Unknown,
}

/// Determine whether argv contains a known heavy or destructive command.
#[must_use]
pub fn classify_command(argv: &[String], shell: ShellFlavor) -> CommandClass {
    let normalized: Vec<String> = argv
        .iter()
        .map(|argument| argument.trim_matches(['\'', '"']).to_ascii_lowercase())
        .collect();
    let Some(program) = normalized.first().map(String::as_str) else {
        return CommandClass::Unknown;
    };
    let destructive = match (shell, program) {
        (ShellFlavor::Posix, "rm") => normalized
            .iter()
            .skip(1)
            .any(|arg| arg == "-rf" || arg == "-fr" || arg == "-r" || arg == "--recursive"),
        (ShellFlavor::PowerShell, "remove-item") => normalized
            .iter()
            .skip(1)
            .any(|arg| matches!(arg.as_str(), "-recurse" | "-r" | "-force" | "-f")),
        (ShellFlavor::Cmd, "del" | "erase" | "rmdir" | "rd") => normalized
            .iter()
            .skip(1)
            .any(|arg| matches!(arg.as_str(), "/s" | "/q" | "/f")),
        _ => false,
    };
    if destructive {
        return CommandClass::Destructive;
    }
    let heavy_program = matches!(
        program,
        "cargo" | "rustc" | "rustup" | "cmake" | "make" | "ninja" | "msbuild" | "cl" | "nvcc"
    );
    let heavy_subcommand = normalized.get(1).is_some_and(|argument| {
        matches!(
            argument.as_str(),
            "build" | "check" | "test" | "bench" | "compile" | "install"
        )
    });
    if heavy_program && (program != "cargo" || heavy_subcommand) {
        CommandClass::Heavy
    } else {
        CommandClass::Unknown
    }
}

/// Typed model arguments for sandboxed command execution.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellInput {
    /// Program and arguments. No implicit host shell is ever selected.
    pub argv: Vec<String>,
    /// Workspace-relative working directory inside the sandbox.
    pub workdir: Option<String>,
    /// Command grammar used only by the advisory classifier.
    pub shell: ShellFlavor,
    /// Requested timeout in milliseconds, capped by the registry deadline.
    pub timeout_ms: Option<u64>,
}

/// Shell output, retained in full within the configured bounded capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShellOutput {
    /// Exit code, or -1 when terminated by the backend.
    pub exit_code: i32,
    /// Captured UTF-8 stdout.
    pub stdout: String,
    /// Captured UTF-8 stderr.
    pub stderr: String,
    /// Advisory classification of argv.
    pub classification: CommandClass,
    /// Elapsed milliseconds.
    pub duration_ms: u64,
}

/// Runtime-owned shell route. Implementations must not create or destroy the sandbox implicitly.
pub trait ShellToolBackend: Send + Sync {
    /// Execute one explicit argv request in the configured sandbox.
    fn execute(
        &self,
        input: &ShellInput,
        limits: ToolLimits,
        deadline: Option<Duration>,
    ) -> impl Future<Output = Result<ShellOutput, ToolError>> + Send;
}

/// P1 registry marker whose P2 shell route fails closed.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoShellBackend;

impl ShellToolBackend for NoShellBackend {
    async fn execute(
        &self,
        _input: &ShellInput,
        _limits: ToolLimits,
        _deadline: Option<Duration>,
    ) -> Result<ShellOutput, ToolError> {
        Err(ToolError::Forbidden(
            "sandbox shell service is not configured".to_owned(),
        ))
    }
}

/// SDK sandbox adapter bound to a runtime-owned handle. It never starts host processes.
pub struct SandboxShellBackend<B> {
    backend: Arc<B>,
    sandbox: SandboxHandle,
}

impl<B> std::fmt::Debug for SandboxShellBackend<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxShellBackend")
            .field("sandbox", &self.sandbox)
            .finish_non_exhaustive()
    }
}

impl<B: SandboxBackend> SandboxShellBackend<B> {
    /// Bind the backend to a sandbox handle owned by the runtime.
    #[must_use]
    pub fn new(backend: Arc<B>, sandbox: SandboxHandle) -> Self {
        Self { backend, sandbox }
    }
}

impl<B: SandboxBackend> ShellToolBackend for SandboxShellBackend<B> {
    async fn execute(
        &self,
        input: &ShellInput,
        limits: ToolLimits,
        deadline: Option<Duration>,
    ) -> Result<ShellOutput, ToolError> {
        validate_shell_input(input)?;
        let mut timeout = input.timeout_ms.map(Duration::from_millis);
        timeout = match (timeout, deadline) {
            (Some(requested), Some(remaining)) => Some(requested.min(remaining)),
            (Some(requested), None) => Some(requested),
            (None, Some(remaining)) => Some(remaining),
            (None, None) => Some(limits.timeout),
        };
        if timeout.is_some_and(|value| value.is_zero()) {
            return Err(ToolError::Timeout);
        }
        let max_output = usize::try_from(limits.max_output_bytes)
            .map_err(|_| ToolError::InvalidInput("output limit exceeds platform size".into()))?;
        let request = ExecRequest {
            argv: input.argv.clone(),
            workdir: input.workdir.clone(),
            timeout,
            output_limit_bytes: Some(max_output),
            ..ExecRequest::default()
        };
        let output = self.backend.exec(&self.sandbox, &request).await?;
        let stdout = String::from_utf8(output.stdout.to_vec()).map_err(|_| {
            ToolError::InvalidInput("sandbox stdout is not UTF-8; output was not folded".into())
        })?;
        let stderr = String::from_utf8(output.stderr.to_vec()).map_err(|_| {
            ToolError::InvalidInput("sandbox stderr is not UTF-8; output was not folded".into())
        })?;
        let duration_ms = output
            .duration
            .as_secs()
            .saturating_mul(1_000)
            .saturating_add(u64::from(output.duration.subsec_millis()));
        Ok(ShellOutput {
            exit_code: output.exit_code,
            stdout,
            stderr,
            classification: classify_command(&input.argv, input.shell),
            duration_ms,
        })
    }
}

fn validate_shell_input(input: &ShellInput) -> Result<(), ToolError> {
    if input.argv.is_empty()
        || input.argv.len() > 128
        || input.argv.iter().any(|argument| {
            argument.is_empty() || argument.len() > 8_192 || argument.contains('\0')
        })
        || input.argv.iter().map(String::len).sum::<usize>() > 32_768
    {
        return Err(ToolError::InvalidInput(
            "sandbox argv is empty or exceeds configured bounds".to_owned(),
        ));
    }
    if let Some(workdir) = input.workdir.as_deref()
        && (workdir.is_empty()
            || workdir.starts_with(['/', '\\'])
            || workdir.contains('\0')
            || workdir
                .split(['/', '\\'])
                .any(|part| part == ".." || part == "."))
    {
        return Err(ToolError::InvalidInput(
            "sandbox workdir must be a contained relative path".to_owned(),
        ));
    }
    if input
        .timeout_ms
        .is_some_and(|timeout| timeout == 0 || timeout > 900_000)
    {
        return Err(ToolError::InvalidInput(
            "sandbox timeout must be between 1 and 900000 milliseconds".to_owned(),
        ));
    }
    Ok(())
}

/// JSON representation consumed by the generic builtin output bounder.
pub(crate) fn output_value(output: &ShellOutput) -> Value {
    json!({
        "exit_code": output.exit_code,
        "stdout": output.stdout,
        "stderr": output.stderr,
        "classification": output.classification,
        "duration_ms": output.duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        CommandClass, SandboxShellBackend, ShellFlavor, ShellInput, ShellToolBackend,
        classify_command,
    };
    use crate::ToolLimits;
    use bytes::Bytes;
    use polaris_sdk::registry::{
        ExecOutput, ExecRequest, SandboxBackend, SandboxError, SandboxHandle, SandboxSnapshotPage,
        SandboxSpec,
    };
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    #[derive(Debug, Default)]
    struct RecordingBackend {
        received: Mutex<Option<(SandboxHandle, ExecRequest)>>,
    }

    impl SandboxBackend for RecordingBackend {
        async fn create(&self, _spec: &SandboxSpec) -> Result<SandboxHandle, SandboxError> {
            Err(SandboxError::PolicyDenied("unused create".to_owned()))
        }

        async fn exec(
            &self,
            sandbox: &SandboxHandle,
            request: &ExecRequest,
        ) -> Result<ExecOutput, SandboxError> {
            let mut received = self
                .received
                .lock()
                .map_err(|_| SandboxError::Protocol("test lock poisoned".to_owned()))?;
            *received = Some((sandbox.clone(), request.clone()));
            Ok(ExecOutput {
                exit_code: 17,
                stdout: Bytes::from_static(b"out"),
                stderr: Bytes::from_static(b"err"),
                duration: Duration::from_millis(23),
            })
        }

        async fn upload(
            &self,
            _sandbox: &SandboxHandle,
            _path: &str,
            _bytes: Bytes,
        ) -> Result<(), SandboxError> {
            Err(SandboxError::PolicyDenied("unused upload".to_owned()))
        }

        async fn download(
            &self,
            _sandbox: &SandboxHandle,
            _path: &str,
        ) -> Result<Bytes, SandboxError> {
            Err(SandboxError::PolicyDenied("unused download".to_owned()))
        }

        async fn list_files(
            &self,
            _sandbox: &SandboxHandle,
            _dir: &str,
        ) -> Result<Vec<String>, SandboxError> {
            Err(SandboxError::PolicyDenied("unused list".to_owned()))
        }

        async fn snapshot_page(
            &self,
            _sandbox: &SandboxHandle,
            _root: &str,
            _generation: Option<&str>,
            _offset: u64,
            _limit: u32,
        ) -> Result<SandboxSnapshotPage, SandboxError> {
            Err(SandboxError::PolicyDenied("unused snapshot".to_owned()))
        }

        async fn destroy(&self, _sandbox: &SandboxHandle) -> Result<(), SandboxError> {
            Err(SandboxError::PolicyDenied("unused destroy".to_owned()))
        }
    }

    #[test]
    fn classifier_identifies_heavy_and_destructive_commands_without_execution() {
        let cases = [
            (
                vec![
                    "cargo".to_owned(),
                    "build".to_owned(),
                    "--workspace".to_owned(),
                ],
                ShellFlavor::Posix,
                CommandClass::Heavy,
            ),
            (
                vec!["nvcc".to_owned(), "kernel.cu".to_owned()],
                ShellFlavor::Posix,
                CommandClass::Heavy,
            ),
            (
                vec!["rm".to_owned(), "-rf".to_owned(), "target".to_owned()],
                ShellFlavor::Posix,
                CommandClass::Destructive,
            ),
            (
                vec![
                    "Remove-Item".to_owned(),
                    "-Recurse".to_owned(),
                    "build".to_owned(),
                ],
                ShellFlavor::PowerShell,
                CommandClass::Destructive,
            ),
            (
                vec!["git".to_owned(), "status".to_owned()],
                ShellFlavor::Posix,
                CommandClass::Unknown,
            ),
        ];
        for (argv, shell, expected) in cases {
            assert_eq!(classify_command(&argv, shell), expected);
        }
    }

    #[test]
    fn shell_route_uses_only_bound_handle_and_explicit_argv()
    -> Result<(), Box<dyn std::error::Error>> {
        let backend = Arc::new(RecordingBackend::default());
        let handle = SandboxHandle::new("runtime-owned-1")?;
        let shell = SandboxShellBackend::new(Arc::clone(&backend), handle.clone());
        let input = ShellInput {
            argv: vec![
                "cargo".to_owned(),
                "test".to_owned(),
                "-p".to_owned(),
                "sample".to_owned(),
            ],
            workdir: Some("workspace".to_owned()),
            shell: ShellFlavor::Posix,
            timeout_ms: Some(2_000),
        };
        let output = block_on(shell.execute(
            &input,
            ToolLimits {
                max_read_bytes: 1 << 20,
                max_results: 256,
                max_output_bytes: 2 << 20,
                max_input_bytes: 64 << 10,
                regex_size_limit: 2 << 20,
                timeout: Duration::from_secs(5),
            },
            Some(Duration::from_millis(900)),
        ))?;

        assert_eq!(output.exit_code, 17);
        assert_eq!(output.stdout, "out");
        assert_eq!(output.stderr, "err");
        assert_eq!(output.classification, CommandClass::Heavy);
        assert_eq!(output.duration_ms, 23);
        let received = backend
            .received
            .lock()
            .map_err(|_| "test lock poisoned")?
            .clone()
            .ok_or("sandbox exec was not called")?;
        assert_eq!(received.0, handle);
        assert_eq!(received.1.argv, input.argv);
        assert_eq!(received.1.workdir.as_deref(), Some("workspace"));
        assert_eq!(received.1.timeout, Some(Duration::from_millis(900)));
        assert_eq!(received.1.output_limit_bytes, Some(2 * 1024 * 1024));
        assert!(received.1.env.is_empty());
        Ok(())
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        let mut future = std::pin::pin!(future);
        loop {
            if let std::task::Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
        }
    }
}
