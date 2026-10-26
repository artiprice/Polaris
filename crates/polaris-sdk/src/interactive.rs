//! Cold resource boundary for supervised bidirectional processes inside dedicated sandboxes.
//!
//! The dynamically dispatched handles cross a resource ownership boundary, not the gateway's hot
//! message loop. Launch remains statically dispatched. Sending EOF or requesting emergency cleanup
//! never proves termination; callers must await [`StdioControl::stop`] or [`StdioControl::wait`].

use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use bytes::Bytes;
use secrecy::SecretString;

use crate::registry::{SandboxError, SandboxHandle, SandboxSpec};

/// Bounded process launch input with explicitly secret environment values.
pub struct StdioRequest {
    /// Program and arguments executed without an implicit shell.
    pub argv: Vec<String>,
    /// Absolute working directory inside the new sandbox.
    pub workdir: Option<String>,
    /// Complete explicitly selected extra environment; no host environment is inherited.
    pub env: Vec<(String, SecretString)>,
    /// Maximum process lifetime. `None` uses the backend's configured execution deadline.
    pub timeout: Option<Duration>,
    /// Maximum stdout/stderr frame size accepted from the backend, at most 896 KiB.
    pub output_limit_bytes: Option<usize>,
}

impl std::fmt::Debug for StdioRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StdioRequest")
            .field("argument_count", &self.argv.len())
            .field("environment_count", &self.env.len())
            .finish_non_exhaustive()
    }
}

/// Future returned by a cold, object-safe process resource handle.
pub type StdioFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, SandboxError>> + Send + 'a>>;

/// One bounded process output event. An exit marker is provisional until clean stream EOF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StdioChunk {
    /// Exact standard-output bytes; never interpreted as trusted instructions.
    Stdout(Bytes),
    /// Exact standard-error bytes; callers must not log secret-bearing contents.
    Stderr(Bytes),
    /// Process exit status. A later transport error still invalidates completion.
    Exit(i32),
}

/// Owned input half. Implementations bound queued bytes and reject writes after EOF.
pub trait StdioInput: Send {
    /// Send exact bytes with backpressure.
    /// # Errors
    /// A closed input, configured size limit, deadline, or transport failure.
    fn send(&mut self, bytes: Bytes) -> StdioFuture<'_, ()>;
    /// Close only standard input; output and process ownership remain live.
    /// # Errors
    /// Failure to close the input transport.
    fn close(&mut self) -> StdioFuture<'_, ()>;
}

/// Owned output half. Bounds apply before copying or accumulating output in this adapter.
pub trait StdioOutput: Send {
    /// Read at most `max_bytes` of stdout/stderr, splitting larger bounded upstream frames.
    /// `None` means clean EOF after an exit marker; missing exits and failed trailers are errors.
    /// # Errors
    /// A zero/oversized bound, malformed stream, deadline, or transport failure.
    fn next(&mut self, max_bytes: usize) -> StdioFuture<'_, Option<StdioChunk>>;
}

/// Shared lifecycle owner. Implementations must bind cleanup to the created sandbox identity.
pub trait StdioControl: Send + Sync {
    /// Await an authoritative clean stream completion, including final transport status.
    /// # Errors
    /// An execution, protocol, transport, or shutdown error.
    fn wait(&self) -> StdioFuture<'_, i32>;
    /// Close input, allow bounded graceful termination, and confirm owned sandbox destruction.
    /// An implementation may force destruction after the grace period; successful cleanup is
    /// distinct from the process's exit status, which remains available through `wait`.
    /// # Errors
    /// Invalid grace, identity mismatch, unconfirmed deletion, or unavailable backend.
    fn stop(&self, grace: Duration) -> StdioFuture<'_, ()>;
    /// Request emergency cleanup synchronously. This does not acknowledge completion.
    /// Implementations retain cleanup ownership and diagnostics until cleanup is observed.
    fn request_stop(&self);
}

/// Split resource handles for one newly created, dedicated sandbox process.
pub struct InteractiveSession {
    /// Exact dedicated sandbox name. Cleanup additionally binds its immutable backend identity.
    pub sandbox: SandboxHandle,
    /// Bounded process input.
    pub input: Box<dyn StdioInput>,
    /// Bounded process output.
    pub output: Box<dyn StdioOutput>,
    /// Retained lifecycle control; it must outlive the I/O bridge tasks.
    pub control: Arc<dyn StdioControl>,
}

impl std::fmt::Debug for InteractiveSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InteractiveSession")
            .field("sandbox", &self.sandbox)
            .finish_non_exhaustive()
    }
}

/// Launch a bidirectional process inside a newly owned sandbox, never directly on the host.
/// Caller authorization precedes launch; an image/policy reference is not itself authorization.
pub trait InteractiveSandbox: Send + Sync {
    /// Create a dedicated sandbox and launch its process without a TTY or login shell.
    /// The implementation must reap partially created resources on failure or cancellation.
    /// # Errors
    /// Invalid limits/argv/policy, unavailable backend, startup failure, or unsupported transport.
    fn launch_stdio(
        &self,
        spec: &SandboxSpec,
        exec: &StdioRequest,
    ) -> impl Future<Output = Result<InteractiveSession, SandboxError>> + Send;
}

impl<T: InteractiveSandbox + ?Sized> InteractiveSandbox for Arc<T> {
    async fn launch_stdio(
        &self,
        spec: &SandboxSpec,
        exec: &StdioRequest,
    ) -> Result<InteractiveSession, SandboxError> {
        (**self).launch_stdio(spec, exec).await
    }
}
