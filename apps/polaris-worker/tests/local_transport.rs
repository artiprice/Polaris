#![cfg(windows)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hyper_util::rt::TokioIo;
use polaris_proto::worker_auth::{WorkerAuthenticator, WorkerCredential};
use polaris_proto::{pb, pb::worker_service_client::WorkerServiceClient};
use polaris_sdk::ids::Cid;
use polaris_storage_cas::{Cas, CasConfig};
use polaris_worker::{IngestWorker, serve_named_pipe};
use tonic::{Request, transport::Endpoint};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

fn unique_pipe() -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    Ok(format!(
        r"\\.\pipe\polaris-ingest-{}-{elapsed}",
        std::process::id()
    ))
}

async fn connect_pipe(
    path: String,
) -> Result<WorkerServiceClient<tonic::transport::Channel>, Box<dyn std::error::Error + Send + Sync>>
{
    let connector = tower::service_fn(move |_uri| {
        let name = path.clone();
        async move {
            for _attempt in 0..100 {
                match tokio::net::windows::named_pipe::ClientOptions::new().open(&name) {
                    Ok(client) => return Ok(TokioIo::new(client)),
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                        ) =>
                    {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "worker pipe did not become available",
            ))
        }
    });
    let channel = Endpoint::from_static("http://[::]:50051")
        .connect_timeout(Duration::from_secs(3))
        .connect_with_connector(connector)
        .await?;
    Ok(WorkerServiceClient::new(channel)
        .max_decoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES)
        .max_encoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES))
}

#[tokio::test]
async fn windows_pipe_ingest_handshake_content_and_shutdown() -> TestResult {
    let root = tempfile::tempdir()?;
    let cas = Cas::open(CasConfig::new(root.path()))?;
    let source = cas.put(b"pipe transport content")?;
    let pipe = unique_pipe()?;
    let credential = WorkerCredential::generate()?;
    let (shutdown, shutdown_signal) = tokio::sync::watch::channel(false);
    let worker = IngestWorker::open(root.path().to_path_buf(), shutdown)?;
    let authenticator = WorkerAuthenticator::new(&credential);
    let serving_pipe = pipe.clone();
    let serving = tokio::spawn(async move {
        serve_named_pipe(serving_pipe, worker, authenticator, shutdown_signal).await
    });
    let mut client = connect_pipe(pipe.clone()).await?;
    let unauthenticated = client
        .handshake(Request::new(pb::WorkerHandshakeRequest {
            protocol_version: Some(polaris_proto::PROTOCOL_VERSION.into()),
        }))
        .await;
    assert!(
        matches!(unauthenticated, Err(status) if status.code() == tonic::Code::Unauthenticated)
    );
    let wrong = WorkerCredential::generate()?;
    let unauthorized = client
        .handshake(
            wrong.authorize_request(Request::new(pb::WorkerHandshakeRequest {
                protocol_version: Some(polaris_proto::PROTOCOL_VERSION.into()),
            }))?,
        )
        .await;
    assert!(matches!(unauthorized, Err(status) if status.code() == tonic::Code::Unauthenticated));
    let handshake = client
        .handshake(
            credential.authorize_request(Request::new(pb::WorkerHandshakeRequest {
                protocol_version: Some(polaris_proto::PROTOCOL_VERSION.into()),
            }))?,
        )
        .await?
        .into_inner();
    assert_eq!(
        pb::WorkerKind::try_from(handshake.kind)?,
        pb::WorkerKind::Ingest
    );
    let ingested = client
        .ingest(
            credential.authorize_request(Request::new(pb::IngestRequest {
                source: Some((&source).into()),
                media_type: "text/plain".to_owned(),
            }))?,
        )
        .await?
        .into_inner();
    let text = Cid::try_from(ingested.text.ok_or("ingest text reference missing")?)?;
    assert_eq!(cas.get(&text)?.as_ref(), b"pipe transport content\n\n");
    client
        .shutdown(
            credential.authorize_request(Request::new(pb::ShutdownRequest { grace_ms: 2_000 }))?,
        )
        .await?;
    drop(client);
    tokio::time::timeout(Duration::from_secs(3), serving).await???;
    let after_shutdown = tokio::net::windows::named_pipe::ClientOptions::new().open(&pipe);
    assert!(
        after_shutdown.is_err(),
        "shutdown closes all pipe instances"
    );
    Ok(())
}

#[tokio::test]
async fn windows_pipe_collision_fails_before_serve_returns_success() -> TestResult {
    let first_root = tempfile::tempdir()?;
    let second_root = tempfile::tempdir()?;
    let pipe = unique_pipe()?;
    let first_credential = WorkerCredential::generate()?;
    let (first_shutdown, first_receiver) = tokio::sync::watch::channel(false);
    let first_worker = IngestWorker::open(first_root.path().to_path_buf(), first_shutdown)?;
    let first_authenticator = WorkerAuthenticator::new(&first_credential);
    let serving_pipe = pipe.clone();
    let serving = tokio::spawn(async move {
        serve_named_pipe(
            serving_pipe,
            first_worker,
            first_authenticator,
            first_receiver,
        )
        .await
    });
    let mut client = connect_pipe(pipe.clone()).await?;
    let (second_shutdown, _second_receiver) = tokio::sync::watch::channel(false);
    let second_worker = IngestWorker::open(second_root.path().to_path_buf(), second_shutdown)?;
    let second_credential = WorkerCredential::generate()?;
    let second_authenticator = WorkerAuthenticator::new(&second_credential);
    let collision = serve_named_pipe(
        pipe,
        second_worker,
        second_authenticator,
        tokio::sync::watch::channel(false).1,
    )
    .await;
    assert!(
        collision.is_err(),
        "first-instance collision is synchronous"
    );
    client
        .shutdown(
            first_credential
                .authorize_request(Request::new(pb::ShutdownRequest { grace_ms: 2_000 }))?,
        )
        .await?;
    drop(client);
    tokio::time::timeout(Duration::from_secs(3), serving).await???;
    Ok(())
}
