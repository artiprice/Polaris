//! Bounded tonic adapters for the SDK kernel API and an injected terminal backend.
use crate::auth::{AuthorizationLease, RequestSubject, RpcAuthenticator, TrustedFactsResolver};
use futures::{Stream, StreamExt};
use polaris_proto::{limits, pb};
use polaris_sdk::{
    contracts::{Command, Unverified},
    ids::{Seq, SessionId},
    registry::{EventSource, KernelApi, KernelError},
};
use polaris_security_capability::Right;
use prost::Message;
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use thiserror::Error;
use tokio::{
    sync::{Semaphore, mpsc, watch},
    task::JoinHandle,
    time,
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

/// Bounded stream policy. Buffer exhaustion disconnects a slow event subscriber with
/// `RESOURCE_EXHAUSTED`; event delivery never accumulates an unbounded queue in this adapter.
#[derive(Debug, Clone, Copy)]
pub struct RpcLimits {
    /// Maximum queued event records per subscriber; valid range is 1..=1024.
    pub event_buffer: usize,
    /// Maximum time the event producer waits for a queue slot before detaching the subscriber.
    pub backpressure_timeout: Duration,
}
const AUTH_RECHECK_INTERVAL: Duration = Duration::from_secs(1);

/// Terminal adapter injected by the composition root. The RPC crate owns no PTY or sandbox.
pub trait TerminalApi: Send + Sync + 'static {
    /// Outbound terminal frames.
    type Stream: Stream<Item = Result<pb::TerminalServerFrame, Status>> + Send + 'static;

    /// Attach to a terminal implementation using a size-checked input stream.
    fn attach(
        &self,
        inbound: Pin<Box<dyn Stream<Item = Result<pb::TerminalClientFrame, Status>> + Send>>,
        cancellation: watch::Receiver<bool>,
    ) -> impl Future<Output = Result<Self::Stream, Status>> + Send;
}

/// Actual kernel readiness probe, injected because `KernelApi` intentionally exposes only submit
/// and subscribe operations.
pub trait Readiness: Send + Sync + 'static {
    /// Whether the composition root is ready to accept work.
    fn is_ready(&self) -> bool;
}

struct TerminalLifecycle {
    cancellation: watch::Sender<bool>,
    watchdog: Option<JoinHandle<()>>,
}
impl Drop for TerminalLifecycle {
    fn drop(&mut self) {
        let _sent = self.cancellation.send(true);
        if let Some(watchdog) = self.watchdog.take() {
            watchdog.abort();
        }
    }
}

/// Typed tonic service adapter. It never bypasses command verification or fabricates a terminal.
pub struct RpcService<K, T, A, H> {
    kernel: Arc<K>,
    terminal: T,
    auth: RpcAuthenticator<A>,
    readiness: H,
    limits: RpcLimits,
}

impl<K, T, A, H> RpcService<K, T, A, H>
where
    K: KernelApi + 'static,
    K::Events: 'static,
    T: TerminalApi,
    A: TrustedFactsResolver,
    H: Readiness,
{
    /// Create the service after validating bounded event-stream settings.
    /// # Errors
    /// Returns `InvalidLimits` for zero, excessive, or unbounded settings.
    pub fn new(
        kernel: K,
        terminal: T,
        auth: RpcAuthenticator<A>,
        readiness: H,
        limits: RpcLimits,
    ) -> Result<Self, RpcError> {
        if limits.event_buffer == 0
            || limits.event_buffer > 1024
            || limits.event_buffer.checked_add(1).is_none()
            || limits.backpressure_timeout.is_zero()
        {
            return Err(RpcError::InvalidLimits);
        }
        Ok(Self {
            kernel: Arc::new(kernel),
            terminal,
            auth,
            readiness,
            limits,
        })
    }

    /// Wrap the adapter in the generated tonic service with protocol message bounds.
    #[must_use]
    pub fn into_grpc_service(
        self,
    ) -> pb::polaris_kernel_service_server::PolarisKernelServiceServer<Self> {
        pb::polaris_kernel_service_server::PolarisKernelServiceServer::new(self)
            .max_decoding_message_size(limits::MAX_MESSAGE_BYTES)
            .max_encoding_message_size(limits::MAX_MESSAGE_BYTES)
    }
}

