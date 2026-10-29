use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use polaris_proto::worker_auth::{WorkerAuthenticator, WorkerCredential};
use polaris_proto::{PROTOCOL_VERSION, pb};
use polaris_sdk::registry::DecisionEngine;
use polaris_sdk::system_one::{DecisionInput, Label, Question, QuestionId, QuestionKind};
use polaris_system_one_client::{ClientConfig, S1Client};
use tonic::transport::Endpoint;

#[derive(Clone)]
struct BatchProbe {
    sizes: Arc<std::sync::Mutex<Vec<usize>>>,
    generation: Arc<AtomicUsize>,
    rotate_after_first_decide: bool,
    started: Option<Arc<tokio::sync::Notify>>,
    resume: Option<Arc<tokio::sync::Notify>>,
}

#[tonic::async_trait]
impl pb::worker_service_server::WorkerService for BatchProbe {
    async fn handshake(
        &self,
        _: tonic::Request<pb::WorkerHandshakeRequest>,
    ) -> Result<tonic::Response<pb::WorkerHandshakeResponse>, tonic::Status> {
        Ok(tonic::Response::new(pb::WorkerHandshakeResponse {
            protocol_version: Some(PROTOCOL_VERSION.into()),
            kind: i32::from(pb::WorkerKind::S1),
            worker_version: "test".into(),
        }))
    }
    async fn decide(
        &self,
        request: tonic::Request<pb::DecideRequest>,
    ) -> Result<tonic::Response<pb::DecideResponse>, tonic::Status> {
        let items = request.into_inner().items;
        let call_number = {
            let mut sizes = self
                .sizes
                .lock()
                .map_err(|_| tonic::Status::internal("probe mutex poisoned"))?;
            sizes.push(items.len());
            sizes.len()
        };
        if self.rotate_after_first_decide && call_number == 1 {
            self.generation.store(2, Ordering::SeqCst);
        }
        if let Some(started) = &self.started {
            started.notify_one();
        }
        if let Some(resume) = &self.resume {
            resume.notified().await;
        }
        let decisions = items
            .into_iter()
            .map(|item| pb::Decision {
                answers: item
                    .questions
                    .into_iter()
                    .map(|question| pb::QuestionAnswer {
                        question: question.id,
                        answer: Some(pb::Answer {
                            kind: Some(pb::answer::Kind::Choice(pb::answer::Choice {
                                selected: "bug".into(),
                                probs: vec![pb::LabelScore {
                                    label: "bug".into(),
                                    probability: 1.0,
                                }],
                            })),
                        }),
                    })
                    .collect(),
                latency_us: 1,
            })
            .collect();
        Ok(tonic::Response::new(pb::DecideResponse { decisions }))
    }
    async fn detect_pii(
        &self,
        request: tonic::Request<pb::DetectPiiRequest>,
    ) -> Result<tonic::Response<pb::DetectPiiResponse>, tonic::Status> {
        let raw_text = request.into_inner().raw_text;
        if let Some(started) = &self.started {
            started.notify_one();
        }
        if let Some(resume) = &self.resume {
            resume.notified().await;
        }
        let spans = if raw_text == "Alice" {
            vec![pb::PiiEntitySpan {
                start: 0,
                end: 5,
                confidence: 0.95,
            }]
        } else if raw_text == "é" {
            vec![pb::PiiEntitySpan {
                start: 1,
                end: 2,
                confidence: 0.95,
            }]
        } else {
            Vec::new()
        };
        Ok(tonic::Response::new(pb::DetectPiiResponse { spans }))
    }
    async fn embed(
        &self,
        _: tonic::Request<pb::EmbedRequest>,
    ) -> Result<tonic::Response<pb::EmbedResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("unused"))
    }
    async fn ingest(
        &self,
        _: tonic::Request<pb::IngestRequest>,
    ) -> Result<tonic::Response<pb::IngestResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("unused"))
    }
    async fn evict_models(
        &self,
        _: tonic::Request<pb::EvictModelsRequest>,
    ) -> Result<tonic::Response<pb::EvictModelsResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("unused"))
    }
    async fn health(
        &self,
        _: tonic::Request<pb::HealthRequest>,
    ) -> Result<tonic::Response<pb::HealthResponse>, tonic::Status> {
        let generation = self.generation.load(Ordering::SeqCst);
        let generation_byte = match u8::try_from(generation) {
            Ok(value) => value,
            Err(_) => return Err(tonic::Status::internal("test generation out of range")),
        };
        let generation = u64::try_from(generation)
            .map_err(|_| tonic::Status::internal("test generation out of range"))?;
        Ok(tonic::Response::new(pb::HealthResponse {
            protocol_version: Some(PROTOCOL_VERSION.into()),
            build_version: "test".into(),
            ready: true,
            loaded_model_identity_blake3: if generation > 0 {
                vec![generation_byte; 32]
            } else {
                Vec::new()
            },
            loaded_model_generation: generation,
            worker_incarnation: vec![1; 16],
            loaded_pii_model_identity_blake3: Vec::new(),
            loaded_pii_model_generation: 0,
        }))
    }
    async fn shutdown(
        &self,
        _: tonic::Request<pb::ShutdownRequest>,
    ) -> Result<tonic::Response<pb::ShutdownResponse>, tonic::Status> {
        Err(tonic::Status::unimplemented("unused"))
    }
}

