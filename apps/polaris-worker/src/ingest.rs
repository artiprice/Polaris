//! CAS-backed ingestion. Synchronous parsers retain their admission permit after RPC cancellation.
//!
//! Byte/output limits do not preempt library code or bound its internal allocations. Deploy this
//! process with supervisor-enforced memory and wall-time limits for hostile PDF/OOXML inputs.
//! The configured CAS root is a trusted local service input, never supplied by an RPC caller.

use std::{
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use polaris_ingest::{DefaultDocParser, DocKind, LocatedBlock};
use polaris_proto::{PROTOCOL_VERSION, ProtocolVersion, negotiate, pb};
use polaris_sdk::{
    ids::Cid,
    registry::{DocBlock, IngestError},
};
use polaris_storage_cas::{Cas, CasConfig, CasError};
use serde::Serialize;
use tokio::sync::{Semaphore, watch};
use tonic::{Request, Response, Status};

const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;

/// Ingestion service with a single bounded parser owner and explicit, hash-verifying CAS access.
/// Clones share admission and shutdown state. No model is loaded and no host paths arrive over RPC.
#[derive(Debug, Clone)]
pub struct IngestWorker {
    cas: Cas,
    permits: Arc<Semaphore>,
    closed: Arc<AtomicBool>,
    shutdown: watch::Sender<bool>,
}

impl IngestWorker {
    /// Open the explicitly configured CAS with a 32 MiB decode limit and hash verification.
    ///
    /// Call on a blocking startup owner before serving RPCs. The directory must be absolute.
    /// # Errors
    /// Invalid root or CAS directory creation failure. The service is never returned half-open.
    pub fn open(cas_root: PathBuf, shutdown: watch::Sender<bool>) -> Result<Self, CasError> {
        if !cas_root.is_absolute() {
            return Err(CasError::Format("ingest CAS root must be absolute"));
        }
        let mut config = CasConfig::new(cas_root);
        config.max_blob_bytes = MAX_SOURCE_BYTES;
        config.verify_on_get = true;
        let cas = Cas::open(config)?;
        Ok(Self {
            cas,
            permits: Arc::new(Semaphore::new(1)),
            closed: Arc::new(AtomicBool::new(false)),
            shutdown,
        })
    }

    fn ensure_open(&self) -> Result<(), Status> {
        if self.closed.load(Ordering::Acquire) {
            Err(Status::unavailable("ingest admission is closed"))
        } else {
            Ok(())
        }
    }
}

#[tonic::async_trait]
impl pb::worker_service_server::WorkerService for IngestWorker {
    async fn handshake(
        &self,
        request: Request<pb::WorkerHandshakeRequest>,
    ) -> Result<Response<pb::WorkerHandshakeResponse>, Status> {
        self.ensure_open()?;
        let peer = request
            .into_inner()
            .protocol_version
            .ok_or_else(|| Status::invalid_argument("protocol_version is required"))?;
        let peer = ProtocolVersion::try_from(peer)
            .map_err(|_| Status::invalid_argument("invalid protocol version"))?;
        let version = negotiate(PROTOCOL_VERSION, peer)
            .map_err(|_| Status::failed_precondition("protocol major version mismatch"))?;
        Ok(Response::new(pb::WorkerHandshakeResponse {
            protocol_version: Some(version.into()),
            kind: i32::from(pb::WorkerKind::Ingest),
            worker_version: env!("CARGO_PKG_VERSION").to_owned(),
        }))
    }

    async fn decide(
        &self,
        _request: Request<pb::DecideRequest>,
    ) -> Result<Response<pb::DecideResponse>, Status> {
        Err(Status::unimplemented(
            "ingest mode does not provide decisions",
        ))
    }

    async fn detect_pii(
        &self,
        _request: Request<pb::DetectPiiRequest>,
    ) -> Result<Response<pb::DetectPiiResponse>, Status> {
        Err(Status::unimplemented(
            "ingest mode does not provide PII detection",
        ))
    }

    async fn embed(
        &self,
        _request: Request<pb::EmbedRequest>,
    ) -> Result<Response<pb::EmbedResponse>, Status> {
        Err(Status::unimplemented(
            "ingest mode does not provide embeddings",
        ))
    }

    async fn ingest(
        &self,
        request: Request<pb::IngestRequest>,
    ) -> Result<Response<pb::IngestResponse>, Status> {
        self.ensure_open()?;
        let request = request.into_inner();
        let source = Cid::try_from(
            request
                .source
                .ok_or_else(|| Status::invalid_argument("source CID is required"))?,
        )
        .map_err(|_| Status::invalid_argument("invalid source CID"))?;
        let media_type = normalize_media_type(&request.media_type)?;
        let permit = Arc::clone(&self.permits)
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("ingest parser is busy or closed"))?;
        self.ensure_open()?;
        let cas = self.cas.clone();
        // Cancellation drops only the async waiter. The blocking owner keeps the permit until
        // parsing/storage ends, preventing abandoned calls from growing an unbounded work queue.
        let response = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            ingest_source(&cas, source, &media_type)
        })
        .await
        .map_err(|_| Status::internal("ingest parser owner failed"))??;
        Ok(Response::new(response))
    }

    async fn evict_models(
        &self,
        _request: Request<pb::EvictModelsRequest>,
    ) -> Result<Response<pb::EvictModelsResponse>, Status> {
        Ok(Response::new(pb::EvictModelsResponse {
            evicted: 0,
            ..Default::default()
        }))
    }

    async fn health(
        &self,
        _request: Request<pb::HealthRequest>,
    ) -> Result<Response<pb::HealthResponse>, Status> {
        Ok(Response::new(pb::HealthResponse {
            protocol_version: Some(PROTOCOL_VERSION.into()),
            build_version: env!("CARGO_PKG_VERSION").to_owned(),
            ready: !self.closed.load(Ordering::Acquire),
            loaded_model_identity_blake3: Vec::new(),
            loaded_model_generation: 0,
            worker_incarnation: Vec::new(),
            loaded_pii_model_identity_blake3: Vec::new(),
            loaded_pii_model_generation: 0,
        }))
    }

    async fn shutdown(
        &self,
        request: Request<pb::ShutdownRequest>,
    ) -> Result<Response<pb::ShutdownResponse>, Status> {
        let grace_ms = request.into_inner().grace_ms;
        if grace_ms > 120_000 {
            return Err(Status::invalid_argument(
                "shutdown grace exceeds 120 seconds",
            ));
        }
        self.closed.store(true, Ordering::Release);
        self.shutdown
            .send(true)
            .map_err(|_| Status::unavailable("shutdown transport owner is gone"))?;
        // The permit belongs to the blocking parser, including when its RPC waiter was
        // cancelled. Waiting here observes actual completion; a timeout never stops that thread.
        let permit = if grace_ms == 0 {
            Arc::clone(&self.permits)
                .try_acquire_owned()
                .map_err(|_| Status::deadline_exceeded("ingest parser has not drained"))?
        } else {
            tokio::time::timeout(
                std::time::Duration::from_millis(u64::from(grace_ms)),
                Arc::clone(&self.permits).acquire_owned(),
            )
            .await
            .map_err(|_| {
                Status::deadline_exceeded("ingest parser did not drain before grace expired")
            })?
            .map_err(|_| Status::unavailable("ingest parser admission owner stopped"))?
        };
        drop(permit);
        Ok(Response::new(pb::ShutdownResponse {}))
    }
}

