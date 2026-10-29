// Unit tests of crates/polaris-system-one/crates/client/src/lib.rs (`tests`); compiled into that module via #[path].
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::{ClientConfig, S1Client, WorkerCredential, validate_response};
use polaris_sdk::registry::DecisionError;
use polaris_sdk::system_one::{
    Answer, Decision, DecisionInput, Label, LabelScore, Probability, Question, QuestionAnswer,
    QuestionId, QuestionKind,
};

fn client(max_input_bytes: usize) -> Result<S1Client, Box<dyn std::error::Error>> {
    let channel = Endpoint::from_static("http://[::]:50051").connect_lazy();
    let config = ClientConfig::new(Duration::from_secs(2), max_input_bytes)?;
    Ok(S1Client::new(
        channel,
        config,
        WorkerCredential::generate()?,
    )?)
}

use tonic::transport::Endpoint;

#[derive(Clone)]
struct HealthService {
    ready: bool,
    model_loaded: bool,
    version: Option<polaris_proto::pb::ProtocolVersion>,
    handshakes: Arc<AtomicUsize>,
    health_calls: Arc<AtomicUsize>,
}

#[tonic::async_trait]
impl polaris_proto::pb::worker_service_server::WorkerService for HealthService {
    async fn handshake(
        &self,
        _request: tonic::Request<polaris_proto::pb::WorkerHandshakeRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::WorkerHandshakeResponse>, tonic::Status> {
        self.handshakes.fetch_add(1, Ordering::Relaxed);
        Ok(tonic::Response::new(
            polaris_proto::pb::WorkerHandshakeResponse {
                protocol_version: Some(polaris_proto::PROTOCOL_VERSION.into()),
                kind: i32::from(polaris_proto::pb::WorkerKind::S1),
                worker_version: "test".to_owned(),
            },
        ))
    }

    async fn decide(
        &self,
        _request: tonic::Request<polaris_proto::pb::DecideRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::DecideResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("test health service"))
    }

    async fn detect_pii(
        &self,
        _request: tonic::Request<polaris_proto::pb::DetectPiiRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::DetectPiiResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("test health service"))
    }

    async fn embed(
        &self,
        _request: tonic::Request<polaris_proto::pb::EmbedRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::EmbedResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("test health service"))
    }

    async fn ingest(
        &self,
        _request: tonic::Request<polaris_proto::pb::IngestRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::IngestResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("test health service"))
    }

    async fn evict_models(
        &self,
        _request: tonic::Request<polaris_proto::pb::EvictModelsRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::EvictModelsResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("test health service"))
    }

    async fn health(
        &self,
        _request: tonic::Request<polaris_proto::pb::HealthRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::HealthResponse>, tonic::Status> {
        self.health_calls.fetch_add(1, Ordering::Relaxed);
        Ok(tonic::Response::new(polaris_proto::pb::HealthResponse {
            protocol_version: self.version,
            build_version: "test".to_owned(),
            ready: self.ready,
            loaded_model_identity_blake3: if self.model_loaded {
                vec![4; 32]
            } else {
                Vec::new()
            },
            loaded_model_generation: if self.model_loaded { 1 } else { 0 },
            worker_incarnation: vec![7; 16],
            loaded_pii_model_identity_blake3: Vec::new(),
            loaded_pii_model_generation: 0,
        }))
    }

    async fn shutdown(
        &self,
        _request: tonic::Request<polaris_proto::pb::ShutdownRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::ShutdownResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("test health service"))
    }
}

async fn health_client(
    ready: bool,
    model_loaded: bool,
    version: Option<polaris_proto::pb::ProtocolVersion>,
) -> Result<
    (
        S1Client,
        tokio::sync::oneshot::Sender<()>,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
    ),
    Box<dyn std::error::Error>,
> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let credential = WorkerCredential::generate()?;
    let authenticator = polaris_proto::worker_auth::WorkerAuthenticator::new(&credential);
    let handshakes = Arc::new(AtomicUsize::new(0));
    let health_calls = Arc::new(AtomicUsize::new(0));
    let service = HealthService {
        ready,
        model_loaded,
        version,
        handshakes: Arc::clone(&handshakes),
        health_calls: Arc::clone(&health_calls),
    };
    let service = polaris_proto::pb::worker_service_server::WorkerServiceServer::with_interceptor(
        service,
        authenticator,
    );
    let (shutdown_sender, shutdown_receiver) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async move {
                let _ = shutdown_receiver.await;
            })
            .await
    });
    let channel = Endpoint::from_shared(format!("http://{address}"))?
        .connect()
        .await?;
    let client = S1Client::new(
        channel,
        ClientConfig::new(Duration::from_secs(2), 64)?,
        credential,
    )?;
    Ok((client, shutdown_sender, handshakes, health_calls, server))
}

#[tokio::test]
async fn request_validation_counts_context_separator_and_utf8_bytes()
-> Result<(), Box<dyn std::error::Error>> {
    let client = client(9)?;
    let question = Question {
        id: QuestionId::new("kind")?,
        kind: QuestionKind::Choice {
            options: vec![Label::new("bug")?],
        },
    };
    let input = DecisionInput {
        text: "éé".to_owned(),
        context: Some("abcd".to_owned()),
    };
    assert!(matches!(
        client.validate_request(&[question], &input),
        Err(DecisionError::InvalidSchema(_))
    ));
    Ok(())
}

#[tokio::test]
async fn request_validation_rejects_empty_schema_without_transport()
-> Result<(), Box<dyn std::error::Error>> {
    let client = client(64)?;
    let input = DecisionInput {
        text: "document".to_owned(),
        context: None,
    };
    assert!(matches!(
        client.validate_request(&[], &input),
        Err(DecisionError::InvalidSchema(_))
    ));
    Ok(())
}

#[test]
fn response_validation_rejects_answers_for_another_question()
-> Result<(), Box<dyn std::error::Error>> {
    let question = Question {
        id: QuestionId::new("kind")?,
        kind: QuestionKind::Choice {
            options: vec![Label::new("bug")?],
        },
    };
    let decision = Decision {
        answers: vec![QuestionAnswer {
            question: QuestionId::new("other")?,
            answer: Answer::Choice {
                selected: Label::new("bug")?,
                probs: vec![LabelScore {
                    label: Label::new("bug")?,
                    probability: Probability::new(1.0)?,
                }],
            },
        }],
        latency_us: 1,
    };
    assert!(validate_response(&[question], &decision).is_err());
    Ok(())
}

#[test]
fn response_validation_rejects_unselected_or_unsorted_choice_scores()
-> Result<(), Box<dyn std::error::Error>> {
    let question = Question {
        id: QuestionId::new("kind")?,
        kind: QuestionKind::Choice {
            options: vec![Label::new("bug")?, Label::new("feature")?],
        },
    };
    let decision = Decision {
        answers: vec![QuestionAnswer {
            question: QuestionId::new("kind")?,
            answer: Answer::Choice {
                selected: Label::new("feature")?,
                probs: vec![
                    LabelScore {
                        label: Label::new("bug")?,
                        probability: Probability::new(0.9)?,
                    },
                    LabelScore {
                        label: Label::new("feature")?,
                        probability: Probability::new(0.1)?,
                    },
                ],
            },
        }],
        latency_us: 1,
    };
    assert!(validate_response(&[question], &decision).is_err());
    Ok(())
}