#[tokio::test]
async fn concurrent_decisions_share_one_worker_batch() -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let credential = WorkerCredential::generate()?;
    let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let service = pb::worker_service_server::WorkerServiceServer::with_interceptor(
        BatchProbe {
            sizes: Arc::clone(&sizes),
            generation: Arc::new(AtomicUsize::new(1)),
            rotate_after_first_decide: false,
            started: None,
            resume: None,
        },
        WorkerAuthenticator::new(&credential),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async {
                let _ = stopped.await;
            })
            .await
    });
    let channel = Endpoint::from_shared(format!("http://{address}"))?
        .connect()
        .await?;
    let client = Arc::new(S1Client::new(
        channel,
        ClientConfig::new(Duration::from_secs(2), 64)?,
        credential,
    )?);
    let schema = vec![Question {
        id: QuestionId::new("kind")?,
        kind: QuestionKind::Choice {
            options: vec![Label::new("bug")?],
        },
    }];
    let tasks = (0..20)
        .map(|index| {
            let client = Arc::clone(&client);
            let schema = schema.clone();
            tokio::spawn(async move {
                client
                    .decide(
                        &schema,
                        &DecisionInput {
                            text: format!("same-{index}"),
                            context: None,
                        },
                    )
                    .await
            })
        })
        .collect::<Vec<_>>();
    for task in tasks {
        task.await??;
    }
    client
        .decide(
            &schema,
            &DecisionInput {
                text: "same-0".into(),
                context: None,
            },
        )
        .await?;
    let spans = client.detect_pii_spans("Alice").await?;
    assert_eq!(
        spans.first().map(|span| (span.start(), span.end())),
        Some((0, 5))
    );
    assert!(client.detect_pii_spans("é").await.is_err());
    client
        .decide(
            &schema,
            &DecisionInput {
                text: "same-0".into(),
                context: Some("changed context".into()),
            },
        )
        .await?;
    let observed = sizes.lock().map_err(|_| "probe mutex poisoned")?.clone();
    let _ = stop.send(());
    server.await??;
    assert_eq!(
        observed.iter().sum::<usize>(),
        21,
        "cache hit skips one RPC and changed context misses; observed {observed:?}"
    );
    assert!(
        observed.len() < 20,
        "concurrent calls should coalesce; observed {observed:?}"
    );
    assert!(observed.iter().all(|size| *size <= 8));
    assert!(
        observed.contains(&8),
        "worker maximum batch should be exercised; observed {observed:?}"
    );
    Ok(())
}