#[tonic::async_trait]
impl<K, T, A, H> pb::polaris_kernel_service_server::PolarisKernelService for RpcService<K, T, A, H>
where
    K: KernelApi + 'static,
    K::Events: 'static,
    T: TerminalApi,
    A: TrustedFactsResolver,
    H: Readiness,
{
    type SubscribeStream =
        Pin<Box<dyn Stream<Item = Result<pb::EventRecord, Status>> + Send + 'static>>;
    type AttachTerminalStream =
        Pin<Box<dyn Stream<Item = Result<pb::TerminalServerFrame, Status>> + Send + 'static>>;

    async fn submit_command(
        &self,
        request: Request<pb::SubmitCommandRequest>,
    ) -> Result<Response<pb::SubmitCommandResponse>, Status> {
        let command = request
            .get_ref()
            .command
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("command is required"))?;
        if command.token.len() > polaris_security_capability::MAX_TOKEN_BYTES {
            return Err(Status::resource_exhausted("command capability too large"));
        }
        let body = command
            .body
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("command body is required"))?;
        let right = command_right(body)?;
        self.auth.authorize(
            &request,
            right,
            RequestSubject::SubmitCommand(body),
            Some(command.token.as_slice()),
        )?;
        let wire = request
            .into_inner()
            .command
            .ok_or_else(|| Status::invalid_argument("command is required"))?;
        let command = Command::<Unverified>::try_from(wire)
            .map_err(|error| Status::invalid_argument(error.to_string()))?;
        let receipt = self.kernel.submit(command).await.map_err(kernel_status)?;
        Ok(Response::new(pb::SubmitCommandResponse {
            command_id: Some(pb::Uuid::from(&receipt.command)),
            accepted_at: Some(receipt.accepted_at.get()),
        }))
    }

    async fn subscribe(
        &self,
        request: Request<pb::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeStream>, Status> {
        let subscribe = request.get_ref();
        let session_wire = subscribe
            .session
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("session is required"))?;
        let lease = self.auth.authorize(
            &request,
            Right::Read,
            RequestSubject::Subscribe(subscribe),
            None,
        )?;
        let session_id = u128::from(session_wire.hi) << 64 | u128::from(session_wire.lo);
        let session = SessionId::from_u128(session_id);
        let from = Seq::new(subscribe.from_seq);
        let events = self.kernel.subscribe(from).await.map_err(kernel_status)?;
        let (sender, receiver) = mpsc::channel(self.limits.event_buffer + 1);
        let permits = Arc::new(Semaphore::new(self.limits.event_buffer));
        let backpressure_timeout = self.limits.backpressure_timeout;
        tokio::spawn(async move {
            pump_events(
                events,
                session,
                from,
                sender,
                permits,
                backpressure_timeout,
                lease,
            )
            .await;
        });
        let stream = ReceiverStream::new(receiver).map(|(item, permit)| {
            drop(permit);
            item
        });
        Ok(Response::new(Box::pin(stream)))
    }

    async fn attach_terminal(
        &self,
        request: Request<Streaming<pb::TerminalClientFrame>>,
    ) -> Result<Response<Self::AttachTerminalStream>, Status> {
        let lease = self.auth.authorize(
            &request,
            Right::ExecuteTool,
            RequestSubject::AttachTerminal,
            None,
        )?;
        let input_lease = lease.clone();
        let (cancellation, cancel_rx) = watch::channel(false);
        let input_cancellation = cancellation.clone();
        let input_cancel_rx = cancel_rx.clone();
        let inbound = request.into_inner().map(move |result| {
            if *input_cancel_rx.borrow() {
                return Err(Status::unauthenticated(
                    "terminal authorization expired or revoked",
                ));
            }
            if let Err(status) = input_lease.revalidate() {
                let _sent = input_cancellation.send(true);
                return Err(status);
            }
            let frame = result?;
            if frame.encoded_len() > limits::MAX_CHUNK_BYTES {
                return Err(Status::resource_exhausted("terminal input frame too large"));
            }
            if let Some(pb::terminal_client_frame::Kind::Input(bytes)) = &frame.kind
                && bytes.len() > limits::MAX_CHUNK_BYTES
            {
                return Err(Status::resource_exhausted("terminal input chunk too large"));
            }
            Ok(frame)
        });
        let watchdog_cancellation = cancellation.clone();
        let watchdog_lease = lease.clone();
        let watchdog = tokio::spawn(async move {
            let mut interval = time::interval(AUTH_RECHECK_INTERVAL);
            interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if watchdog_lease.revalidate().is_err() {
                    let _sent = watchdog_cancellation.send(true);
                    return;
                }
            }
        });
        let lifecycle = TerminalLifecycle {
            cancellation,
            watchdog: Some(watchdog),
        };
        // Authorization remains live while the injected backend is attaching too. A backend
        // that is waiting for its first input must not keep an expired attachment alive.
        let mut attach_cancellation = cancel_rx.clone();
        let attach = self.terminal.attach(Box::pin(inbound), cancel_rx.clone());
        let stream = tokio::select! {
            result = attach => result?,
            changed = attach_cancellation.changed() => {
                let _ = changed;
                return Err(Status::unauthenticated("terminal authorization expired or revoked"));
            }
        };
        let output = Box::pin(stream);
        let mut rechecks = time::interval(AUTH_RECHECK_INTERVAL);
        rechecks.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        let output_cancel_rx = cancel_rx;
        let bounded = futures::stream::unfold(
            (output, lease, rechecks, output_cancel_rx, lifecycle, false),
            |(mut output, lease, mut rechecks, mut cancellation, lifecycle, done)| async move {
                if done {
                    return None;
                }
                loop {
                    tokio::select! {
                        changed = cancellation.changed() => {
                            if changed.is_err() || *cancellation.borrow() {
                                return Some((Err(Status::unauthenticated("terminal authorization expired or revoked")), (output, lease, rechecks, cancellation, lifecycle, true)));
                            }
                        },
                        result = output.next() => match result {
                            Some(Err(status)) => {
                                let _sent = lifecycle.cancellation.send(true);
                                return Some((Err(status), (output, lease, rechecks, cancellation, lifecycle, true)));
                            }
                            Some(Ok(frame)) => {
                                if let Err(status) = lease.revalidate() {
                                    let _sent = lifecycle.cancellation.send(true);
                                    return Some((Err(status), (output, lease, rechecks, cancellation, lifecycle, true)));
                                }
                                if frame.encoded_len() > limits::MAX_MESSAGE_BYTES {
                                    let _sent = lifecycle.cancellation.send(true);
                                    return Some((Err(Status::resource_exhausted("terminal output frame too large")), (output, lease, rechecks, cancellation, lifecycle, true)));
                                }
                                if let Some(pb::terminal_server_frame::Kind::Output(bytes)) = &frame.kind
                                    && bytes.len() > limits::MAX_CHUNK_BYTES {
                                        let _sent = lifecycle.cancellation.send(true);
                                        return Some((Err(Status::resource_exhausted("terminal output chunk too large")), (output, lease, rechecks, cancellation, lifecycle, true)));
                                }
                                return Some((Ok(frame), (output, lease, rechecks, cancellation, lifecycle, false)));
                            }
                            None => return None,
                        },
                        _ = rechecks.tick() => {
                            if let Err(status) = lease.revalidate() {
                                let _sent = lifecycle.cancellation.send(true);
                                return Some((Err(status), (output, lease, rechecks, cancellation, lifecycle, true)));
                            }
                        }
                    }
                }
            },
        );
        Ok(Response::new(Box::pin(bounded)))
    }

    async fn health(
        &self,
        request: Request<pb::HealthRequest>,
    ) -> Result<Response<pb::HealthResponse>, Status> {
        self.auth
            .authorize(&request, Right::Read, RequestSubject::Health, None)?;
        Ok(Response::new(pb::HealthResponse {
            protocol_version: Some(polaris_proto::PROTOCOL_VERSION.into()),
            build_version: env!("CARGO_PKG_VERSION").to_owned(),
            ready: self.readiness.is_ready(),
        }))
    }
}

