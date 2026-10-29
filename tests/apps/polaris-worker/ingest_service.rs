use polaris_proto::{PROTOCOL_VERSION, pb, pb::worker_service_server::WorkerService};
use polaris_sdk::ids::Cid;
use polaris_storage_cas::{Cas, CasConfig};
use polaris_worker::IngestWorker;
use serde_json::Value;
use tonic::{Code, Request};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[tokio::test]
async fn ingest_persists_text_and_untrusted_provenance() -> TestResult {
    let root = tempfile::tempdir()?;
    let cas = Cas::open(CasConfig::new(root.path()))?;
    let source = cas.put(b"first\n\nsecond\n")?;
    let (shutdown, _receiver) = tokio::sync::watch::channel(false);
    let worker = IngestWorker::open(root.path().to_path_buf(), shutdown)?;
    let response = worker
        .ingest(Request::new(pb::IngestRequest {
            source: Some((&source).into()),
            media_type: "text/plain".to_owned(),
        }))
        .await?
        .into_inner();
    let text = Cid::try_from(response.text.ok_or("text CID missing")?)?;
    let structure = Cid::try_from(response.structure.ok_or("structure CID missing")?)?;
    assert_eq!(
        cas.get(&text)?.as_ref(),
        b"first\n\nsecond\n\n",
        "exact normalized Markdown is durable"
    );
    let record: Value = serde_json::from_slice(&cas.get(&structure)?)?;
    assert_eq!(
        record.get("schema").and_then(Value::as_u64),
        Some(1),
        "versioned record"
    );
    assert_eq!(
        record.get("source").and_then(Value::as_str),
        Some(source.to_hex().as_str()),
        "source identity"
    );
    assert_eq!(
        record.get("text").and_then(Value::as_str),
        Some(text.to_hex().as_str()),
        "exact text identity"
    );
    assert_eq!(
        record.get("trust").and_then(Value::as_str),
        Some("untrusted"),
        "document data cannot acquire policy trust"
    );
    let blocks = record
        .get("blocks")
        .and_then(Value::as_array)
        .ok_or("blocks missing")?;
    let last = blocks.last().ok_or("block missing")?;
    assert_eq!(
        last.get("location")
            .and_then(|value| value.get("line"))
            .and_then(Value::as_u64),
        Some(3),
        "original blank lines preserve line locations"
    );
    Ok(())
}

#[tokio::test]
async fn invalid_or_missing_source_is_rejected() -> TestResult {
    let root = tempfile::tempdir()?;
    let (shutdown, _receiver) = tokio::sync::watch::channel(false);
    let worker = IngestWorker::open(root.path().to_path_buf(), shutdown)?;
    let missing = worker
        .ingest(Request::new(pb::IngestRequest {
            source: None,
            media_type: "text/plain".to_owned(),
        }))
        .await;
    assert!(
        matches!(missing, Err(error) if error.code() == Code::InvalidArgument),
        "missing source is malformed"
    );
    let absent = worker
        .ingest(Request::new(pb::IngestRequest {
            source: Some((&Cid::from_bytes([17; 32])).into()),
            media_type: "text/plain".to_owned(),
        }))
        .await;
    assert!(
        matches!(absent, Err(error) if error.code() == Code::NotFound),
        "absent source cannot produce success"
    );
    let malformed = worker
        .ingest(Request::new(pb::IngestRequest {
            source: Some(pb::Cid { digest: vec![0; 3] }),
            media_type: "text/plain".to_owned(),
        }))
        .await;
    assert!(
        matches!(malformed, Err(error) if error.code() == Code::InvalidArgument),
        "invalid digest cannot address CAS"
    );
    Ok(())
}

