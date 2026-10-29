// Unit tests of apps/polaris-worker/src/lib.rs (`tests`); compiled into that module via #[path].
use std::path::PathBuf;
use std::time::Duration;

use super::{S1IdlePolicy, S1Worker, bounded_server};
use polaris_proto::pb;
use polaris_system_one_engine::{EngineConfig, PiiEngineConfig, TextNormalization};
use tokio::sync::watch;
use tonic::Request;

fn worker() -> Result<(S1Worker, watch::Receiver<bool>), std::io::Error> {
    let (shutdown, receiver) = watch::channel(false);
    let config = EngineConfig::new(
        PathBuf::from("missing-model"),
        PathBuf::from("missing-runtime.dll"),
        TextNormalization::Raw,
    );
    Ok((S1Worker::new(config, shutdown)?, receiver))
}

fn worker_with_pii_config() -> Result<(S1Worker, watch::Receiver<bool>), std::io::Error> {
    let (shutdown, receiver) = watch::channel(false);
    let config = EngineConfig::new(
        PathBuf::from("missing-decide-model"),
        PathBuf::from("missing-runtime.dll"),
        TextNormalization::Raw,
    );
    let pii = PiiEngineConfig::new(
        PathBuf::from("missing-pii-model"),
        PathBuf::from("missing-runtime.dll"),
    );
    Ok((
        S1Worker::with_pii_engine(config, pii, S1IdlePolicy::default(), shutdown)?,
        receiver,
    ))
}

#[tokio::test]
async fn empty_decision_batch_is_rejected_before_model_loading()
-> Result<(), Box<dyn std::error::Error>> {
    let (worker, _shutdown) = worker()?;
    let response = pb::worker_service_server::WorkerService::decide(
        &worker,
        Request::new(pb::DecideRequest { items: Vec::new() }),
    )
    .await;
    assert!(matches!(response, Err(status) if status.code() == tonic::Code::ResourceExhausted));
    Ok(())
}

#[tokio::test]
async fn health_has_no_loaded_model_identity_before_first_decision()
-> Result<(), Box<dyn std::error::Error>> {
    let (worker, _shutdown) = worker()?;
    let response = pb::worker_service_server::WorkerService::health(
        &worker,
        Request::new(pb::HealthRequest {}),
    )
    .await?
    .into_inner();
    assert!(response.loaded_model_identity_blake3.is_empty());
    assert_eq!(response.loaded_model_generation, 0);
    assert_eq!(response.worker_incarnation.len(), 16);
    assert!(response.loaded_pii_model_identity_blake3.is_empty());
    assert_eq!(response.loaded_pii_model_generation, 0);
    worker.drain_local(Duration::from_secs(1)).await?;
    Ok(())
}

#[tokio::test]
async fn worker_incarnation_changes_across_process_owners() -> Result<(), Box<dyn std::error::Error>>
{
    let (first, _first_shutdown) = worker()?;
    let (second, _second_shutdown) = worker()?;
    let first_health = pb::worker_service_server::WorkerService::health(
        &first,
        Request::new(pb::HealthRequest {}),
    )
    .await?
    .into_inner();
    let second_health = pb::worker_service_server::WorkerService::health(
        &second,
        Request::new(pb::HealthRequest {}),
    )
    .await?
    .into_inner();
    assert_ne!(
        first_health.worker_incarnation,
        second_health.worker_incarnation
    );
    first.drain_local(Duration::from_secs(1)).await?;
    second.drain_local(Duration::from_secs(1)).await?;
    Ok(())
}

#[tokio::test]
async fn pii_rpc_unavailable_is_explicit_when_no_companion_model_is_configured()
-> Result<(), Box<dyn std::error::Error>> {
    let (worker, _shutdown) = worker()?;
    let result = pb::worker_service_server::WorkerService::detect_pii(
        &worker,
        Request::new(pb::DetectPiiRequest {
            raw_text: "Alice Example".to_owned(),
        }),
    )
    .await;
    assert!(matches!(result, Err(status) if status.code() == tonic::Code::Unavailable));
    worker.drain_local(Duration::from_secs(1)).await?;
    Ok(())
}