#[tokio::test]
async fn schema_bound_counts_derived_ordinal_labels() -> Result<(), Box<dyn std::error::Error>> {
    let client = client(64)?;
    let schema = (0..24)
        .map(|index| -> Result<Question, Box<dyn std::error::Error>> {
            Ok(Question {
                id: QuestionId::new(&format!("ordinal_{index}"))?,
                kind: QuestionKind::Score {
                    min: i32::MIN,
                    max: i32::MIN + 256,
                },
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let input = DecisionInput {
        text: "document".to_owned(),
        context: None,
    };
    assert!(matches!(
        client.validate_request(&schema, &input),
        Err(DecisionError::InvalidSchema(_))
    ));
    Ok(())
}

#[test]
fn client_input_bound_matches_worker_cap() {
    assert!(ClientConfig::new(Duration::from_secs(1), 262_145).is_err());
    assert!(ClientConfig::new(Duration::from_secs(121), 1).is_err());
}

#[test]
fn pipe_retry_classifies_pipe_busy_without_retrying_acl_denial() {
    let pipe_busy = std::io::Error::from_raw_os_error(231);
    let access_denied = std::io::Error::from_raw_os_error(5);
    assert!(super::retryable_pipe_open_error(&pipe_busy));
    assert!(!super::retryable_pipe_open_error(&access_denied));
}

#[test]
fn health_response_validation_distinguishes_admission_from_model_readiness() {
    let response = polaris_proto::pb::HealthResponse {
        protocol_version: Some(polaris_proto::PROTOCOL_VERSION.into()),
        build_version: "worker".to_owned(),
        ready: false,
        loaded_model_identity_blake3: Vec::new(),
        loaded_model_generation: 0,
        worker_incarnation: vec![0; 16],
        loaded_pii_model_identity_blake3: Vec::new(),
        loaded_pii_model_generation: 0,
    };
    assert_eq!(super::validate_health_response(response), Ok(false));
}

#[test]
fn health_response_validation_rejects_missing_and_incompatible_versions() {
    let missing = polaris_proto::pb::HealthResponse {
        protocol_version: None,
        build_version: String::new(),
        ready: true,
        loaded_model_identity_blake3: Vec::new(),
        loaded_model_generation: 0,
        worker_incarnation: vec![0; 16],
        loaded_pii_model_identity_blake3: Vec::new(),
        loaded_pii_model_generation: 0,
    };
    assert!(super::validate_health_response(missing).is_err());

    let incompatible = polaris_proto::pb::HealthResponse {
        protocol_version: Some(polaris_proto::pb::ProtocolVersion { major: 2, minor: 0 }),
        build_version: String::new(),
        ready: true,
        loaded_model_identity_blake3: Vec::new(),
        loaded_model_generation: 0,
        worker_incarnation: vec![0; 16],
        loaded_pii_model_identity_blake3: Vec::new(),
        loaded_pii_model_generation: 0,
    };
    assert!(super::validate_health_response(incompatible).is_err());
}

#[tokio::test]
async fn readiness_does_not_report_ready_when_authenticated_rpc_cannot_connect()
-> Result<(), Box<dyn std::error::Error>> {
    let client = client(64)?;
    assert!(client.is_ready().await.is_err());
    Ok(())
}

#[tokio::test]
async fn readiness_authenticates_handshake_and_health_and_reports_admission_only()
-> Result<(), Box<dyn std::error::Error>> {
    for expected_ready in [true, false] {
        let (client, shutdown, handshakes, health_calls, server) = health_client(
            expected_ready,
            false,
            Some(polaris_proto::PROTOCOL_VERSION.into()),
        )
        .await?;
        assert_eq!(client.is_ready().await?, expected_ready);
        assert_eq!(handshakes.load(Ordering::Relaxed), 1);
        assert_eq!(health_calls.load(Ordering::Relaxed), 1);
        let _ = shutdown.send(());
        server.await??;
    }
    Ok(())
}

#[tokio::test]
async fn model_readiness_requires_authenticated_loaded_identity()
-> Result<(), Box<dyn std::error::Error>> {
    for (ready, loaded, expected) in [
        (true, false, false),
        (true, true, true),
        (false, true, false),
    ] {
        let (client, shutdown, _, _, server) =
            health_client(ready, loaded, Some(polaris_proto::PROTOCOL_VERSION.into())).await?;
        assert_eq!(client.is_model_ready().await?, expected);
        let _ = shutdown.send(());
        server.await??;
    }
    Ok(())
}

#[tokio::test]
async fn readiness_rejects_invalid_health_protocol_versions()
-> Result<(), Box<dyn std::error::Error>> {
    let invalid_versions = [
        None,
        Some(polaris_proto::pb::ProtocolVersion { major: 2, minor: 0 }),
        Some(polaris_proto::pb::ProtocolVersion {
            major: u32::MAX,
            minor: 0,
        }),
    ];
    for version in invalid_versions {
        let (client, shutdown, _, _, server) = health_client(true, false, version).await?;
        assert!(client.is_ready().await.is_err());
        let _ = shutdown.send(());
        server.await??;
    }
    Ok(())
}

#[derive(Clone)]
struct EvictionService {
    evicted: u32,
    delay: Duration,
    failure: Option<(tonic::Code, String)>,
    handshakes: Arc<AtomicUsize>,
    eviction_calls: Arc<AtomicUsize>,
}

#[tonic::async_trait]
impl polaris_proto::pb::worker_service_server::WorkerService for EvictionService {
    async fn handshake(
        &self,
        _request: tonic::Request<polaris_proto::pb::WorkerHandshakeRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::WorkerHandshakeResponse>, tonic::Status> {
        self.handshakes.fetch_add(1, Ordering::Relaxed);
        Ok(tonic::Response::new(
            polaris_proto::pb::WorkerHandshakeResponse {
                protocol_version: Some(polaris_proto::PROTOCOL_VERSION.into()),
                kind: i32::from(polaris_proto::pb::WorkerKind::S1),
                worker_version: "test".to_owned(),
            },
        ))
    }

    async fn decide(
        &self,
        _request: tonic::Request<polaris_proto::pb::DecideRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::DecideResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("test eviction service"))
    }

    async fn detect_pii(
        &self,
        _request: tonic::Request<polaris_proto::pb::DetectPiiRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::DetectPiiResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("test eviction service"))
    }

    async fn embed(
        &self,
        _request: tonic::Request<polaris_proto::pb::EmbedRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::EmbedResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("test eviction service"))
    }

    async fn ingest(
        &self,
        _request: tonic::Request<polaris_proto::pb::IngestRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::IngestResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("test eviction service"))
    }

    async fn evict_models(
        &self,
        _request: tonic::Request<polaris_proto::pb::EvictModelsRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::EvictModelsResponse>, tonic::Status> {
        self.eviction_calls.fetch_add(1, Ordering::Relaxed);
        tokio::time::sleep(self.delay).await;
        if let Some((code, message)) = &self.failure {
            return Err(tonic::Status::new(*code, message.clone()));
        }
        Ok(tonic::Response::new(
            polaris_proto::pb::EvictModelsResponse {
                evicted: self.evicted,
                ..Default::default()
            },
        ))
    }

    async fn health(
        &self,
        _request: tonic::Request<polaris_proto::pb::HealthRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::HealthResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("test eviction service"))
    }

    async fn shutdown(
        &self,
        _request: tonic::Request<polaris_proto::pb::ShutdownRequest>,
    ) -> Result<tonic::Response<polaris_proto::pb::ShutdownResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("test eviction service"))
    }
}