fn normalize_media_type(value: &str) -> Result<String, Status> {
    if value.len() > 256 || value.chars().any(char::is_control) {
        return Err(Status::invalid_argument("invalid document media type"));
    }
    let mime = value
        .split(';')
        .next()
        .ok_or_else(|| Status::invalid_argument("invalid media type"))?
        .trim();
    if mime.is_empty() {
        return Ok("application/octet-stream".to_owned());
    }
    Ok(mime.to_ascii_lowercase())
}

fn source_name(media_type: &str) -> &'static str {
    match media_type {
        "text/plain" => "document.txt",
        "text/markdown" => "document.md",
        "text/html" => "document.html",
        "text/csv" => "document.csv",
        "text/tab-separated-values" => "document.tsv",
        "application/pdf" => "document.pdf",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            "document.docx"
        }
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "document.xlsx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            "document.pptx"
        }
        _ => "document.bin",
    }
}

fn ingest_source(cas: &Cas, source: Cid, media_type: &str) -> Result<pb::IngestResponse, Status> {
    let bytes = cas.get(&source).map_err(cas_status)?;
    let parsed = DefaultDocParser::default()
        .ingest_bytes(source_name(media_type), Some(media_type), &bytes)
        .map_err(ingest_status)?;
    let blocks = parsed
        .blocks
        .iter()
        .map(|block| RecordBlock {
            block: &block.block,
            location: RecordLocation::from(block),
        })
        .collect::<Vec<_>>();
    // Serialize borrowed blocks with a capped writer, without duplicating document text.
    let markdown =
        polaris_ingest::to_markdown_blocks(parsed.blocks.iter().map(|located| &located.block));
    if markdown.len() > MAX_RECORD_BYTES {
        return Err(Status::resource_exhausted(
            "normalized document exceeds output limit",
        ));
    }
    let text = polaris_storage_cas::content_id(markdown.as_bytes());
    let record = Record {
        schema: 1,
        source: source.to_hex(),
        text: text.to_hex(),
        media_type,
        format: kind_name(parsed.kind),
        trust: "untrusted",
        title: parsed.title.as_deref(),
        blocks,
    };
    let mut writer = BoundedRecord::default();
    if serde_json::to_writer(&mut writer, &record).is_err() {
        if writer.exceeded {
            return Err(Status::resource_exhausted(
                "document provenance exceeds output limit",
            ));
        }
        return Err(Status::internal("document provenance serialization failed"));
    }
    let stored_text = cas.put(markdown.as_bytes()).map_err(cas_status)?;
    let structure = cas.put(&writer.bytes).map_err(cas_status)?;
    Ok(pb::IngestResponse {
        text: Some((&stored_text).into()),
        structure: Some((&structure).into()),
    })
}

