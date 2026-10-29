//! Real daemon lifecycle over an isolated local pipe or private Unix socket.
use futures::Stream;
use polaris_daemon::{
    DaemonConfig, DaemonListenerConfig, DaemonSettings, EndpointSettings, serve_daemon,
    serve_kernel_api,
};
use polaris_kernel::{Kernel, LocalKernelConfig};
use polaris_rpc::{
    ClientError, ClientTransport, KernelClient, LocalEndpoint, Readiness, RequestSubject,
    RpcAuthenticator, TerminalApi, TrustedFacts, TrustedFactsResolver,
};
use polaris_sdk::{
    contracts::{Command, CommandBody},
    ids::{Cid, CommandId, FrameId, SessionId},
};
use polaris_security_capability::{
    Authority, ResourcePath, ResourceScope, Right, Rights, Scope, SystemClock,
};
use std::{error::Error, pin::Pin, time::Duration};
use tokio::sync::watch;
use tonic::Status;

type TestResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug, Clone, Copy)]
struct Ready;
impl Readiness for Ready {
    fn is_ready(&self) -> bool {
        true
    }
}

struct Resolver(SessionId);
impl TrustedFactsResolver for Resolver {
    fn resolve(&self, _right: Right, subject: RequestSubject<'_>) -> Result<TrustedFacts, Status> {
        if let RequestSubject::Subscribe(request) = subject {
            let Some(session) = &request.session else {
                return Err(Status::invalid_argument("session is required"));
            };
            let requested = (u128::from(session.hi) << 64) | u128::from(session.lo);
            if requested != self.0.as_uuid().as_u128() {
                return Err(Status::permission_denied(
                    "session is outside fixture scope",
                ));
            }
        }
        Ok(TrustedFacts {
            resource: ResourcePath::new(&format!("sessions/{}/rpc", self.0))
                .map_err(|_| Status::internal("invalid fixture resource"))?,
            symbol: None,
            churn_lines: 0,
            touches_unsafe: false,
            diff_blake3: None,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct NoTerminal;
impl TerminalApi for NoTerminal {
    type Stream = futures::stream::Empty<Result<polaris_proto::pb::TerminalServerFrame, Status>>;

    async fn attach(
        &self,
        _inbound: Pin<
            Box<dyn Stream<Item = Result<polaris_proto::pb::TerminalClientFrame, Status>> + Send>,
        >,
        _cancellation: watch::Receiver<bool>,
    ) -> Result<Self::Stream, Status> {
        Err(Status::failed_precondition(
            "test profile has no terminal backend",
        ))
    }
}

#[cfg(windows)]
fn endpoint() -> TestResult<(EndpointSettings, LocalEndpoint)> {
    let name = format!("polaris-daemon-{}", CommandId::new_v7());
    let server = EndpointSettings::NamedPipe(name.clone());
    let local = LocalEndpoint::named_pipe(&name)?;
    Ok((server, local))
}

#[cfg(unix)]
fn endpoint(directory: &std::path::Path) -> TestResult<(EndpointSettings, LocalEndpoint)> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    let path = directory.join("daemon.sock");
    let endpoint = LocalEndpoint::unix(path.clone())?;
    Ok((EndpointSettings::Unix(path), endpoint))
}

async fn connect_local(
    endpoint: LocalEndpoint,
    session: SessionId,
    credential: polaris_sdk::contracts::TokenBytes,
) -> TestResult<KernelClient> {
    let mut attempt = 0_usize;
    loop {
        match KernelClient::connect(
            ClientTransport::Local(endpoint.clone()),
            session,
            credential.clone(),
        )
        .await
        {
            Ok(client) => return Ok(client),
            Err(error @ ClientError::Transport(polaris_rpc::TransportError::Tonic(_)))
                if attempt < 300 =>
            {
                let _last_error = error;
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn token(
    authority: &Authority,
    frame: FrameId,
    session: SessionId,
    right: Right,
) -> TestResult<polaris_sdk::contracts::TokenBytes> {
    let resource = ResourcePath::new(&format!("sessions/{session}/rpc"))?;
    let scope = Scope {
        resource: ResourceScope::Exact(resource),
        max_churn_lines: 0,
        ttl: Duration::from_secs(60),
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(&[right]),
    };
    Ok(authority
        .issue_root(frame, &scope, &SystemClock)?
        .to_bytes()
        .clone())
}

#[cfg(any(unix, windows))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authenticated_rpc_p1_event_replay_denial_and_shutdown() -> TestResult<()> {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    #[cfg(unix)]
    let (endpoint, local_endpoint) = endpoint(state.path())?;
    #[cfg(windows)]
    let (endpoint, local_endpoint) = endpoint()?;

    let session = SessionId::from_u128(0xBEEF);
    let frame = FrameId::from_u128(0xCAFE);
    let authority = Authority::new_session()?;
    let kernel = Kernel::start(LocalKernelConfig::new(
        state.path(),
        workspace.path(),
        session,
        frame,
    ))
    .await?;
    let writer = kernel.clone();

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let ready = Ready;
    let serving = tokio::spawn(serve_kernel_api(
        kernel.clone(),
        None,
        NoTerminal,
        RpcAuthenticator::new(authority.public_key(), Resolver(session)),
        ready,
        DaemonListenerConfig {
            endpoint,
            settings: DaemonSettings::default(),
        },
        async move {
            let _received = stop_rx.await;
        },
        move || async move {
            let _report = kernel.shutdown().await?;
            Ok(())
        },
    ));

    let reader = connect_local(
        local_endpoint.clone(),
        session,
        token(&authority, frame, session, Right::Read)?,
    )
    .await?;
    let health = reader.health().await?;
    assert!(
        health.ready,
        "health must report the started kernel as ready"
    );
    let read_only_write = reader.submit_text("hello", false).await;
    assert!(
        matches!(read_only_write, Err(ClientError::Status(status)) if status.code() == tonic::Code::PermissionDenied),
        "a read-only credential cannot record text"
    );
    let receipt = writer.apply_directive("$websearch(off)").await?;
    let mut events = reader.subscribe_checked(receipt.accepted_at).await?;
    let event = tokio::time::timeout(Duration::from_secs(3), events.next_record())
        .await
        .map_err(|_| "kernel event replay timed out")??;
    assert!(
        event.is_some_and(|record| record.seq() == receipt.accepted_at),
        "the real directive event must replay at its accepted sequence"
    );

    let credential = token(&authority, frame, session, Right::ExecuteTool)?;
    let executor = connect_local(local_endpoint, session, credential.clone()).await?;
    let command = Command::new(
        CommandId::new_v7(),
        frame,
        credential,
        CommandBody::ExecuteTool {
            call: Cid::from_bytes([1; 32]),
        },
    );
    let rejected = executor.submit_checked(command).await;
    assert!(
        matches!(rejected, Err(ClientError::Status(status)) if status.code() == tonic::Code::PermissionDenied),
        "the P1 kernel must reject RPC mutations without a shared command authority"
    );

    drop(events);
    drop(executor);
    drop(reader);
    stop_tx
        .send(())
        .map_err(|_| "daemon shutdown receiver disappeared")?;
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .map_err(|_| "daemon shutdown exceeded five seconds")???;
    Ok(())
}

#[cfg(any(unix, windows))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_local_daemon_health_and_graceful_shutdown() -> TestResult<()> {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    #[cfg(unix)]
    let (endpoint, local_endpoint) = endpoint(state.path())?;
    #[cfg(windows)]
    let (endpoint, local_endpoint) = endpoint()?;

    let session = SessionId::from_u128(0xFEED);
    let frame = FrameId::from_u128(0xFACE);
    let authority = Authority::new_session()?;
    let config = DaemonConfig {
        kernel: LocalKernelConfig::new(state.path(), workspace.path(), session, frame),
        endpoint,
        token_root: authority.public_key(),
        settings: DaemonSettings::default(),
    };
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(serve_daemon(config, async move {
        let _received = stop_rx.await;
    }));
    let client = connect_local(
        local_endpoint,
        session,
        token(&authority, frame, session, Right::Read)?,
    )
    .await?;
    assert!(client.health().await?.ready);
    stop_tx
        .send(())
        .map_err(|_| "daemon shutdown receiver disappeared")?;
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .map_err(|_| "daemon graceful shutdown exceeded five seconds")???;
    Ok(())
}

fn session_token(
    authority: &Authority,
    frame: FrameId,
    session: SessionId,
    rights: &[Right],
) -> TestResult<polaris_sdk::contracts::TokenBytes> {
    let scope = Scope {
        resource: ResourceScope::Exact(ResourcePath::new(&format!("sessions/{session}/rpc"))?),
        max_churn_lines: 0,
        ttl: Duration::from_secs(60),
        allow_unsafe: false,
        diff_blake3: None,
        rights: Rights::new(rights),
    };
    Ok(authority
        .issue_root(frame, &scope, &SystemClock)?
        .to_bytes()
        .clone())
}

fn status_code<T>(result: Result<T, ClientError>) -> Option<tonic::Code> {
    match result {
        Err(ClientError::Status(status)) => Some(status.code()),
        _ => None,
    }
}

/// P9: a frontend attaches over the local endpoint with the credential polarisd issued, records
/// typed text and a directive through the daemon's own command authority, and reads the text
/// back from the session's events; weaker, foreign or out-of-scope credentials are refused.
#[cfg(any(unix, windows))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_attached_frontend_records_and_reads_session_text() -> TestResult<()> {
    use polaris_sdk::contracts::Event;
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    #[cfg(unix)]
    let (endpoint, local_endpoint) = endpoint(state.path())?;
    #[cfg(windows)]
    let (endpoint, local_endpoint) = endpoint()?;
    let session = SessionId::from_u128(0xA77A);
    let frame = FrameId::from_u128(0xF00D);
    let (root, credential) =
        polaris_daemon::issue_local_client_token(session, frame, Duration::from_secs(600))?;
    assert!(
        polaris_daemon::credential_hex(&credential)
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    );
    let config = DaemonConfig {
        kernel: LocalKernelConfig::new(state.path(), workspace.path(), session, frame),
        endpoint,
        token_root: root,
        settings: DaemonSettings::default(),
    };
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(serve_daemon(config, async move {
        let _received = stop_rx.await;
    }));
    let client = connect_local(local_endpoint.clone(), session, credential.clone()).await?;
    assert!(client.health().await?.ready);

    let typed = client.submit_text("Explain the parser", false).await?;
    let directive = client.submit_text("$websearch(off)", true).await?;
    assert!(directive.accepted_at > typed.accepted_at);
    let mut events = client.subscribe_checked(typed.accepted_at).await?;
    let mut texts = Vec::new();
    while texts.len() < 2 {
        let record = tokio::time::timeout(Duration::from_secs(5), events.next_record())
            .await
            .map_err(|_| "attached event replay timed out")??
            .ok_or("event stream ended")?;
        let cid = match record.event() {
            Event::UserInputSubmitted { text } => *text,
            Event::DirectiveApplied { directive } => *directive,
            _ => continue,
        };
        texts.push(client.read_text(cid).await?);
    }
    assert_eq!(texts, ["Explain the parser", "$websearch(off)"]);

    // An unreferenced digest is not readable, even with a valid credential.
    assert_eq!(
        status_code(client.read_text(Cid::from_bytes([9; 32])).await),
        Some(tonic::Code::NotFound)
    );
    // Malformed directives are rejected by the kernel, not recorded.
    assert_eq!(
        status_code(client.submit_text("$not-a-directive(", true).await),
        Some(tonic::Code::InvalidArgument)
    );
    // Oversized text never leaves the client.
    assert!(
        client
            .submit_text(&"x".repeat(70 * 1024), false)
            .await
            .is_err()
    );

    // A credential from another issuer is refused.
    let other_issuer = Authority::new_session()?;
    let foreign = session_token(
        &other_issuer,
        frame,
        session,
        &[Right::Read, Right::ProposeDiff],
    )?;
    let foreign_client = connect_local(local_endpoint.clone(), session, foreign).await?;
    assert_eq!(
        status_code(foreign_client.submit_text("hello", false).await),
        Some(tonic::Code::Unauthenticated),
        "a credential from another issuer is refused"
    );
    // A client pinned to another session is refused by the daemon's scope.
    let elsewhere = connect_local(local_endpoint, SessionId::from_u128(0xBAD), credential).await?;
    assert_eq!(
        status_code(elsewhere.submit_text("hello", false).await),
        Some(tonic::Code::PermissionDenied)
    );

    drop(events);
    drop(client);
    stop_tx
        .send(())
        .map_err(|_| "daemon shutdown receiver disappeared")?;
    tokio::time::timeout(Duration::from_secs(5), serving)
        .await
        .map_err(|_| "daemon shutdown exceeded five seconds")???;
    Ok(())
}

#[test]
fn local_client_tokens_have_bounded_lifetimes() {
    let session = SessionId::from_u128(1);
    let frame = FrameId::from_u128(2);
    assert!(polaris_daemon::issue_local_client_token(session, frame, Duration::ZERO).is_err());
    assert!(
        polaris_daemon::issue_local_client_token(session, frame, Duration::from_secs(8 * 86_400))
            .is_err()
    );
}
