//! Kernel-side System-One client for the bounded `polaris.v1.WorkerService` RPC.
//!
//! The client supports configured local named-pipe and Unix-domain-socket endpoints and an injected
//! tonic channel. It validates the protocol handshake, limits each request, applies a separate
//! timeout to handshake and decision RPCs, and propagates worker errors. It does not cache answers
//! because the current wire contract does not identify the loaded checkpoint.

#![forbid(unsafe_code)]

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use hyper_util::rt::TokioIo;
use polaris_proto::worker_auth::WorkerCredential;
use polaris_proto::{PROTOCOL_VERSION, ProtocolVersion, negotiate, pb};
use polaris_sdk::registry::{DecisionEngine, DecisionError};
use polaris_sdk::system_one::{Answer, Decision, DecisionInput, Question, QuestionKind};
use std::collections::HashSet;
use tonic::transport::{Channel, Endpoint};
use tonic::{Request, Status};

const MAX_INPUT_BYTES: usize = 262_144;

/// Bounds and deadline for calls through one worker channel.
#[derive(Debug, Clone, Copy)]
pub struct ClientConfig {
    /// Maximum wall-clock time for handshake and one decision RPC.
    pub timeout: Duration,
    /// Maximum combined primary text and context length in UTF-8 bytes.
    pub max_input_bytes: usize,
}

impl ClientConfig {
    /// Validates the timeout and input bound.
    ///
    /// # Errors
    /// Returns [`ClientError::InvalidConfiguration`] when any bound is outside the protocol cap.
    pub fn new(timeout: Duration, max_input_bytes: usize) -> Result<Self, ClientError> {
        if timeout.is_zero() || max_input_bytes == 0 || max_input_bytes > MAX_INPUT_BYTES {
            return Err(ClientError::InvalidConfiguration);
        }
        Ok(Self {
            timeout,
            max_input_bytes,
        })
    }
}

/// Construction failures for an S1 client.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// A timeout or byte bound is zero or exceeds the supported protocol limit.
    #[error("invalid S1 client bounds")]
    InvalidConfiguration,
    /// The configured local endpoint is malformed or unsafe.
    #[error("invalid local worker endpoint")]
    InvalidEndpoint,
    /// The endpoint could not be opened.
    #[error("worker connection failed: {0}")]
    Connection(String),
}

/// `DecisionEngine` adapter for a single local S1 worker channel.
pub struct S1Client {
    channel: Channel,
    config: ClientConfig,
    credential: WorkerCredential,
    handshake: tokio::sync::OnceCell<()>,
}

impl std::fmt::Debug for S1Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("S1Client")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl S1Client {
    /// Wraps a tonic channel created by the composition root.
    ///
    /// # Errors
    /// Returns [`ClientError::InvalidConfiguration`] for an invalid deadline or bound.
    pub fn new(
        channel: Channel,
        config: ClientConfig,
        credential: WorkerCredential,
    ) -> Result<Self, ClientError> {
        if config.timeout.is_zero()
            || config.max_input_bytes == 0
            || config.max_input_bytes > MAX_INPUT_BYTES
        {
            return Err(ClientError::InvalidConfiguration);
        }
        Ok(Self {
            channel,
            config,
            credential,
            handshake: tokio::sync::OnceCell::new(),
        })
    }