#[tokio::test]
async fn pii_input_cap_is_checked_before_loading_configured_model()
-> Result<(), Box<dyn std::error::Error>> {
    let (worker, _shutdown) = worker_with_pii_config()?;
    let result = pb::worker_service_server::WorkerService::detect_pii(
        &worker,
        Request::new(pb::DetectPiiRequest {
            raw_text: "x".repeat(polaris_proto::limits::MAX_PII_INPUT_BYTES + 1),
        }),
    )
    .await;
    assert!(matches!(result, Err(status) if status.code() == tonic::Code::ResourceExhausted));
    worker.drain_local(Duration::from_secs(1)).await?;
    Ok(())
}

#[tokio::test]
async fn ingest_and_embedding_do_not_claim_s1_results() -> Result<(), Box<dyn std::error::Error>> {
    let (worker, _shutdown) = worker()?;
    let embedding = pb::worker_service_server::WorkerService::embed(
        &worker,
        Request::new(pb::EmbedRequest { texts: Vec::new() }),
    )
    .await;
    let ingest = pb::worker_service_server::WorkerService::ingest(
        &worker,
        Request::new(pb::IngestRequest {
            source: None,
            media_type: String::new(),
        }),
    )
    .await;
    assert!(matches!(embedding, Err(status) if status.code() == tonic::Code::Unimplemented));
    assert!(matches!(ingest, Err(status) if status.code() == tonic::Code::Unimplemented));
    Ok(())
}

#[tokio::test]
async fn canceled_shutdown_still_signals_transport_stop() -> Result<(), Box<dyn std::error::Error>>
{
    let (worker, mut shutdown) = worker()?;
    let drain_guard = worker.drain_lock.lock().await;
    let shutdown_worker = worker.clone();
    let shutdown_call = tokio::spawn(async move {
        pb::worker_service_server::WorkerService::shutdown(
            &shutdown_worker,
            Request::new(pb::ShutdownRequest { grace_ms: 60_000 }),
        )
        .await
    });
    tokio::time::timeout(Duration::from_millis(100), shutdown.changed()).await??;
    shutdown_call.abort();
    match shutdown_call.await {
        Ok(_) | Err(_) => {}
    }
    drop(drain_guard);
    worker.drain_local(Duration::from_secs(1)).await?;
    Ok(())
}

#[tokio::test]
async fn drain_lock_wait_observes_requested_grace() -> Result<(), Box<dyn std::error::Error>> {
    let (worker, _shutdown) = worker()?;
    let drain_guard = worker.drain_lock.lock().await;
    let started = tokio::time::Instant::now();
    let draining_worker = worker.clone();
    let mut drainage =
        tokio::spawn(async move { draining_worker.drain_local(Duration::from_millis(20)).await });
    tokio::time::sleep(Duration::from_millis(1)).await;
    let health = pb::worker_service_server::WorkerService::health(
        &worker,
        Request::new(pb::HealthRequest {}),
    )
    .await?;
    assert!(!health.into_inner().ready);
    let result = tokio::time::timeout(Duration::from_millis(200), &mut drainage).await;
    assert!(
        matches!(result, Ok(Ok(Err(status))) if status.code() == tonic::Code::DeadlineExceeded)
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    drop(drain_guard);
    worker.drain_local(Duration::from_secs(1)).await?;
    Ok(())
}

#[tokio::test]
async fn transport_shutdown_wait_is_bounded() -> Result<(), Box<dyn std::error::Error>> {
    let (_shutdown, receiver) = watch::channel(true);
    let result = bounded_server(
        std::future::pending::<Result<(), std::io::Error>>(),
        receiver,
        Duration::from_millis(20),
    )
    .await;
    assert!(matches!(result, Err(super::ServerWaitError::Deadline)));
    Ok(())
}
