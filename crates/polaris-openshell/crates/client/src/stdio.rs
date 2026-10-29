//! Interactive OpenShell transport. Every process receives a newly owned sandbox.
use crate::{MAX_REQUEST_CHUNK_BYTES, OpenShellBackend, PolicySource};
use bytes::Bytes;
use futures::StreamExt;
use openshell_sdk::{
    OpenShellClient, SdkError,
    raw::{self, proto},
};
use polaris_sdk::{
    interactive::{
        InteractiveSandbox, InteractiveSession, StdioChunk, StdioControl, StdioFuture, StdioInput,
        StdioOutput, StdioRequest,
    },
    registry::{SandboxBackend, SandboxError, SandboxSpec},
};
use secrecy::ExposeSecret;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, mpsc, watch};

const QUEUE: usize = 8;
const MAX_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
type Completion = Option<Result<i32, SandboxError>>;

// Static dispatch keeps the exact identity/acknowledgement routine independently testable.
trait CleanupGateway: Send + Sync {
    fn inspect(
        &self,
        name: &str,
    ) -> impl std::future::Future<Output = Result<Option<(String, String)>, SandboxError>> + Send;
    fn delete(
        &self,
        name: &str,
    ) -> impl std::future::Future<Output = Result<Option<String>, SandboxError>> + Send;
    fn await_deleted(
        &self,
        name: &str,
        id: &str,
    ) -> impl std::future::Future<Output = Result<(), SandboxError>> + Send;
}
impl CleanupGateway for OpenShellClient {
    async fn inspect(&self, name: &str) -> Result<Option<(String, String)>, SandboxError> {
        match self.get_sandbox(name).await {
            Ok(snapshot) => Ok(Some((snapshot.name, snapshot.id))),
            Err(SdkError::NotFound { .. }) => Ok(None),
            Err(_) => Err(unavailable()),
        }
    }
    async fn delete(&self, name: &str) -> Result<Option<String>, SandboxError> {
        self.delete_sandbox(name, Default::default())
            .await
            .map(|receipt| receipt.sandbox_id)
            .map_err(|_| unavailable())
    }
    async fn await_deleted(&self, name: &str, id: &str) -> Result<(), SandboxError> {
        self.wait_deleted(name, Duration::from_secs(30), Some(id))
            .await
            .map_err(|_| unavailable())
    }
}
async fn delete_owned<G: CleanupGateway>(
    gateway: &G,
    name: &str,
    id: &str,
) -> Result<(), SandboxError> {
    // The gateway has no conditional delete API. Private nonce names and root lifecycle
    // ownership exclude cooperating replacement; this preflight is not a remote CAS.
    match gateway.inspect(name).await? {
        None => return Ok(()),
        Some((observed_name, _)) if observed_name != name => {
            return Err(protocol("interactive lookup identity mismatch"));
        }
        Some((_, observed_id)) if observed_id != id => return Ok(()),
        Some(_) => {}
    }
    let receipt = gateway.delete(name).await?;
    if receipt
        .as_ref()
        .is_some_and(|observed_id| observed_id != id)
    {
        return Err(protocol("interactive deletion identity mismatch"));
    }
    gateway.await_deleted(name, id).await
}

struct Lifecycle {
    client: Arc<OpenShellClient>,
    name: String,
    id: String,
    input: Mutex<Option<mpsc::Sender<proto::ExecSandboxInput>>>,
    stopped: watch::Sender<bool>,
    completion: watch::Receiver<Completion>,
    cleanup: Mutex<bool>,
    emergency_started: AtomicBool,
}

impl Lifecycle {
    async fn finish(&self) -> Result<i32, SandboxError> {
        let mut receiver = self.completion.clone();
        loop {
            if let Some(result) = receiver.borrow().clone() {
                return result;
            }
            receiver
                .changed()
                .await
                .map_err(|_| protocol("interactive owner ended without completion"))?;
        }
    }
    async fn delete_owned(&self) -> Result<(), SandboxError> {
        let mut cleaned = self.cleanup.lock().await;
        if *cleaned {
            return Ok(());
        }
        delete_owned(self.client.as_ref(), &self.name, &self.id).await?;
        *cleaned = true;
        Ok(())
    }
    fn emergency(self: &Arc<Self>) {
        self.stopped.send_replace(true);
        if self.cleanup.try_lock().is_ok_and(|cleaned| *cleaned) {
            return;
        }
        if self.emergency_started.swap(true, Ordering::AcqRel) {
            return;
        }
        let state = Arc::clone(self);
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                drop(runtime.spawn(async move {
                    let result = tokio::time::timeout(Duration::from_secs(40), state.delete_owned())
                        .await
                        .map_err(|_| SandboxError::Timeout)
                        .and_then(|result| result);
                    if let Err(error) = result {
                    state.emergency_started.store(false, Ordering::Release);
                    tracing::error!(%error, "interactive sandbox cleanup failed; explicit retry required");
                }
            }));
            }
            Err(_) => {
                self.emergency_started.store(false, Ordering::Release);
                tracing::error!("interactive cleanup requested after Tokio runtime stopped");
            }
        }
    }
}