async fn pump_events<E: EventSource + 'static, A: TrustedFactsResolver>(
    mut events: E,
    session: SessionId,
    from: Seq,
    sender: mpsc::Sender<(
        Result<pb::EventRecord, Status>,
        Option<tokio::sync::OwnedSemaphorePermit>,
    )>,
    permits: Arc<Semaphore>,
    backpressure_timeout: Duration,
    lease: AuthorizationLease<A>,
) {
    let mut last = Seq::new(from.get().saturating_sub(1));
    let mut rechecks = time::interval(AUTH_RECHECK_INTERVAL);
    rechecks.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    loop {
        let next = tokio::select! {
            _ = sender.closed() => return,
            _ = rechecks.tick() => {
                if let Err(status) = lease.revalidate() {
                    let _sent = sender.try_send((Err(status), None));
                    return;
                }
                continue;
            },
            event = events.next_event() => event,
        };
        let Some(record) = next else {
            return;
        };
        if let Err(status) = lease.revalidate() {
            let _sent = sender.try_send((Err(status), None));
            return;
        }
        let sequence = record.seq();
        if sequence <= last {
            continue;
        }
        last = sequence;
        if record.session() != session {
            continue;
        }
        let wire = pb::EventRecord::from(&record);
        if wire.encoded_len() > limits::MAX_MESSAGE_BYTES {
            let _sent = sender.try_send((
                Err(Status::resource_exhausted("event record too large")),
                None,
            ));
            return;
        }
        let acquire = permits.clone().acquire_owned();
        tokio::pin!(acquire);
        let deadline = time::sleep(backpressure_timeout);
        tokio::pin!(deadline);
        let permit = loop {
            tokio::select! {
                _ = sender.closed() => return,
                _ = rechecks.tick() => {
                    if let Err(status) = lease.revalidate() {
                        let _sent = sender.try_send((Err(status), None));
                        return;
                    }
                }
                result = &mut acquire => match result {
                    Ok(permit) => break permit,
                    Err(_) => return,
                },
                _ = &mut deadline => {
                    let _sent = sender.try_send((Err(Status::resource_exhausted("event subscriber is too slow")), None));
                    return;
                }
            }
        };
        if sender.send((Ok(wire), Some(permit))).await.is_err() {
            return;
        }
    }
}

