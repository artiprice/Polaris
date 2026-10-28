//! Bounded JSON-lines control for an explicitly trusted non-model host transport.
//!
//! This is a host authority surface: transport authentication, local ACLs and endpoint custody
//! belong to the embedding host. Never pass these streams or a host handle to agent tools.
//! Model-requested tests still wait for a separate, exact human approval through this boundary.
use crate::{HumanRequestOutcome, KernelHandle};
use polaris_sdk::{
    human::{HumanQuestionAnswer, HumanRequestId},
    ids::{Cid, FrameId},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

const MAX_LINE_BYTES: usize = 1_048_576;
const MAX_RESPONSE_BYTES: usize = 2_097_152;
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Transport failures contain no request/answer text or credential values.
#[derive(Debug, thiserror::Error)]
pub enum HostControlError {
    /// Host transport read/write failed or the bounded write deadline expired.
    #[error("host control transport failed")]
    Transport,
    /// One request exceeded the hard line limit; the stream must be replaced.
    #[error("host control request exceeded size limit")]
    Limit,
    /// A response could not be encoded within the hard output limit.
    #[error("host control response exceeded size limit")]
    Response,
    /// Host authority files must live in an explicitly provisioned private directory outside the workspace.
    #[error("host control directory rejected")]
    Directory,
}

impl polaris_sdk::error::HasKind for HostControlError {
    fn kind(&self) -> polaris_sdk::error::ErrorKind {
        match self {
            Self::Transport => polaris_sdk::error::ErrorKind::Transient,
            Self::Limit | Self::Response => polaris_sdk::error::ErrorKind::Permanent,
            Self::Directory => polaris_sdk::error::ErrorKind::Policy,
        }
    }
}

/// Serve an explicit host-private request directory without an uninterruptible stdin reader.
///
/// A host atomically publishes `<id>.request.json`, using the same bounded request schema as the
/// stream API. This owner claims it as `<id>.processing.json` before dispatch, then atomically
/// publishes `<id>.response.json`. `id` must be 1–64 ASCII alphanumeric/underscore/hyphen bytes.
/// A leftover `.processing.json` is an uncertain accepted operation after interruption: it is
/// never retried automatically. Hosts reconcile it against kernel receipts/events before retrying.
/// The directory must be protected by the host's ACLs, remain outside the workspace, and have no
/// hostile concurrent entry replacement. This module neither provisions ACLs nor accepts model paths.
/// # Errors
/// Invalid directory, bounded queue exhaustion, kernel admission, owner failure or file transport error.
pub async fn serve_host_control_directory(
    kernel: KernelHandle,
    directory: impl AsRef<Path>,
    cancellation: &CancellationToken,
) -> Result<(), HostControlError> {
    let directory = directory.as_ref().to_owned();
    if !directory.is_absolute() {
        return Err(HostControlError::Directory);
    }
    let owner = kernel
        .inspection_owner()
        .await
        .map_err(|_| HostControlError::Directory)?;
    let (directory, directory_lock) = tokio::task::spawn_blocking(move || {
        owner
            .ensure_open()
            .map_err(|_| HostControlError::Directory)?;
        let metadata =
            std::fs::symlink_metadata(&directory).map_err(|_| HostControlError::Directory)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(HostControlError::Directory);
        }
        let directory = directory
            .canonicalize()
            .map_err(|_| HostControlError::Directory)?;
        let workspace = owner
            .workspace_root()
            .canonicalize()
            .map_err(|_| HostControlError::Directory)?;
        if directory.starts_with(&workspace) || workspace.starts_with(&directory) {
            return Err(HostControlError::Directory);
        }
        let lock_path = directory.join(".polaris-host-control.lock");
        if let Ok(metadata) = std::fs::symlink_metadata(&lock_path) {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(HostControlError::Directory);
            }
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(lock_path)
            .map_err(|_| HostControlError::Directory)?;
        file.try_lock().map_err(|_| HostControlError::Directory)?;
        Ok((directory, Arc::new(file)))
    })
    .await
    .map_err(|_| HostControlError::Transport)??;
    let directory = Arc::new(directory);
    loop {
        if cancellation.is_cancelled() {
            return Ok(());
        }
        let owner = kernel
            .inspection_owner()
            .await
            .map_err(|_| HostControlError::Transport)?;
        let scan_root = Arc::clone(&directory);
        let scan_lock = Arc::clone(&directory_lock);
        let next = tokio::task::spawn_blocking(move || {
            let _retained_lock = scan_lock;
            owner
                .ensure_open()
                .map_err(|_| HostControlError::Transport)?;
            claim_directory_request(&scan_root)
        })
        .await
        .map_err(|_| HostControlError::Transport)??;
        let Some(DirectoryRequest {
            id,
            processing,
            bytes,
        }) = next
        else {
            tokio::select! {
                biased;
                () = cancellation.cancelled() => return Ok(()),
                () = tokio::time::sleep(Duration::from_millis(50)) => {},
            }
            continue;
        };
        // Once claimed, always persist a terminal response even if the host interrupts dispatch.
        let response = match serde_json::from_slice::<Request>(&bytes) {
            Ok(request) if valid_id(&request.id) && request.id == id => {
                let result = tokio::select! {
                    biased;
                    () = cancellation.cancelled() => Err("interrupted_outcome_may_be_accepted"),
                    result = dispatch(&kernel, request.request) => result,
                };
                match result {
                    Ok(result) => json!({"id":id,"ok":true,"result":result}),
                    Err(code) => json!({"id":id,"ok":false,"error":code}),
                }
            }
            Ok(_) | Err(_) => json!({"id":id,"ok":false,"error":"invalid_request"}),
        };
        let owner = kernel
            .inspection_owner()
            .await
            .map_err(|_| HostControlError::Transport)?;
        let write_root = Arc::clone(&directory);
        let write_lock = Arc::clone(&directory_lock);
        tokio::task::spawn_blocking(move || {
            let _retained_lock = write_lock;
            owner
                .ensure_open()
                .map_err(|_| HostControlError::Transport)?;
            publish_directory_response(&write_root, &id, &processing, &response)
        })
        .await
        .map_err(|_| HostControlError::Transport)??;
    }
}