#[tokio::test]
async fn identical_concurrent_requests_infer_once() -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let credential = WorkerCredential::generate()?;
    let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let service = pb::worker_service_server::WorkerServiceServer::with_interceptor(
        BatchProbe {
            sizes: Arc::clone(&sizes),
            generation: Arc::new(AtomicUsize::new(1)),
            rotate_after_first_decide: false,
            started: None,
            resume: None,
        },
        WorkerAuthenticator::new(&credential),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async {
                let _ = stopped.await;
            })
            .await
    });
    let channel = Endpoint::from_shared(format!("http://{address}"))?
        .connect()
        .await?;
    let client = Arc::new(S1Client::new(
        channel,
        ClientConfig::new(Duration::from_secs(2), 64)?,
        credential,
    )?);
    let schema = vec![Question {
        id: QuestionId::new("kind")?,
        kind: QuestionKind::Choice {
            options: vec![Label::new("bug")?],
        },
    }];
    let input = DecisionInput {
        text: "same request singleflight".to_owned(),
        context: None,
    };
    let first_client = Arc::clone(&client);
    let first_schema = schema.clone();
    let first_input = input.clone();
    let first = tokio::spawn(async move { first_client.decide(&first_schema, &first_input).await });
    tokio::time::sleep(Duration::from_millis(1)).await;
    let second = client.decide(&schema, &input).await?;
    let first_result = first.await??;
    assert_eq!(first_result, second);
    let observed = sizes.lock().map_err(|_| "probe mutex poisoned")?.clone();
    let _ = stop.send(());
    server.await??;
    assert_eq!(
        observed.iter().sum::<usize>(),
        1,
        "duplicate inference observed: {observed:?}"
    );
    Ok(())
}

#[tokio::test]
async fn canceled_singleflight_owner_wakes_waiter_to_recompute()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let credential = WorkerCredential::generate()?;
    let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let started = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let service = pb::worker_service_server::WorkerServiceServer::with_interceptor(
        BatchProbe {
            sizes: Arc::clone(&sizes),
            generation: Arc::new(AtomicUsize::new(1)),
            rotate_after_first_decide: false,
            started: Some(Arc::clone(&started)),
            resume: Some(Arc::clone(&resume)),
        },
        WorkerAuthenticator::new(&credential),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async {
                let _ = stopped.await;
            })
            .await
    });
    let channel = Endpoint::from_shared(format!("http://{address}"))?
        .connect()
        .await?;
    let client = Arc::new(S1Client::new(
        channel,
        ClientConfig::new(Duration::from_secs(2), 64)?,
        credential,
    )?);
    let schema = vec![Question {
        id: QuestionId::new("kind")?,
        kind: QuestionKind::Choice {
            options: vec![Label::new("bug")?],
        },
    }];
    let input = DecisionInput {
        text: "owner cancellation retry".to_owned(),
        context: None,
    };
    let first_client = Arc::clone(&client);
    let first_schema = schema.clone();
    let first_input = input.clone();
    let first = tokio::spawn(async move { first_client.decide(&first_schema, &first_input).await });
    tokio::time::timeout(Duration::from_secs(1), started.notified()).await?;
    let waiting_client = Arc::clone(&client);
    let waiting_schema = schema.clone();
    let waiting_input = input.clone();
    let waiter =
        tokio::spawn(async move { waiting_client.decide(&waiting_schema, &waiting_input).await });
    tokio::time::sleep(Duration::from_millis(5)).await;
    first.abort();
    resume.notify_one();
    tokio::time::timeout(Duration::from_secs(1), started.notified()).await?;
    resume.notify_one();
    let result = waiter.await??;
    let observed = sizes.lock().map_err(|_| "probe mutex poisoned")?.clone();
    let _ = stop.send(());
    server.await??;
    assert_eq!(result.answers.len(), 1);
    assert_eq!(
        observed.iter().sum::<usize>(),
        2,
        "waiter must recompute after owner cancellation: {observed:?}"
    );
    Ok(())
}

#[tokio::test]
async fn pii_span_rpc_obeys_client_deadline() -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let credential = WorkerCredential::generate()?;
    let started = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let service = pb::worker_service_server::WorkerServiceServer::with_interceptor(
        BatchProbe {
            sizes: Arc::new(std::sync::Mutex::new(Vec::new())),
            generation: Arc::new(AtomicUsize::new(1)),
            rotate_after_first_decide: false,
            started: Some(Arc::clone(&started)),
            resume: Some(Arc::clone(&resume)),
        },
        WorkerAuthenticator::new(&credential),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async {
                let _ = stopped.await;
            })
            .await
    });
    let channel = Endpoint::from_shared(format!("http://{address}"))?
        .connect()
        .await?;
    let client = S1Client::new(
        channel,
        ClientConfig::new(Duration::from_millis(20), 64)?,
        credential,
    )?;
    let result = client.detect_pii_spans("Alice").await;
    assert!(matches!(
        result,
        Err(polaris_sdk::registry::DecisionError::Timeout)
    ));
    tokio::time::timeout(Duration::from_secs(1), started.notified()).await?;
    resume.notify_one();
    let _ = stop.send(());
    server.await??;
    Ok(())
}