fn kind_name(kind: DocKind) -> &'static str {
    match kind {
        DocKind::Docx => "docx",
        DocKind::Xlsx => "xlsx",
        DocKind::Csv => "csv",
        DocKind::Pdf => "pdf",
        DocKind::Pptx => "pptx",
        DocKind::Html => "html",
        DocKind::Markdown => "markdown",
        DocKind::Text => "text",
        DocKind::Image => "image",
    }
}

#[derive(Serialize)]
struct Record<'a> {
    schema: u32,
    source: String,
    text: String,
    media_type: &'a str,
    format: &'static str,
    trust: &'static str,
    title: Option<&'a str>,
    blocks: Vec<RecordBlock<'a>>,
}

#[derive(Serialize)]
struct RecordBlock<'a> {
    block: &'a DocBlock,
    location: RecordLocation<'a>,
}

#[derive(Serialize)]
struct RecordLocation<'a> {
    page: Option<u32>,
    line: Option<u32>,
    sheet: Option<&'a str>,
    cells: Option<&'a str>,
}

impl<'a> From<&'a LocatedBlock> for RecordLocation<'a> {
    fn from(block: &'a LocatedBlock) -> Self {
        Self {
            page: block.location.page,
            line: block.location.line,
            sheet: block.location.sheet.as_deref(),
            cells: block.location.cells.as_deref(),
        }
    }
}

#[derive(Default)]
struct BoundedRecord {
    bytes: Vec<u8>,
    exceeded: bool,
}

impl Write for BoundedRecord {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self.bytes.len().checked_add(bytes.len());
        if !matches!(next, Some(length) if length <= MAX_RECORD_BYTES) {
            self.exceeded = true;
            return Err(io::Error::other("document provenance limit exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn cas_status(error: CasError) -> Status {
    match error {
        CasError::NotFound(_) | CasError::UnknownShortRef(_) => {
            Status::not_found("source blob is absent")
        }
        CasError::TooLarge { .. } => Status::resource_exhausted("CAS blob exceeds ingest limit"),
        CasError::Corrupt { .. } | CasError::Decompress(_) | CasError::Format(_) => {
            Status::data_loss("CAS integrity verification failed")
        }
        CasError::Io(_) => Status::unavailable("document storage is unavailable"),
        CasError::Ambiguous(_) | CasError::ShortRefCollision(_) => {
            Status::internal("document storage reference conflict")
        }
    }
}

fn ingest_status(error: IngestError) -> Status {
    // Parser diagnostic strings can contain document text. Do not copy them into RPC errors.
    match error {
        IngestError::Unsupported(_) => {
            Status::unimplemented("document format or OCR is not configured")
        }
        IngestError::Malformed(_) => {
            Status::invalid_argument("document is malformed or exceeds parser limits")
        }
        IngestError::TooLarge { .. } => {
            Status::resource_exhausted("document exceeds parser resource limits")
        }
        IngestError::Ocr(_) => Status::failed_precondition("configured OCR failed"),
    }
}

#[cfg(test)]
mod tests {
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
}