struct DirectoryRequest {
    id: String,
    processing: PathBuf,
    bytes: Zeroizing<Vec<u8>>,
}

fn claim_directory_request(root: &Path) -> Result<Option<DirectoryRequest>, HostControlError> {
    use std::io::Read;
    let mut candidates = Vec::new();
    for (index, entry) in std::fs::read_dir(root)
        .map_err(|_| HostControlError::Transport)?
        .enumerate()
    {
        if index >= 4096 {
            return Err(HostControlError::Limit);
        }
        let entry = entry.map_err(|_| HostControlError::Transport)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(id) = name.strip_suffix(".request.json") else {
            continue;
        };
        if !valid_id(id) {
            return Err(HostControlError::Directory);
        }
        candidates.push(id.to_owned());
        if candidates.len() > 128 {
            return Err(HostControlError::Limit);
        }
    }
    candidates.sort();
    let Some(id) = candidates.into_iter().next() else {
        return Ok(None);
    };
    let request = root.join(format!("{id}.request.json"));
    let processing = root.join(format!("{id}.processing.json"));
    let response = root.join(format!("{id}.response.json"));
    if processing
        .try_exists()
        .map_err(|_| HostControlError::Transport)?
        || response
            .try_exists()
            .map_err(|_| HostControlError::Transport)?
    {
        return Err(HostControlError::Directory);
    }
    let metadata = std::fs::symlink_metadata(&request).map_err(|_| HostControlError::Transport)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(HostControlError::Directory);
    }
    if metadata.len() > u64::try_from(MAX_LINE_BYTES).map_err(|_| HostControlError::Limit)? {
        return Err(HostControlError::Limit);
    }
    std::fs::rename(&request, &processing).map_err(|_| HostControlError::Transport)?;
    let mut bytes = Zeroizing::new(Vec::new());
    std::fs::File::open(&processing)
        .map_err(|_| HostControlError::Transport)?
        .take(
            u64::try_from(MAX_LINE_BYTES)
                .map_err(|_| HostControlError::Limit)?
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)
        .map_err(|_| HostControlError::Transport)?;
    if bytes.len() > MAX_LINE_BYTES {
        return Err(HostControlError::Limit);
    }
    Ok(Some(DirectoryRequest {
        id,
        processing,
        bytes,
    }))
}

