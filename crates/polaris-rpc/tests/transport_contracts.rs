use polaris_rpc::{LocalEndpoint, Transport, TransportError};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

#[tokio::test]
async fn public_plaintext_bind_is_refused() {
    let public = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
    let result = polaris_rpc::BoundServer::bind(Transport::Tcp {
        addr: public,
        tls: None,
    })
    .await;
    assert!(matches!(result, Err(TransportError::TlsRequired)));
}

#[test]
fn local_endpoint_rejects_remote_or_ambiguous_names() {
    for name in [
        "",
        "../polaris-test",
        "polaris-test/other",
        r"\\remote\pipe\polaris-test",
        "polaris-test\0other",
    ] {
        assert!(LocalEndpoint::named_pipe(name).is_err());
    }
}

mod fixture {
    use futures::Stream;
    use polaris_proto::pb;
    use polaris_rpc::{
        Readiness, RequestSubject, RpcAuthenticator, RpcLimits, RpcService, TerminalApi,
        TrustedFacts, TrustedFactsResolver,
    };
    use polaris_sdk::{
        contracts::{Command, EventRecord, TokenBytes, Unverified},
        ids::{FrameId, Seq, SessionId},
        registry::{CommandReceipt, EventSource, KernelApi, KernelError},
    };
    use polaris_security_capability::{
        Authority, PublicKeyBytes, ResourcePath, ResourceScope, Right, Rights, Scope, SystemClock,
    };
    use std::{
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio::sync::watch;
    use tonic::Status;
    pub type Failure = Box<dyn std::error::Error + Send + Sync>;

    pub struct PendingEvents;
    impl EventSource for PendingEvents {
        async fn next_event(&mut self) -> Option<EventRecord> {
            std::future::pending().await
        }
    }
    pub struct ReplayEvents(pub std::collections::VecDeque<EventRecord>);
    impl EventSource for ReplayEvents {
        async fn next_event(&mut self) -> Option<EventRecord> {
            self.0.pop_front()
        }
    }
    pub struct ReplayKernel(pub Vec<EventRecord>);
    impl KernelApi for ReplayKernel {
        type Events = ReplayEvents;
        async fn submit(
            &self,
            _command: Command<Unverified>,
        ) -> Result<CommandReceipt, KernelError> {
            Err(KernelError::Closed)
        }
        async fn subscribe(&self, _from: Seq) -> Result<Self::Events, KernelError> {
            Ok(ReplayEvents(self.0.iter().cloned().collect()))
        }
    }
    pub struct TestKernel(pub Arc<AtomicUsize>);
    impl KernelApi for TestKernel {
        type Events = PendingEvents;
        async fn submit(
            &self,
            command: Command<Unverified>,
        ) -> Result<CommandReceipt, KernelError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(CommandReceipt {
                command: command.id(),
                accepted_at: Seq::new(77),
            })
        }
        async fn subscribe(&self, _from: Seq) -> Result<Self::Events, KernelError> {
            Ok(PendingEvents)
        }
    }
    pub struct TestTerminal;
    impl TerminalApi for TestTerminal {
        type Stream = futures::stream::Empty<Result<pb::TerminalServerFrame, Status>>;
        async fn attach(
            &self,
            _inbound: Pin<Box<dyn Stream<Item = Result<pb::TerminalClientFrame, Status>> + Send>>,
            _cancellation: watch::Receiver<bool>,
        ) -> Result<Self::Stream, Status> {
            Err(Status::failed_precondition("test fixture has no terminal"))
        }
    }
    pub struct Resolver;
    impl TrustedFactsResolver for Resolver {
        fn resolve(
            &self,
            _right: Right,
            _subject: RequestSubject<'_>,
        ) -> Result<TrustedFacts, Status> {
            Ok(TrustedFacts {
                resource: ResourcePath::new("rpc/session")
                    .map_err(|_| Status::internal("fixture resource"))?,
                symbol: None,
                churn_lines: 0,
                touches_unsafe: false,
                diff_blake3: None,
            })
        }
    }
    pub struct Ready;
    impl Readiness for Ready {
        fn is_ready(&self) -> bool {
            true
        }
    }
    pub struct RevocableResolver(pub Arc<std::sync::atomic::AtomicBool>);
    impl TrustedFactsResolver for RevocableResolver {
        fn resolve(
            &self,
            right: Right,
            subject: RequestSubject<'_>,
        ) -> Result<TrustedFacts, Status> {
            Resolver.resolve(right, subject)
        }
        fn is_revoked(&self, _token: &polaris_security_capability::Token) -> Result<bool, Status> {
            Ok(self.0.load(Ordering::SeqCst))
        }
    }
    pub type Service = RpcService<TestKernel, TestTerminal, Resolver, Ready>;
    pub fn service(key: PublicKeyBytes, calls: Arc<AtomicUsize>) -> Result<Service, Failure> {
        Ok(RpcService::new(
            TestKernel(calls),
            TestTerminal,
            RpcAuthenticator::new(key, Resolver),
            Ready,
            RpcLimits {
                event_buffer: 4,
                backpressure_timeout: Duration::from_millis(100),
            },
        )?)
    }
    pub fn token(authority: &Authority, rights: &[Right]) -> Result<TokenBytes, Failure> {
        token_at(authority, rights, &SystemClock)
    }
    pub fn token_at(
        authority: &Authority,
        rights: &[Right],
        clock: &impl polaris_security_capability::Clock,
    ) -> Result<TokenBytes, Failure> {
        Ok(authority
            .issue_root(
                FrameId::from_u128(2),
                &Scope {
                    resource: ResourceScope::Exact(ResourcePath::new("rpc/session")?),
                    max_churn_lines: 0,
                    ttl: Duration::from_secs(60),
                    allow_unsafe: false,
                    diff_blake3: None,
                    rights: Rights::new(rights),
                },
                clock,
            )?
            .to_bytes()
            .clone())
    }
    pub const SESSION: SessionId = SessionId::from_u128(1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_stream_revocation_is_enforced_and_failure_is_retained() -> Result<(), fixture::Failure>
{
    use fixture::*;
    use polaris_rpc::{
        BoundAddress, BoundServer, ClientError, ClientTransport, KernelClient, RpcAuthenticator,
        RpcLimits, RpcService, RpcStreamError, TransportLimits,
    };
    use polaris_sdk::ids::Seq;
    use polaris_security_capability::{Authority, Right};
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };
    let authority = Authority::new_session()?;
    let revoked = Arc::new(AtomicBool::new(false));
    let server = BoundServer::bind(Transport::Tcp {
        addr: "127.0.0.1:0".parse()?,
        tls: None,
    })
    .await?;
    let BoundAddress::Tcp(addr) = server.address() else {
        return Err("unexpected listener".into());
    };
    let addr = *addr;
    let service = RpcService::new(
        TestKernel(Arc::new(AtomicUsize::new(0))),
        TestTerminal,
        RpcAuthenticator::new(
            authority.public_key(),
            RevocableResolver(Arc::clone(&revoked)),
        ),
        Ready,
        RpcLimits {
            event_buffer: 4,
            backpressure_timeout: Duration::from_millis(100),
        },
    )?;
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(
        server.serve(service, TransportLimits::default(), async move {
            let _received = stop_rx.await;
        }),
    );
    let client = KernelClient::connect(
        ClientTransport::Tcp { addr, tls: None },
        SESSION,
        token(&authority, &[Right::Read])?,
    )
    .await?;
    let mut events = client.subscribe_checked(Seq::ZERO).await?;
    revoked.store(true, Ordering::SeqCst);
    let error = tokio::time::timeout(Duration::from_secs(3), events.next_record()).await?;
    assert!(
        matches!(error, Err(RpcStreamError::Status(status)) if status.code() == tonic::Code::Unauthenticated)
    );
    assert!(
        matches!(events.terminal_error(), Some(RpcStreamError::Status(status)) if status.code() == tonic::Code::Unauthenticated)
    );
    assert!(events.next_record().await?.is_none());
    assert!(events.terminal_error().is_some());
    assert!(
        matches!(client.health().await, Err(ClientError::Status(status)) if status.code() == tonic::Code::Unauthenticated)
    );
    stop_tx
        .send(())
        .map_err(|_| "shutdown receiver disappeared")?;
    tokio::time::timeout(Duration::from_secs(3), serving).await???;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_includes_zero_cursor_and_isolates_other_session_sequences()
-> Result<(), fixture::Failure> {
    use fixture::*;
    use polaris_rpc::{
        BoundAddress, BoundServer, ClientTransport, KernelClient, RpcAuthenticator, RpcLimits,
        RpcService, TransportLimits,
    };
    use polaris_sdk::{
        contracts::{Event, EventRecord, Timestamp},
        ids::{Seq, SessionId},
    };
    use polaris_security_capability::{Authority, Right};
    let authority = Authority::new_session()?;
    let record = |session, sequence| {
        EventRecord::new(
            Seq::new(sequence),
            Timestamp::UNIX_EPOCH,
            session,
            Event::SessionClosed,
        )
    };
    let records = vec![
        record(SESSION, 0),
        record(SessionId::from_u128(99), 100),
        record(SESSION, 1),
        record(SESSION, 1),
        // Typed user input streams like any other schema-7 record.
        EventRecord::new(
            Seq::new(2),
            Timestamp::UNIX_EPOCH,
            SESSION,
            Event::UserInputSubmitted {
                text: polaris_sdk::ids::Cid::from_bytes([2; 32]),
            },
        ),
    ];
    let server = BoundServer::bind(Transport::Tcp {
        addr: "127.0.0.1:0".parse()?,
        tls: None,
    })
    .await?;
    let BoundAddress::Tcp(addr) = server.address() else {
        return Err("unexpected listener".into());
    };
    let addr = *addr;
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let service = RpcService::new(
        ReplayKernel(records),
        TestTerminal,
        RpcAuthenticator::new(authority.public_key(), Resolver),
        Ready,
        RpcLimits {
            event_buffer: 8,
            backpressure_timeout: std::time::Duration::from_secs(1),
        },
    )?;
    let serving = tokio::spawn(
        server.serve(service, TransportLimits::default(), async move {
            let _received = stop_rx.await;
        }),
    );
    let client = KernelClient::connect(
        ClientTransport::Tcp { addr, tls: None },
        SESSION,
        token(&authority, &[Right::Read])?,
    )
    .await?;
    for from in [0, 1, 2, 3] {
        let mut events = client.subscribe_checked(Seq::new(from)).await?;
        let mut received = Vec::new();
        while let Some(record) =
            tokio::time::timeout(std::time::Duration::from_secs(2), events.next_record()).await??
        {
            received.push(record.seq().get());
        }
        assert_eq!(received, (from..3).collect::<Vec<_>>());
        assert!(events.terminal_error().is_none());
    }
    stop_tx
        .send(())
        .map_err(|_| "shutdown receiver disappeared")?;
    tokio::time::timeout(std::time::Duration::from_secs(3), serving).await???;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tcp_authentication_and_receipt_roundtrip() -> Result<(), fixture::Failure> {
    use fixture::*;
    use polaris_rpc::{
        BoundAddress, BoundServer, ClientError, ClientTransport, KernelClient, TransportLimits,
    };
    use polaris_sdk::{
        contracts::{Command, CommandBody, CommandId},
        ids::{Cid, FrameId},
    };
    use polaris_security_capability::{Authority, Right};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    let authority = Authority::new_session()?;
    let credential = token(&authority, &[Right::Read, Right::ExecuteTool])?;
    let calls = Arc::new(AtomicUsize::new(0));
    let server = BoundServer::bind(Transport::Tcp {
        addr: "127.0.0.1:0".parse()?,
        tls: None,
    })
    .await?;
    let BoundAddress::Tcp(addr) = server.address() else {
        return Err("unexpected listener kind".into());
    };
    let addr = *addr;
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(server.serve(
        service(authority.public_key(), Arc::clone(&calls))?,
        TransportLimits::default(),
        async move {
            let _received = stop_rx.await;
        },
    ));
    let client = KernelClient::connect(
        ClientTransport::Tcp { addr, tls: None },
        SESSION,
        credential.clone(),
    )
    .await?;
    assert!(client.health().await?.ready);
    let command = Command::new(
        CommandId::from_u128(3),
        FrameId::from_u128(2),
        credential,
        CommandBody::ExecuteTool {
            call: Cid::from_bytes([1; 32]),
        },
    );
    let receipt = client.submit_checked(command).await?;
    assert_eq!(receipt.accepted_at.get(), 77);
    assert_eq!(receipt.command, CommandId::from_u128(3));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{addr}"))?
        .connect()
        .await?;
    let error = polaris_proto::pb::polaris_kernel_service_client::PolarisKernelServiceClient::new(
        channel.clone(),
    )
    .health(polaris_proto::pb::HealthRequest {})
    .await;
    assert!(matches!(error,Err(status) if status.code()==tonic::Code::Unauthenticated));
    for header in ["Basic abc", "Bearer ", "Bearer 0", "Bearer zz", "Bearer 00"] {
        let mut request = tonic::Request::new(polaris_proto::pb::HealthRequest {});
        request
            .metadata_mut()
            .insert("authorization", header.parse()?);
        let result =
            polaris_proto::pb::polaris_kernel_service_client::PolarisKernelServiceClient::new(
                channel.clone(),
            )
            .health(request)
            .await;
        assert!(matches!(result, Err(status) if status.code() == tonic::Code::Unauthenticated));
    }
    let mut duplicate = tonic::Request::new(polaris_proto::pb::HealthRequest {});
    duplicate
        .metadata_mut()
        .append("authorization", "Bearer 00".parse()?);
    duplicate
        .metadata_mut()
        .append("authorization", "Bearer 00".parse()?);
    let duplicate_result =
        polaris_proto::pb::polaris_kernel_service_client::PolarisKernelServiceClient::new(channel)
            .health(duplicate)
            .await;
    assert!(matches!(duplicate_result, Err(status) if status.code()==tonic::Code::Unauthenticated));
    struct Past;
    impl polaris_security_capability::Clock for Past {
        fn now(&self) -> polaris_sdk::contracts::Timestamp {
            polaris_sdk::contracts::Timestamp::UNIX_EPOCH
        }
    }
    let expired = KernelClient::connect(
        ClientTransport::Tcp { addr, tls: None },
        SESSION,
        token_at(&authority, &[Right::Read], &Past)?,
    )
    .await?;
    let expired_response = expired.health().await;
    assert!(
        matches!(&expired_response, Err(ClientError::Status(status)) if status.code()==tonic::Code::PermissionDenied),
        "expired capability response: {expired_response:?}"
    );
    let readonly = token(&authority, &[Right::Read])?;
    let readonly_client = KernelClient::connect(
        ClientTransport::Tcp { addr, tls: None },
        SESSION,
        readonly.clone(),
    )
    .await?;
    let denied = readonly_client
        .submit_checked(Command::new(
            CommandId::from_u128(4),
            FrameId::from_u128(2),
            readonly,
            CommandBody::ExecuteTool {
                call: Cid::from_bytes([1; 32]),
            },
        ))
        .await;
    assert!(
        matches!(denied, Err(ClientError::Status(status)) if status.code()==tonic::Code::PermissionDenied)
    );
    let wrong_authority = Authority::new_session()?;
    let bad = KernelClient::connect(
        ClientTransport::Tcp { addr, tls: None },
        SESSION,
        token(&wrong_authority, &[Right::Read])?,
    )
    .await?;
    assert!(
        matches!(bad.health().await,Err(ClientError::Status(status)) if status.code()==tonic::Code::Unauthenticated)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    stop_tx
        .send(())
        .map_err(|_| "server shutdown receiver disappeared")?;
    tokio::time::timeout(std::time::Duration::from_secs(3), serving).await???;
    Ok(())
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_pipe_name_ownership_and_grpc_roundtrip() -> Result<(), fixture::Failure> {
    use fixture::*;
    use polaris_rpc::{BoundServer, ClientTransport, KernelClient, TransportLimits};
    use polaris_sdk::ids::CommandId;
    use polaris_security_capability::{Authority, Right};
    use std::sync::{Arc, atomic::AtomicUsize};
    let name = format!("polaris-rpc-{}", CommandId::new_v7());
    let endpoint = LocalEndpoint::named_pipe(&name)?;
    let authority = Authority::new_session()?;
    let credential = token(&authority, &[Right::Read])?;
    let server = BoundServer::bind(Transport::Local(endpoint.clone())).await?;
    assert!(
        BoundServer::bind(Transport::Local(endpoint.clone()))
            .await
            .is_err()
    );
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(server.serve(
        service(authority.public_key(), Arc::new(AtomicUsize::new(0)))?,
        TransportLimits::default(),
        async move {
            let _received = stop_rx.await;
        },
    ));
    for _ in 0..100 {
        let client = KernelClient::connect(
            ClientTransport::Local(endpoint.clone()),
            SESSION,
            credential.clone(),
        )
        .await?;
        assert!(client.health().await?.ready);
    }
    stop_tx
        .send(())
        .map_err(|_| "shutdown receiver disappeared")?;
    tokio::time::timeout(std::time::Duration::from_secs(3), serving).await???;
    let rebound = BoundServer::bind(Transport::Local(endpoint)).await?;
    drop(rebound);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_closes_idle_streams_within_grace() -> Result<(), fixture::Failure> {
    use fixture::*;
    use polaris_rpc::{BoundAddress, BoundServer, ClientTransport, KernelClient, TransportLimits};
    use polaris_sdk::ids::Seq;
    use polaris_security_capability::{Authority, Right};
    use std::{
        sync::{Arc, atomic::AtomicUsize},
        time::{Duration, Instant},
    };
    let authority = Authority::new_session()?;
    let server = BoundServer::bind(Transport::Tcp {
        addr: "127.0.0.1:0".parse()?,
        tls: None,
    })
    .await?;
    let BoundAddress::Tcp(addr) = server.address() else {
        return Err("unexpected listener".into());
    };
    let addr = *addr;
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(server.serve(
        service(authority.public_key(), Arc::new(AtomicUsize::new(0)))?,
        TransportLimits {
            shutdown_grace: Duration::from_millis(100),
            ..Default::default()
        },
        async move {
            let _received = stop_rx.await;
        },
    ));
    let client = KernelClient::connect(
        ClientTransport::Tcp { addr, tls: None },
        SESSION,
        token(&authority, &[Right::Read])?,
    )
    .await?;
    let mut events = client.subscribe_checked(Seq::new(0)).await?;
    let start = Instant::now();
    stop_tx
        .send(())
        .map_err(|_| "shutdown receiver disappeared")?;
    tokio::time::timeout(Duration::from_secs(3), serving).await???;
    assert!(start.elapsed() < Duration::from_secs(3));
    let ended = tokio::time::timeout(Duration::from_secs(1), events.next_record()).await?;
    assert!(matches!(ended, Ok(None) | Err(_)));
    Ok(())
}

struct Certificates {
    ca: Vec<u8>,
    server: Vec<u8>,
    server_key: String,
    client: Vec<u8>,
    client_key: String,
}
fn certificates() -> Result<Certificates, fixture::Failure> {
    use rcgen::{
        BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
        KeyUsagePurpose,
    };
    let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::CrlSign,
    ];
    let ca_key = KeyPair::generate()?;
    let ca = ca_params.self_signed(&ca_key)?.pem().into_bytes();
    let issuer = Issuer::new(ca_params, ca_key);
    let mut server_params = CertificateParams::new(vec!["localhost".to_owned()])?;
    server_params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ServerAuth);
    server_params
        .key_usages
        .push(KeyUsagePurpose::DigitalSignature);
    let server_key = KeyPair::generate()?;
    let server = server_params
        .signed_by(&server_key, &issuer)?
        .pem()
        .into_bytes();
    let mut client_params = CertificateParams::new(vec!["client.polaris".to_owned()])?;
    client_params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ClientAuth);
    client_params
        .key_usages
        .push(KeyUsagePurpose::DigitalSignature);
    let client_key = KeyPair::generate()?;
    let client = client_params
        .signed_by(&client_key, &issuer)?
        .pem()
        .into_bytes();
    Ok(Certificates {
        ca,
        server,
        server_key: server_key.serialize_pem(),
        client,
        client_key: client_key.serialize_pem(),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mtls_rejects_missing_or_wrong_client_identity() -> Result<(), fixture::Failure> {
    use fixture::*;
    use polaris_rpc::{
        BoundAddress, BoundServer, ClientTransport, KernelClient, TlsClientConfig, TlsServerConfig,
        TransportLimits,
    };
    use polaris_security_capability::{Authority, Right};
    use std::{
        sync::{Arc, atomic::AtomicUsize},
        time::Duration,
    };
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let installed = rustls::crypto::aws_lc_rs::default_provider().install_default();
        if installed.is_err() && rustls::crypto::CryptoProvider::get_default().is_none() {
            return Err("TLS provider unavailable".into());
        }
    }
    let certs = certificates()?;
    let authority = Authority::new_session()?;
    let credential = token(&authority, &[Right::Read])?;
    let server = BoundServer::bind(Transport::Tcp {
        addr: "127.0.0.1:0".parse()?,
        tls: Some(TlsServerConfig::new(
            certs.server.clone(),
            certs.server_key.into(),
            certs.ca.clone(),
        )?),
    })
    .await?;
    let BoundAddress::Tcp(addr) = server.address() else {
        return Err("unexpected listener".into());
    };
    let addr = *addr;
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(server.serve(
        service(authority.public_key(), Arc::new(AtomicUsize::new(0)))?,
        TransportLimits::default(),
        async move {
            let _received = stop_rx.await;
        },
    ));
    let good_tls = TlsClientConfig::new(
        certs.ca.clone(),
        certs.client.clone(),
        certs.client_key.clone().into(),
        "localhost",
    )?;
    let good = KernelClient::connect(
        ClientTransport::Tcp {
            addr,
            tls: Some(good_tls),
        },
        SESSION,
        credential.clone(),
    )
    .await?;
    assert!(good.health().await?.ready);

    // A client trusted by a different CA must not reach the authenticated RPC handler.
    let wrong = certificates()?;
    let wrong_tls = TlsClientConfig::new(
        certs.ca.clone(),
        wrong.client,
        wrong.client_key.into(),
        "localhost",
    )?;
    if let Ok(client) = KernelClient::connect(
        ClientTransport::Tcp {
            addr,
            tls: Some(wrong_tls),
        },
        SESSION,
        credential.clone(),
    )
    .await
    {
        assert!(client.health().await.is_err());
    }
    let wrong_name = TlsClientConfig::new(
        certs.ca.clone(),
        certs.client,
        certs.client_key.into(),
        "wrong.invalid",
    )?;
    assert!(
        KernelClient::connect(
            ClientTransport::Tcp {
                addr,
                tls: Some(wrong_name)
            },
            SESSION,
            credential.clone()
        )
        .await
        .is_err()
    );

    // Trusting the server CA alone must not accidentally enable anonymous TLS clients.
    let endpoint = tonic::transport::Endpoint::from_shared(format!("https://{addr}"))?
        .connect_timeout(Duration::from_secs(2))
        .tls_config(
            tonic::transport::ClientTlsConfig::new()
                .ca_certificate(tonic::transport::Certificate::from_pem(certs.ca))
                .domain_name("localhost"),
        )?;
    if let Ok(channel) = endpoint.connect().await {
        let mut request = tonic::Request::new(polaris_proto::pb::HealthRequest {});
        request.set_timeout(Duration::from_secs(2));
        let encoded: String = credential
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        request
            .metadata_mut()
            .insert("authorization", format!("Bearer {encoded}").parse()?);
        assert!(
            polaris_proto::pb::polaris_kernel_service_client::PolarisKernelServiceClient::new(
                channel
            )
            .health(request)
            .await
            .is_err()
        );
    }
    drop(good);
    stop_tx
        .send(())
        .map_err(|_| "shutdown receiver disappeared")?;
    tokio::time::timeout(Duration::from_secs(3), serving).await???;
    Ok(())
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unix_private_socket_roundtrip_and_cleanup() -> Result<(), fixture::Failure> {
    use fixture::*;
    use polaris_rpc::{BoundServer, ClientTransport, KernelClient, TransportLimits};
    use polaris_security_capability::{Authority, Right};
    use std::{
        os::unix::fs::PermissionsExt,
        sync::{Arc, atomic::AtomicUsize},
    };
    let directory = tempfile::Builder::new()
        .prefix("polaris-rpc-")
        .tempdir_in("/tmp")?;
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))?;
    let path = directory.path().join("rpc.sock");
    let endpoint = LocalEndpoint::unix(path.clone())?;
    let authority = Authority::new_session()?;
    let server = BoundServer::bind(Transport::Local(endpoint.clone())).await?;
    assert!(
        BoundServer::bind(Transport::Local(endpoint.clone()))
            .await
            .is_err()
    );
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(server.serve(
        service(authority.public_key(), Arc::new(AtomicUsize::new(0)))?,
        TransportLimits::default(),
        async move {
            let _received = stop_rx.await;
        },
    ));
    let client = KernelClient::connect(
        ClientTransport::Local(endpoint),
        SESSION,
        token(&authority, &[Right::Read])?,
    )
    .await?;
    assert!(client.health().await?.ready);
    stop_tx
        .send(())
        .map_err(|_| "shutdown receiver disappeared")?;
    tokio::time::timeout(std::time::Duration::from_secs(3), serving).await???;
    assert!(!path.exists());
    Ok(())
}