    /// Connects to a local Windows named pipe whose name begins with `polaris-`.
    ///
    /// The service rejects remote clients and creates the first pipe instance exclusively. Access
    /// for local processes depends on the creator token's default pipe DACL; this crate does not
    /// verify that the DACL restricts access to a particular user or process.
    ///
    /// # Errors
    /// Returns an error for an invalid local pipe name or a failed tonic connection.
    #[cfg(windows)]
    pub async fn connect_named_pipe(
        path: PathBuf,
        config: ClientConfig,
        credential: WorkerCredential,
    ) -> Result<Self, ClientError> {
        let name = path.to_str().ok_or(ClientError::InvalidEndpoint)?;
        let prefix = r"\\.\pipe\polaris-";
        let suffix = name
            .strip_prefix(prefix)
            .ok_or(ClientError::InvalidEndpoint)?;
        if suffix.is_empty()
            || suffix.len() > 128
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(ClientError::InvalidEndpoint);
        }
        let connector = tower::service_fn(move |_uri| {
            let pipe_path = path.clone();
            async move {
                let pipe_name = pipe_path.to_str().ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid pipe name")
                })?;
                tokio::net::windows::named_pipe::ClientOptions::new()
                    .open(pipe_name)
                    .map(TokioIo::new)
            }
        });
        let channel = Endpoint::from_static("http://[::]:50051")
            .connect_with_connector(connector)
            .await
            .map_err(|error| ClientError::Connection(error.to_string()))?;
        Self::new(channel, config, credential)
    }

    /// Connects to a Unix socket in a private directory.
    ///
    /// # Errors
    /// Returns an error when the path is invalid, its parent is not private, or connection fails.
    #[cfg(unix)]
    pub async fn connect_unix(
        path: PathBuf,
        config: ClientConfig,
        credential: WorkerCredential,
    ) -> Result<Self, ClientError> {
        use std::os::unix::fs::PermissionsExt;

        if !path.is_absolute() || path.as_os_str().len() > 100 {
            return Err(ClientError::InvalidEndpoint);
        }
        let parent = path.parent().ok_or(ClientError::InvalidEndpoint)?;
        let metadata = std::fs::symlink_metadata(parent)
            .map_err(|error| ClientError::Connection(error.to_string()))?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(ClientError::InvalidEndpoint);
        }
        let connector = tower::service_fn(move |_uri| {
            let socket_path = path.clone();
            async move {
                tokio::net::UnixStream::connect(socket_path)
                    .await
                    .map(TokioIo::new)
            }
        });
        let channel = Endpoint::from_static("http://[::]:50051")
            .connect_with_connector(connector)
            .await
            .map_err(|error| ClientError::Connection(error.to_string()))?;
        Self::new(channel, config, credential)
    }

    async fn ensure_handshake(&self) -> Result<(), DecisionError> {
        self.handshake
            .get_or_try_init(|| async {
                let mut client = worker_client(self.channel.clone());
                let request = pb::WorkerHandshakeRequest {
                    protocol_version: Some(PROTOCOL_VERSION.into()),
                };
                let request = self
                    .credential
                    .authorize_request(Request::new(request))
                    .map_err(status_error)?;
                let response = tokio::time::timeout(self.config.timeout, client.handshake(request))
                    .await
                    .map_err(|_| DecisionError::Timeout)?
                    .map_err(status_error)?
                    .into_inner();
                let remote_version = response.protocol_version.ok_or_else(|| {
                    DecisionError::Model("worker omitted protocol version".to_owned())
                })?;
                let remote_version = ProtocolVersion::try_from(remote_version).map_err(|_| {
                    DecisionError::Model("worker sent invalid protocol version".to_owned())
                })?;
                negotiate(PROTOCOL_VERSION, remote_version).map_err(|_| {
                    DecisionError::Model("worker protocol major version mismatch".to_owned())
                })?;
                let kind = pb::WorkerKind::try_from(response.kind).map_err(|_| {
                    DecisionError::Model("worker sent invalid service kind".to_owned())
                })?;
                if kind != pb::WorkerKind::S1 {
                    return Err(DecisionError::Model(
                        "worker is not an S1 service".to_owned(),
                    ));
                }
                Ok(())
            })
            .await
            .map(|_| ())
    }

    async fn decide_rpc(
        &self,
        schema: &[Question],
        input: &DecisionInput,
    ) -> Result<Decision, DecisionError> {
        self.validate_request(schema, input)?;
        self.ensure_handshake().await?;
        let request = pb::DecideRequest {
            items: vec![pb::DecisionRequest {
                questions: schema.iter().map(pb::Question::from).collect(),
                input: Some(input.into()),
            }],
        };
        let mut client = worker_client(self.channel.clone());
        let request = self
            .credential
            .authorize_request(Request::new(request))
            .map_err(status_error)?;
        let response = tokio::time::timeout(self.config.timeout, client.decide(request))
            .await
            .map_err(|_| DecisionError::Timeout)?
            .map_err(status_error)?
            .into_inner();
        let mut decisions = response.decisions.into_iter();
        let decision = decisions
            .next()
            .ok_or_else(|| DecisionError::Model("worker returned no decision".to_owned()))?;
        if decisions.next().is_some() {
            return Err(DecisionError::Model(
                "worker returned an unexpected number of decisions".to_owned(),
            ));
        }
        let decision = Decision::try_from(decision)
            .map_err(|_| DecisionError::Model("worker returned a malformed decision".to_owned()))?;
        validate_response(schema, &decision)?;
        Ok(decision)
    }

    fn validate_request(
        &self,
        schema: &[Question],
        input: &DecisionInput,
    ) -> Result<(), DecisionError> {
        if schema.is_empty() || schema.len() > 64 {
            return Err(DecisionError::InvalidSchema(
                "question count must be between 1 and 64".to_owned(),
            ));
        }
        const MAX_SCHEMA_BYTES: usize = 65_536;
        let mut question_ids = HashSet::with_capacity(schema.len());
        let mut prompts = HashSet::with_capacity(schema.len());
        let mut schema_bytes = 0_usize;
        let context_len = match &input.context {
            Some(context) => context.len(),
            None => 0,
        };
        let separator = if input.context.is_some() {
            2_usize
        } else {
            0_usize
        };
        let total = input
            .text
            .len()
            .checked_add(context_len)
            .and_then(|size| size.checked_add(separator))
            .ok_or_else(|| DecisionError::InvalidSchema("input size overflow".to_owned()))?;
        if total > self.config.max_input_bytes {
            return Err(DecisionError::InvalidSchema(
                "input exceeds the configured S1 byte limit".to_owned(),
            ));
        }
        for question in schema {
            if !question_ids.insert(question.id.as_str()) {
                return Err(DecisionError::InvalidSchema(
                    "question IDs must be unique".to_owned(),
                ));
            }
            let (prompt, label_bytes) = match &question.kind {
                QuestionKind::Choice { options } => {
                    if options.is_empty() || options.len() > 128 {
                        return Err(DecisionError::InvalidSchema(
                            "choice count must be between 1 and 128".to_owned(),
                        ));
                    }
                    let mut unique = HashSet::with_capacity(options.len());
                    if options.iter().any(|label| !unique.insert(label.as_str())) {
                        return Err(DecisionError::InvalidSchema(
                            "choice labels must be unique".to_owned(),
                        ));
                    }
                    let label_bytes = options
                        .iter()
                        .try_fold(0_usize, |total, label| {
                            total.checked_add(label.as_str().len())
                        })
                        .ok_or_else(|| {
                            DecisionError::InvalidSchema("schema byte size overflow".to_owned())
                        })?;
                    (question.id.as_str(), label_bytes)
                }
                QuestionKind::Score { min, max } => {
                    if min > max
                        || !matches!(i64::from(*max).checked_sub(i64::from(*min)), Some(0..=256))
                    {
                        return Err(DecisionError::InvalidSchema(
                            "score range must be ordered and span at most 256".to_owned(),
                        ));
                    }
                    let label_bytes = (*min..=*max)
                        .try_fold(0_usize, |total, score| {
                            total.checked_add(score.to_string().len())
                        })
                        .ok_or_else(|| {
                            DecisionError::InvalidSchema("schema byte size overflow".to_owned())
                        })?;
                    (question.id.as_str(), label_bytes)
                }
                QuestionKind::Noul { statement } => {
                    if statement.trim().is_empty() || statement.len() > 4096 {
                        return Err(DecisionError::InvalidSchema(
                            "noul statement must be non-empty and at most 4096 bytes".to_owned(),
                        ));
                    }
                    (statement.as_str(), "yes".len() + "no".len())
                }
            };
            if prompt.len() > 4096 || !prompts.insert(prompt) {
                return Err(DecisionError::InvalidSchema(
                    "model prompt is oversized or duplicated".to_owned(),
                ));
            }
            schema_bytes = schema_bytes
                .checked_add(question.id.as_str().len())
                .and_then(|size| size.checked_add(prompt.len()))
                .and_then(|size| size.checked_add(label_bytes))
                .ok_or_else(|| {
                    DecisionError::InvalidSchema("schema byte size overflow".to_owned())
                })?;
            if schema_bytes > MAX_SCHEMA_BYTES {
                return Err(DecisionError::InvalidSchema(
                    "aggregate question schema exceeds 65536 bytes".to_owned(),
                ));
            }
        }
        Ok(())
    }
}

