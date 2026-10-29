// Unit tests of apps/polaris-worker/src/ingest.rs (`tests`); compiled into that module via #[path].
use super::{BoundedRecord, IngestWorker, MAX_RECORD_BYTES, ingest_status};
use polaris_proto::{pb, pb::worker_service_server::WorkerService};
use polaris_sdk::registry::IngestError;
use std::{io::Write, sync::Arc, time::Duration};
use tonic::{Code, Request};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[test]
fn provenance_writer_rejects_growth_and_parser_errors_are_private() -> TestResult {
    let mut writer = BoundedRecord::default();
    writer.write_all(&vec![b'x'; MAX_RECORD_BYTES])?;
    assert!(
        writer.write_all(b"x").is_err(),
        "record cap rejects extra data"
    );
    assert_eq!(
        writer.bytes.len(),
        MAX_RECORD_BYTES,
        "overflow is never appended"
    );
    for error in [
        IngestError::Malformed("private-document-content".to_owned()),
        IngestError::Ocr("private-document-content".to_owned()),
    ] {
        assert!(
            !ingest_status(error)
                .message()
                .contains("private-document-content"),
            "parser-controlled bytes never cross error boundary"
        );
    }
    Ok(())
}

#[tokio::test]
async fn shutdown_waits_for_active_parser_permit_and_closes_admission() -> TestResult {
    let root = tempfile::tempdir()?;
    let (shutdown, _receiver) = tokio::sync::watch::channel(false);
    let worker = IngestWorker::open(root.path().to_path_buf(), shutdown)?;
    let permit = Arc::clone(&worker.permits).acquire_owned().await?;
    let source = polaris_sdk::ids::Cid::from_bytes([9; 32]);
    let busy = worker
        .ingest(Request::new(pb::IngestRequest {
            source: Some((&source).into()),
            media_type: "text/plain".to_owned(),
        }))
        .await;
    assert!(
        matches!(busy, Err(error) if error.code() == Code::ResourceExhausted),
        "contending requests are refused without a queue"
    );
    let owner = worker.clone();
    let mut task = tokio::spawn(async move {
        owner
            .shutdown(Request::new(pb::ShutdownRequest { grace_ms: 1000 }))
            .await
    });
    let early = tokio::time::timeout(Duration::from_millis(10), &mut task).await;
    assert!(
        early.is_err(),
        "shutdown cannot claim drain while parser still owns admission"
    );
    assert!(
        !worker
            .health(Request::new(pb::HealthRequest {}))
            .await?
            .into_inner()
            .ready,
        "shutdown closes readiness before drain"
    );
    drop(permit);
    let _result = tokio::time::timeout(Duration::from_secs(1), task).await???;
    Ok(())
}

#[tokio::test]
async fn shutdown_reports_drain_timeout_without_claiming_parser_stopped() -> TestResult {
    let root = tempfile::tempdir()?;
    let (shutdown, receiver) = tokio::sync::watch::channel(false);
    let worker = IngestWorker::open(root.path().to_path_buf(), shutdown)?;
    let _permit = Arc::clone(&worker.permits).acquire_owned().await?;
    let response = worker
        .shutdown(Request::new(pb::ShutdownRequest { grace_ms: 1 }))
        .await;
    assert!(
        matches!(response, Err(error) if error.code() == Code::DeadlineExceeded),
        "a synchronous parser cannot be forcibly cancelled by a grace timeout"
    );
    assert!(
        *receiver.borrow(),
        "supervisor/transport still receives shutdown"
    );
    Ok(())
}