struct Control(Arc<Lifecycle>);
impl StdioControl for Control {
    fn wait(&self) -> StdioFuture<'_, i32> {
        Box::pin(self.0.finish())
    }
    fn stop(&self, grace: Duration) -> StdioFuture<'_, ()> {
        Box::pin(async move {
            if grace > Duration::from_secs(120) {
                return Err(protocol("interactive shutdown grace exceeds limit"));
            }
            self.0.input.lock().await.take();
            // Cleanup success and process success are deliberately separate outcomes. Even a
            // failed execution or expired grace must proceed to destruction; wait retains it.
            let _completion = tokio::time::timeout(grace, self.0.finish()).await;
            self.0.stopped.send_replace(true);
            tokio::time::timeout(Duration::from_secs(40), self.0.delete_owned())
                .await
                .map_err(|_| SandboxError::Timeout)?
        })
    }
    fn request_stop(&self) {
        self.0.emergency();
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.0.emergency();
    }
}

struct Input(Arc<Lifecycle>);
impl Drop for Input {
    fn drop(&mut self) {
        match self.0.input.try_lock() {
            Ok(mut sender) => {
                sender.take();
            }
            Err(_) => self.0.emergency(),
        }
    }
}
impl StdioInput for Input {
    fn send(&mut self, bytes: Bytes) -> StdioFuture<'_, ()> {
        Box::pin(async move {
            if bytes.len() > MAX_REQUEST_CHUNK_BYTES {
                return Err(limit("interactive input frame too large"));
            }
            if *self.0.stopped.borrow() {
                return Err(protocol("interactive process stopped"));
            }
            let sender = self
                .0
                .input
                .lock()
                .await
                .as_ref()
                .cloned()
                .ok_or_else(|| protocol("interactive input is closed"))?;
            tokio::time::timeout(
                Duration::from_secs(30),
                sender.send(proto::ExecSandboxInput {
                    payload: Some(proto::exec_sandbox_input::Payload::Stdin(bytes.to_vec())),
                }),
            )
            .await
            .map_err(|_| SandboxError::Timeout)?
            .map_err(|_| unavailable())
        })
    }
    fn close(&mut self) -> StdioFuture<'_, ()> {
        Box::pin(async move {
            self.0.input.lock().await.take();
            Ok(())
        })
    }
}

struct Output {
    receiver: mpsc::Receiver<Result<StdioChunk, SandboxError>>,
    pending: Option<StdioChunk>,
    state: Arc<Lifecycle>,
    terminal: Option<Result<(), SandboxError>>,
}
impl StdioOutput for Output {
    fn next(&mut self, max_bytes: usize) -> StdioFuture<'_, Option<StdioChunk>> {
        Box::pin(async move {
            if max_bytes == 0 || max_bytes > MAX_REQUEST_CHUNK_BYTES {
                return Err(limit("invalid interactive read bound"));
            }
            if let Some(result) = &self.terminal {
                result.clone()?;
                return Ok(None);
            }
            let item = match self.pending.take() {
                Some(item) => Some(Ok(item)),
                None => self.receiver.recv().await,
            };
            let Some(item) = item else {
                let result = self.state.finish().await.map(|_| ());
                self.terminal = Some(result.clone());
                result?;
                return Ok(None);
            };
            let mut chunk = item?;
            match &mut chunk {
                StdioChunk::Stdout(bytes) if bytes.len() > max_bytes => {
                    let first = bytes.split_to(max_bytes);
                    self.pending = Some(StdioChunk::Stdout(bytes.clone()));
                    Ok(Some(StdioChunk::Stdout(first)))
                }
                StdioChunk::Stderr(bytes) if bytes.len() > max_bytes => {
                    let first = bytes.split_to(max_bytes);
                    self.pending = Some(StdioChunk::Stderr(bytes.clone()));
                    Ok(Some(StdioChunk::Stderr(first)))
                }
                StdioChunk::Stdout(_) | StdioChunk::Stderr(_) | StdioChunk::Exit(_) => {
                    Ok(Some(chunk))
                }
            }
        })
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        if self.terminal.is_none() {
            self.state.emergency();
        }
    }
}