fn command_right(body: &pb::CommandBody) -> Result<Right, Status> {
    use pb::command_body::Kind;
    match body.kind.as_ref() {
        Some(Kind::SubmitUserInput(_)) => Ok(Right::Read),
        Some(Kind::ProposeDiff(_)) => Ok(Right::ProposeDiff),
        Some(Kind::AcquireAstScope(_)) | Some(Kind::ReleaseAstScope(_)) => Ok(Right::Read),
        Some(Kind::SpawnSubagent(_)) => Ok(Right::SpawnSubagent),
        Some(Kind::ExecuteTool(_)) => Ok(Right::ExecuteTool),
        Some(Kind::RollbackTo(_)) | Some(Kind::CancelFrame(_)) | Some(Kind::ApplyDirective(_)) => {
            Err(Status::permission_denied(
                "command has no RPC capability right",
            ))
        }
        None => Err(Status::invalid_argument("command kind is required")),
    }
}

fn kernel_status(error: KernelError) -> Status {
    match error {
        KernelError::Rejected(_) => Status::permission_denied("command rejected"),
        KernelError::Busy => Status::resource_exhausted("kernel busy"),
        KernelError::Closed => Status::unavailable("kernel closed"),
        KernelError::Internal(_) => Status::internal("kernel failure"),
    }
}

/// RPC adapter configuration or startup error.
#[derive(Debug, Error)]
pub enum RpcError {
    /// Invalid bounded stream configuration.
    #[error("invalid RPC limits")]
    InvalidLimits,
}