fn publish_directory_response(
    root: &Path,
    id: &str,
    processing: &Path,
    response: &Value,
) -> Result<(), HostControlError> {
    use std::io::Write;
    let bytes = serde_json::to_vec(response).map_err(|_| HostControlError::Response)?;
    let bytes = if bytes.len() <= MAX_RESPONSE_BYTES {
        bytes
    } else {
        serde_json::to_vec(&json!({"id":id,"ok":false,"error":"response_too_large"}))
            .map_err(|_| HostControlError::Response)?
    };
    let temporary = root.join(format!("{id}.response.pending"));
    let target = root.join(format!("{id}.response.json"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| HostControlError::Transport)?;
    file.write_all(&bytes)
        .map_err(|_| HostControlError::Transport)?;
    file.sync_all().map_err(|_| HostControlError::Transport)?;
    drop(file);
    if target
        .try_exists()
        .map_err(|_| HostControlError::Transport)?
    {
        return Err(HostControlError::Directory);
    }
    std::fs::rename(&temporary, &target).map_err(|_| HostControlError::Transport)?;
    #[cfg(unix)]
    std::fs::File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| HostControlError::Transport)?;
    std::fs::remove_file(processing).map_err(|_| HostControlError::Transport)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: String,
    request: Operation,
}

#[derive(Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    PendingQuestions,
    AnswerQuestions {
        request_id: HumanRequestId,
        revision: Cid,
        answers: Vec<HumanQuestionAnswer>,
    },
    ExecutionReview {
        review: Cid,
    },
    PendingExecutionReviews,
    RecoverS1ModelAdmission,
    ApproveExecution {
        review: Cid,
        accepted_digest: Cid,
    },
    CancelFrame {
        frame: FrameId,
    },
}

/// Serve one trusted duplex control session until EOF or explicit cancellation.
///
/// Request shape: `{"id":"host-1","request":{"method":"pending_questions"}}`.
/// Replies echo only a validated bounded identifier. Malformed requests receive a sanitized error;
/// oversized lines close the stream. Methods run serially, with no unbounded spawned request queue.
/// Cancellation drops the response wait; already accepted commands remain kernel-owned and must
/// be observed through durable events. Reconnecting must not blindly retry a one-use approval.
/// No approval is inferred from receiving a model question, an EOF, a timeout or a matching test name.
///
/// # Errors
/// Transport failure, oversized line or output. Operation rejections are encoded as bounded replies.
pub async fn serve_host_control<R, W>(
    kernel: KernelHandle,
    input: R,
    mut output: W,
    cancellation: &CancellationToken,
) -> Result<(), HostControlError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send,
{
    let mut input = BufReader::with_capacity(8192, input);
    loop {
        let line = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Ok(()),
            line = read_line(&mut input) => line?,
        };
        let Some(line) = line else {
            return Ok(());
        };
        let request = match serde_json::from_slice::<Request>(&line) {
            Ok(request) if valid_id(&request.id) => request,
            Ok(_) | Err(_) => {
                write_reply(
                    &mut output,
                    &json!({"id":null,"ok":false,"error":"invalid_request"}),
                    cancellation,
                )
                .await?;
                continue;
            }
        };
        let result = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Ok(()),
            result = dispatch(&kernel, request.request) => result,
        };
        let response = match result {
            Ok(result) => json!({"id":request.id,"ok":true,"result":result}),
            Err(code) => json!({"id":request.id,"ok":false,"error":code}),
        };
        write_reply(&mut output, &response, cancellation).await?;
    }
}

