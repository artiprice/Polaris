// Unit tests of crates/polaris-tools/crates/builtin/src/shell.rs (`tests`); compiled into that module via #[path].
use super::{
    CommandClass, SandboxShellBackend, ShellFlavor, ShellInput, ShellToolBackend, classify_command,
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

    async fn download(&self, _sandbox: &SandboxHandle, _path: &str) -> Result<Bytes, SandboxError> {
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
fn shell_route_uses_only_bound_handle_and_explicit_argv() -> Result<(), Box<dyn std::error::Error>>
{
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