fn protocol(message: &'static str) -> SandboxError {
    SandboxError::Protocol(message.into())
}
fn limit(message: &'static str) -> SandboxError {
    SandboxError::LimitExceeded(message.into())
}
fn unavailable() -> SandboxError {
    SandboxError::Unavailable("interactive sandbox transport failed".into())
}

fn validate(request: &StdioRequest, lifetime: Duration) -> Result<usize, SandboxError> {
    if lifetime.is_zero() || lifetime > MAX_LIFETIME {
        return Err(protocol("invalid interactive process lifetime"));
    }
    if request.argv.is_empty() || request.argv.len() > 256 {
        return Err(protocol("invalid interactive argument count"));
    }
    let mut total = 0_usize;
    for argument in &request.argv {
        if argument.len() > 8192 || argument.contains('\0') {
            return Err(protocol("invalid interactive argument"));
        }
        total = total
            .checked_add(argument.len())
            .ok_or_else(|| limit("interactive launch size overflow"))?;
    }
    if request
        .argv
        .first()
        .is_none_or(|program| program.is_empty())
    {
        return Err(protocol("interactive program is empty"));
    }
    if let Some(path) = &request.workdir {
        if !path.starts_with('/') || path.len() > 4096 || path.contains('\0') {
            return Err(protocol("invalid interactive working directory"));
        }
        total = total
            .checked_add(path.len())
            .ok_or_else(|| limit("interactive launch size overflow"))?;
    }
    if request.env.len() > 128 {
        return Err(limit("too many interactive environment entries"));
    }
    let mut names = std::collections::HashSet::new();
    for (name, value) in &request.env {
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || !names.insert(name)
            || value.expose_secret().contains('\0')
        {
            return Err(protocol("invalid interactive environment"));
        }
        total = total
            .checked_add(name.len())
            .and_then(|n| n.checked_add(value.expose_secret().len()))
            .ok_or_else(|| limit("interactive launch size overflow"))?;
    }
    if total > 128 * 1024 {
        return Err(limit("interactive launch request too large"));
    }
    let bound = match request.output_limit_bytes {
        Some(value) => value,
        None => MAX_REQUEST_CHUNK_BYTES,
    };
    if bound == 0 || bound > MAX_REQUEST_CHUNK_BYTES {
        return Err(limit("invalid interactive output frame bound"));
    }
    Ok(bound)
}