async fn eviction_client(
    timeout: Duration,
    evicted: u32,
    delay: Duration,
    failure: Option<(tonic::Code, String)>,
) -> Result<
    (
        S1Client,
        tokio::sync::oneshot::Sender<()>,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
    ),
    Box<dyn std::error::Error>,
> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let credential = WorkerCredential::generate()?;
    let authenticator = polaris_proto::worker_auth::WorkerAuthenticator::new(&credential);
    let handshakes = Arc::new(AtomicUsize::new(0));
    let eviction_calls = Arc::new(AtomicUsize::new(0));
    let service = EvictionService {
        evicted,
        delay,
        failure,
        handshakes: Arc::clone(&handshakes),
        eviction_calls: Arc::clone(&eviction_calls),
    };
    let service = polaris_proto::pb::worker_service_server::WorkerServiceServer::with_interceptor(
        service,
        authenticator,
    );
    let (shutdown_sender, shutdown_receiver) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async move {
                let _ = shutdown_receiver.await;
            })
            .await
    });
    let channel = Endpoint::from_shared(format!("http://{address}"))?
        .connect()
        .await?;
    let client = S1Client::new(channel, ClientConfig::new(timeout, 64)?, credential)?;
    Ok((client, shutdown_sender, handshakes, eviction_calls, server))
}