#[tokio::test]
async fn corrupt_and_oversized_sources_fail_before_parsing() -> TestResult {
    let root = tempfile::tempdir()?;
    let cas = Cas::open(CasConfig::new(root.path()))?;
    let source = cas.put(b"private source material")?;
    std::fs::write(cas.blob_path(&source), b"secret-corrupt-payload")?;
    let (shutdown, _receiver) = tokio::sync::watch::channel(false);
    let worker = IngestWorker::open(root.path().to_path_buf(), shutdown)?;
    let corrupt = worker
        .ingest(Request::new(pb::IngestRequest {
            source: Some((&source).into()),
            media_type: "text/plain".to_owned(),
        }))
        .await;
    assert!(
        matches!(corrupt, Err(error) if error.code() == Code::DataLoss && !error.message().contains("secret-corrupt-payload")),
        "CAS corruption is denied without reflecting document bytes"
    );
    let oversized = cas.put(&vec![b'x'; 33 * 1024 * 1024])?;
    let too_large = worker
        .ingest(Request::new(pb::IngestRequest {
            source: Some((&oversized).into()),
            media_type: "text/plain".to_owned(),
        }))
        .await;
    assert!(
        matches!(too_large, Err(error) if error.code() == Code::ResourceExhausted),
        "worker CAS decode cap is applied before parsing"
    );
    Ok(())
}

#[tokio::test]
async fn unsupported_ocr_remains_an_error() -> TestResult {
    let root = tempfile::tempdir()?;
    let cas = Cas::open(CasConfig::new(root.path()))?;
    let image = cas.put(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x02\0\0\0")?;
    let (shutdown, _receiver) = tokio::sync::watch::channel(false);
    let worker = IngestWorker::open(root.path().to_path_buf(), shutdown)?;
    let response = worker
        .ingest(Request::new(pb::IngestRequest {
            source: Some((&image).into()),
            media_type: "image/png".to_owned(),
        }))
        .await;
    assert!(
        matches!(response, Err(error) if error.code() == Code::Unimplemented),
        "no OCR success without configured OCR"
    );
    Ok(())
}

#[tokio::test]
async fn ingest_worker_protocol_and_lifecycle() -> TestResult {
    let root = tempfile::tempdir()?;
    let (shutdown, receiver) = tokio::sync::watch::channel(false);
    let worker = IngestWorker::open(root.path().to_path_buf(), shutdown)?;
    let handshake = worker
        .handshake(Request::new(pb::WorkerHandshakeRequest {
            protocol_version: Some(PROTOCOL_VERSION.into()),
        }))
        .await?
        .into_inner();
    assert_eq!(
        pb::WorkerKind::try_from(handshake.kind)?,
        pb::WorkerKind::Ingest,
        "actual ingest worker identity"
    );
    let ready = worker
        .health(Request::new(pb::HealthRequest {}))
        .await?
        .into_inner();
    assert!(ready.ready, "CAS initialized before admission");
    let decision = worker
        .decide(Request::new(pb::DecideRequest { items: Vec::new() }))
        .await;
    assert!(
        matches!(decision, Err(error) if error.code() == Code::Unimplemented),
        "modes do not masquerade as each other"
    );
    assert_eq!(
        worker
            .evict_models(Request::new(pb::EvictModelsRequest::default()))
            .await?
            .into_inner()
            .evicted,
        0,
        "no nonexistent model eviction"
    );
    let _shutdown = worker
        .shutdown(Request::new(pb::ShutdownRequest { grace_ms: 1000 }))
        .await?;
    assert!(*receiver.borrow(), "shutdown reaches transport owner");
    assert!(
        !worker
            .health(Request::new(pb::HealthRequest {}))
            .await?
            .into_inner()
            .ready,
        "closed admission is not ready"
    );
    let closed = worker
        .ingest(Request::new(pb::IngestRequest {
            source: None,
            media_type: "text/plain".to_owned(),
        }))
        .await;
    assert!(
        matches!(closed, Err(error) if error.code() == Code::Unavailable),
        "post-shutdown requests are denied"
    );
    Ok(())
}