#[tokio::test]
async fn worker_without_loaded_identity_never_serves_cached_answers()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let credential = WorkerCredential::generate()?;
    let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let service = pb::worker_service_server::WorkerServiceServer::with_interceptor(
        BatchProbe {
            sizes: Arc::clone(&sizes),
            generation: Arc::new(AtomicUsize::new(0)),
            rotate_after_first_decide: false,
            started: None,
            resume: None,
        },
        WorkerAuthenticator::new(&credential),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async {
                let _ = stopped.await;
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
    let schema = vec![Question {
        id: QuestionId::new("kind")?,
        kind: QuestionKind::Choice {
            options: vec![Label::new("bug")?],
        },
    }];
    let input = DecisionInput {
        text: "same masked text".into(),
        context: Some("same masked context".into()),
    };
    client.decide(&schema, &input).await?;
    client.decide(&schema, &input).await?;
    let observed = sizes.lock().map_err(|_| "probe mutex poisoned")?.clone();
    let _ = stop.send(());
    server.await??;
    assert_eq!(observed, vec![1, 1]);
    Ok(())
}

#[tokio::test]
async fn result_from_prior_model_generation_is_not_cached_under_reloaded_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let credential = WorkerCredential::generate()?;
    let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let service = pb::worker_service_server::WorkerServiceServer::with_interceptor(
        BatchProbe {
            sizes: Arc::clone(&sizes),
            generation: Arc::new(AtomicUsize::new(1)),
            rotate_after_first_decide: true,
            started: None,
            resume: None,
        },
        WorkerAuthenticator::new(&credential),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async {
                let _ = stopped.await;
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
    let schema = vec![Question {
        id: QuestionId::new("kind")?,
        kind: QuestionKind::Choice {
            options: vec![Label::new("bug")?],
        },
    }];
    let input = DecisionInput {
        text: "same request across worker reload".into(),
        context: None,
    };
    client.decide(&schema, &input).await?;
    client.decide(&schema, &input).await?;
    let observed = sizes.lock().map_err(|_| "probe mutex poisoned")?.clone();
    let _ = stop.send(());
    server.await??;
    assert_eq!(observed, vec![1, 1]);
    Ok(())
}

#[tokio::test]
async fn cancelled_caller_does_not_stall_other_items_in_the_batch()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
    let credential = WorkerCredential::generate()?;
    let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let started = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    let service = pb::worker_service_server::WorkerServiceServer::with_interceptor(
        BatchProbe {
            sizes: Arc::clone(&sizes),
            generation: Arc::new(AtomicUsize::new(1)),
            rotate_after_first_decide: false,
            started: Some(Arc::clone(&started)),
            resume: Some(Arc::clone(&resume)),
        },
        WorkerAuthenticator::new(&credential),
    );
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(service)
            .serve_with_incoming_shutdown(incoming, async {
                let _ = stopped.await;
            })
            .await
    });
    let channel = Endpoint::from_shared(format!("http://{address}"))?
        .connect()
        .await?;
    let client = Arc::new(S1Client::new(
        channel,
        ClientConfig::new(Duration::from_secs(2), 64)?,
        credential,
    )?);
    let schema = vec![Question {
        id: QuestionId::new("kind")?,
        kind: QuestionKind::Choice {
            options: vec![Label::new("bug")?],
        },
    }];
    let mut tasks = Vec::new();
    for index in 0..4 {
        let client = Arc::clone(&client);
        let schema = schema.clone();
        tasks.push(tokio::spawn(async move {
            client
                .decide(
                    &schema,
                    &DecisionInput {
                        text: format!("cancel-case-{index}"),
                        context: None,
                    },
                )
                .await
        }));
    }
    tokio::time::timeout(Duration::from_secs(1), started.notified()).await?;
    let mut tasks = tasks.into_iter();
    let cancelled = tasks
        .next()
        .ok_or("cancellation fixture did not create a caller")?;
    cancelled.abort();
    match cancelled.await {
        Err(error) if error.is_cancelled() => {}
        _ => return Err("cancelled caller completed unexpectedly".into()),
    }
    resume.notify_one();
    for task in tasks {
        task.await??;
    }
    let observed = sizes.lock().map_err(|_| "probe mutex poisoned")?.clone();
    let _ = stop.send(());
    server.await??;
    assert_eq!(observed, vec![4]);
    Ok(())
}