impl DecisionEngine for S1Client {
    fn decide(
        &self,
        schema: &[Question],
        input: &DecisionInput,
    ) -> impl Future<Output = Result<Decision, DecisionError>> + Send {
        async move { self.decide_rpc(schema, input).await }
    }
}

fn worker_client(channel: Channel) -> pb::worker_service_client::WorkerServiceClient<Channel> {
    pb::worker_service_client::WorkerServiceClient::new(channel)
        .max_decoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES)
        .max_encoding_message_size(polaris_proto::limits::MAX_MESSAGE_BYTES)
}

fn status_error(status: Status) -> DecisionError {
    match status.code() {
        tonic::Code::DeadlineExceeded => DecisionError::Timeout,
        tonic::Code::Unavailable | tonic::Code::ResourceExhausted => {
            DecisionError::Unavailable(format!("worker RPC returned {}", status.code()))
        }
        _ => DecisionError::Model(format!("worker RPC returned {}", status.code())),
    }
}

fn validate_response(schema: &[Question], decision: &Decision) -> Result<(), DecisionError> {
    if decision.answers.len() != schema.len() {
        return Err(DecisionError::Model(
            "worker returned the wrong answer count".to_owned(),
        ));
    }
    let mut returned_ids = HashSet::with_capacity(decision.answers.len());
    for (question, answer) in schema.iter().zip(&decision.answers) {
        if answer.question != question.id || !returned_ids.insert(answer.question.as_str()) {
            return Err(DecisionError::Model(
                "worker returned unexpected or duplicate question IDs".to_owned(),
            ));
        }
        match (&question.kind, &answer.answer) {
            (QuestionKind::Choice { options }, Answer::Choice { selected, probs }) => {
                let expected = options
                    .iter()
                    .map(|label| label.as_str())
                    .collect::<HashSet<_>>();
                let actual = probs
                    .iter()
                    .map(|score| score.label.as_str())
                    .collect::<HashSet<_>>();
                if probs.len() != options.len()
                    || actual.len() != probs.len()
                    || actual != expected
                    || !expected.contains(selected.as_str())
                    || probs.first().is_none_or(|score| score.label != *selected)
                    || !probs
                        .iter()
                        .zip(probs.iter().skip(1))
                        .all(|(left, right)| left.probability >= right.probability)
                {
                    return Err(DecisionError::Model(
                        "worker returned mismatched choice labels".to_owned(),
                    ));
                }
            }
            (QuestionKind::Score { min, max }, Answer::Score { value, .. })
                if value >= min && value <= max => {}
            (QuestionKind::Noul { .. }, Answer::Noul { .. }) => {}
            _ => {
                return Err(DecisionError::Model(
                    "worker returned a mismatched answer kind or score".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
    async fn schema_bound_counts_derived_ordinal_labels() -> Result<(), Box<dyn std::error::Error>>
    {
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
    }
}
