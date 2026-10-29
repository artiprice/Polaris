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
#[path = "../../../../../tests/crates/polaris-tools/crates/builtin/src/shell.rs"]
mod tests;