#[tokio::test]
async fn eviction_authenticates_handshake_and_rpc_and_returns_drop_acknowledgement()
-> Result<(), Box<dyn std::error::Error>> {
    let (client, shutdown, handshakes, eviction_calls, server) =
        eviction_client(Duration::from_secs(2), 1, Duration::ZERO, None).await?;
    assert_eq!(client.evict_models().await?, 1);
    assert_eq!(handshakes.load(Ordering::Relaxed), 1);
    assert_eq!(eviction_calls.load(Ordering::Relaxed), 1);
    let _ = shutdown.send(());
    server.await??;
    Ok(())
}

#[tokio::test]
async fn eviction_rejects_acknowledgement_above_two_native_models()
-> Result<(), Box<dyn std::error::Error>> {
    let (client, shutdown, _, _, server) =
        eviction_client(Duration::from_secs(2), 3, Duration::ZERO, None).await?;
    assert!(matches!(
        client.evict_models().await,
        Err(DecisionError::Model(_))
    ));
    let _ = shutdown.send(());
    server.await??;
    Ok(())
}

#[tokio::test]
async fn eviction_obeys_configured_deadline() -> Result<(), Box<dyn std::error::Error>> {
    let (client, shutdown, _, _, server) = eviction_client(
        Duration::from_millis(20),
        1,
        Duration::from_millis(100),
        None,
    )
    .await?;
    assert!(matches!(
        client.evict_models().await,
        Err(DecisionError::Timeout)
    ));
    let _ = shutdown.send(());
    server.await??;
    Ok(())
}

#[tokio::test]
async fn eviction_maps_worker_status_errors() -> Result<(), Box<dyn std::error::Error>> {
    for (code, expected_unavailable) in [
        (tonic::Code::Unavailable, true),
        (tonic::Code::PermissionDenied, false),
    ] {
        let (client, shutdown, _, _, server) = eviction_client(
            Duration::from_secs(2),
            0,
            Duration::ZERO,
            Some((code, "test failure".to_owned())),
        )
        .await?;
        let result = client.evict_models().await;
        if expected_unavailable {
            assert!(matches!(result, Err(DecisionError::Unavailable(_))));
        } else {
            assert!(matches!(result, Err(DecisionError::Model(_))));
        }
        let _ = shutdown.send(());
        server.await??;
    }
    Ok(())
}

#[tokio::test]
async fn eviction_rejects_missing_authentication() -> Result<(), Box<dyn std::error::Error>> {
    let (mut client, shutdown, handshakes, eviction_calls, server) =
        eviction_client(Duration::from_secs(2), 1, Duration::ZERO, None).await?;
    client.credential = std::sync::Arc::new(WorkerCredential::generate()?);
    if client.handshake.set(()).is_err() {
        return Err("test could not bypass the handshake to exercise EvictModels auth".into());
    }
    assert!(matches!(
        client.evict_models().await,
        Err(DecisionError::Model(_))
    ));
    assert_eq!(handshakes.load(Ordering::Relaxed), 0);
    assert_eq!(eviction_calls.load(Ordering::Relaxed), 0);
    let _ = shutdown.send(());
    server.await??;
    Ok(())
}