impl<P: PolicySource> InteractiveSandbox for OpenShellBackend<P> {
    async fn launch_stdio(
        &self,
        spec: &SandboxSpec,
        request: &StdioRequest,
    ) -> Result<InteractiveSession, SandboxError> {
        let lifetime = match request.timeout {
            Some(value) => value,
            None => self.config.exec_timeout,
        };
        let bound = validate(request, lifetime)?;
        if spec.name.is_some() {
            return Err(protocol(
                "interactive sandbox name must be generated by its owner",
            ));
        }
        let mut dedicated = spec.clone();
        let unique = blake3::hash(uuid::Uuid::now_v7().as_bytes()).to_hex();
        let suffix: String = unique.chars().take(16).collect();
        dedicated.name = Some(format!("mcp{suffix}"));
        let sandbox = self.create(&dedicated).await?;
        let mut startup = crate::implementation::SandboxCleanupGuard::new(
            Arc::clone(&self.client),
            sandbox.as_str().to_owned(),
        );
        let snapshot = self
            .client
            .get_sandbox(sandbox.as_str())
            .await
            .map_err(|_| unavailable())?;
        if snapshot.name != sandbox.as_str() || uuid::Uuid::parse_str(&snapshot.id).is_err() {
            return Err(protocol("invalid interactive sandbox identity"));
        }
        let (sender, receiver) = mpsc::channel(QUEUE);
        let environment = request
            .env
            .iter()
            .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
            .collect::<HashMap<_, _>>();
        sender
            .send(proto::ExecSandboxInput {
                payload: Some(proto::exec_sandbox_input::Payload::Start(
                    raw::ExecSandboxRequest {
                        request_id: uuid::Uuid::now_v7().to_string(),
                        sandbox: sandbox.as_str().to_owned(),
                        workspace_scope: Some(proto::workspace_selector("default")),
                        command: request.argv.clone(),
                        workdir: match &request.workdir {
                            Some(value) => value.clone(),
                            None => String::new(),
                        },
                        environment,
                        execution_timeout: None,
                        stdin: Vec::new(),
                        tty: false,
                        cols: 0,
                        rows: 0,
                        no_login_shell: true,
                    },
                )),
            })
            .await
            .map_err(|_| unavailable())?;
        let input_stream = futures::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|frame| (frame, receiver))
        });
        let output = tokio::time::timeout(
            Duration::from_secs(30),
            self.client
                .raw_grpc()
                .exec_sandbox_interactive(input_stream),
        )
        .await
        .map_err(|_| SandboxError::Timeout)?
        .map_err(|_| unavailable())?
        .into_inner();
        let (completed_tx, completed_rx) = watch::channel(None);
        let (stop_tx, stop_rx) = watch::channel(false);
        let state = Arc::new(Lifecycle {
            client: Arc::clone(&self.client),
            name: sandbox.as_str().to_owned(),
            id: snapshot.id,
            input: Mutex::new(Some(sender)),
            stopped: stop_tx,
            completion: completed_rx,
            cleanup: Mutex::new(false),
            emergency_started: AtomicBool::new(false),
        });
        let (events_tx, events_rx) = mpsc::channel(QUEUE);
        let pump_state = Arc::clone(&state);
        drop(tokio::spawn(async move {
            let result = pump(output, events_tx, stop_rx, lifetime, bound).await;
            completed_tx.send_replace(Some(result));
            pump_state.emergency();
        }));
        startup.disarm();
        Ok(InteractiveSession {
            sandbox,
            input: Box::new(Input(Arc::clone(&state))),
            output: Box::new(Output {
                receiver: events_rx,
                pending: None,
                state: Arc::clone(&state),
                terminal: None,
            }),
            control: Arc::new(Control(state)),
        })
    }
}

async fn pump<S>(
    mut stream: S,
    sender: mpsc::Sender<Result<StdioChunk, SandboxError>>,
    mut stopped: watch::Receiver<bool>,
    lifetime: Duration,
    bound: usize,
) -> Result<i32, SandboxError>
where
    S: futures::Stream<Item = Result<proto::ExecSandboxEvent, tonic::Status>> + Unpin,
{
    let deadline = tokio::time::Instant::now() + lifetime;
    let mut exit = None;
    loop {
        let event = tokio::select! {
            biased;
            () = sender.closed() => return Err(protocol("interactive output consumer closed")),
            result = stopped.changed() => { let _changed = result; return Err(protocol("interactive process stopped by owner")); },
            () = tokio::time::sleep_until(deadline) => return Err(SandboxError::Timeout),
            event = stream.next() => event,
        };
        let Some(event) = event else {
            return exit.ok_or_else(|| protocol("interactive EOF without exit"));
        };
        let event = event.map_err(|_| unavailable())?;
        if exit.is_some() {
            return Err(protocol("interactive payload after exit"));
        }
        let chunk = match event.payload {
            Some(proto::exec_sandbox_event::Payload::Stdout(frame)) => {
                if frame.data.len() > bound {
                    return Err(limit("interactive stdout frame exceeded limit"));
                }
                StdioChunk::Stdout(Bytes::from(frame.data))
            }
            Some(proto::exec_sandbox_event::Payload::Stderr(frame)) => {
                if frame.data.len() > bound {
                    return Err(limit("interactive stderr frame exceeded limit"));
                }
                StdioChunk::Stderr(Bytes::from(frame.data))
            }
            Some(proto::exec_sandbox_event::Payload::Exit(status)) => {
                exit = Some(status.exit_code);
                StdioChunk::Exit(status.exit_code)
            }
            None => return Err(protocol("empty interactive output event")),
        };
        tokio::select! {
            result = sender.send(Ok(chunk)) => result.map_err(|_| protocol("interactive output consumer closed"))?,
            _ = stopped.changed() => return Err(protocol("interactive process stopped by owner")),
            () = tokio::time::sleep_until(deadline) => return Err(SandboxError::Timeout),
        }
    }
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-openshell/crates/client/src/stdio.rs"]
mod tests;