async fn dispatch(kernel: &KernelHandle, request: Operation) -> Result<Value, &'static str> {
    match request {
        Operation::PendingQuestions => {
            let questions = kernel
                .human_requests()
                .list()
                .await
                .map_err(|_| "questions_unavailable")?;
            let pending = questions
                .into_iter()
                .filter(|snapshot| snapshot.outcome == HumanRequestOutcome::Pending)
                .map(|snapshot| {
                    json!({
                        "request_id":snapshot.ticket.request_id,
                        "revision":snapshot.ticket.revision,
                        "frame":snapshot.ticket.frame,
                        "request":snapshot.request,
                    })
                })
                .collect::<Vec<_>>();
            Ok(json!({"questions":pending}))
        }
        Operation::AnswerQuestions {
            request_id,
            revision,
            answers,
        } => {
            let receipt = kernel
                .answer_human_request(request_id, revision, answers)
                .await
                .map_err(|_| "answer_rejected")?;
            Ok(json!({"command":receipt.command,"accepted_at":receipt.accepted_at}))
        }
        Operation::ExecutionReview { review } => {
            let review = kernel
                .execution_review(review)
                .await
                .map_err(|_| "review_rejected")?;
            serde_json::to_value(review).map_err(|_| "review_unavailable")
        }
        Operation::PendingExecutionReviews => {
            let reviews = kernel
                .pending_execution_reviews()
                .await
                .map_err(|_| "reviews_unavailable")?;
            Ok(json!({"reviews":reviews}))
        }
        Operation::ApproveExecution {
            review,
            accepted_digest,
        } => {
            if review != accepted_digest {
                return Err("approval_digest_mismatch");
            }
            let receipt = kernel
                .approve_execution(review, accepted_digest)
                .await
                .map_err(|_| "execution_rejected")?;
            Ok(json!({"command":receipt.command,"accepted_at":receipt.accepted_at}))
        }
        Operation::CancelFrame { frame } => {
            let receipt = kernel
                .cancel_frame(frame)
                .await
                .map_err(|_| "cancellation_rejected")?;
            Ok(json!({"command":receipt.command,"accepted_at":receipt.accepted_at}))
        }
        Operation::RecoverS1ModelAdmission => {
            kernel
                .recover_s1_model_admission()
                .await
                .map_err(|_| "model_admission_recovery_rejected")?;
            Ok(json!({"status":"model_admission_recovered"}))
        }
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

async fn read_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> Result<Option<Vec<u8>>, HostControlError> {
    let mut line = Vec::new();
    loop {
        let buffer = reader
            .fill_buf()
            .await
            .map_err(|_| HostControlError::Transport)?;
        if buffer.is_empty() {
            return Ok(if line.is_empty() { None } else { Some(line) });
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let consumed = match newline {
            Some(index) => index.checked_add(1).ok_or(HostControlError::Limit)?,
            None => buffer.len(),
        };
        let next_len = line
            .len()
            .checked_add(consumed)
            .ok_or(HostControlError::Limit)?;
        if next_len > MAX_LINE_BYTES {
            return Err(HostControlError::Limit);
        }
        line.extend_from_slice(buffer.get(..consumed).ok_or(HostControlError::Limit)?);
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(Some(line));
        }
    }
}

async fn write_reply<W: AsyncWrite + Unpin>(
    output: &mut W,
    response: &Value,
    cancellation: &CancellationToken,
) -> Result<(), HostControlError> {
    let mut bytes = serde_json::to_vec(response).map_err(|_| HostControlError::Response)?;
    if bytes.len() >= MAX_RESPONSE_BYTES {
        return Err(HostControlError::Response);
    }
    bytes.push(b'\n');
    let operation = async {
        output
            .write_all(&bytes)
            .await
            .map_err(|_| HostControlError::Transport)?;
        output
            .flush()
            .await
            .map_err(|_| HostControlError::Transport)
    };
    tokio::select! {
        biased;
        () = cancellation.cancelled() => Ok(()),
        result = tokio::time::timeout(WRITE_TIMEOUT, operation) => result.map_err(|_| HostControlError::Transport)?,
    }
}
