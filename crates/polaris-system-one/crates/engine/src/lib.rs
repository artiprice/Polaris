//! Real, local GLiNER2.5-Decide inference for the isolated `polaris-worker` process.
//!
//! The engine accepts explicit model and ONNX Runtime library paths. It never downloads a
//! checkpoint, searches a hub cache, or fabricates results. The engine requires
//! `GLINER2_DEVICE=cpu` because it is the only provider verified by spike S0-4. It pins standard
//! execution and fp32 independently of GLiNER execution-mode environment variables. Text
//! normalization is a required configuration choice:
//! raw text and the Python GLiNER2 2.1 period rule are distinct model inputs.

#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use gliner2_rs::{Entity, ExecutionMode, Precision, SchemaTask, SpanConfig, SpanEngine};
use polaris_sdk::system_one::{
    Answer, Decision, DecisionInput, Label, LabelScore, PiiEntitySpan, Question, QuestionAnswer,
    QuestionId, QuestionKind,
};

/// Explicit local model and runtime paths for independent semantic PII detection.
#[derive(Debug, Clone)]
pub struct PiiEngineConfig {
    model_directory: PathBuf,
    ort_library: PathBuf,
    intra_threads: usize,
    max_input_bytes: usize,
    max_spans: usize,
}

impl PiiEngineConfig {
    /// Creates a separate PII companion-model config with conservative local bounds.
    #[must_use]
    pub fn new(model_directory: PathBuf, ort_library: PathBuf) -> Self {
        Self {
            model_directory,
            ort_library,
            intra_threads: 2,
            max_input_bytes: polaris_proto::limits::MAX_PII_INPUT_BYTES,
            max_spans: 4_096,
        }
    }

    /// Sets the ONNX intra-op thread count; zero is rejected on load.
    #[must_use]
    pub const fn with_intra_threads(mut self, threads: usize) -> Self {
        self.intra_threads = threads;
        self
    }

    /// Sets the maximum exact raw UTF-8 input size.
    #[must_use]
    pub const fn with_max_input_bytes(mut self, max_bytes: usize) -> Self {
        self.max_input_bytes = max_bytes;
        self
    }

    /// Sets the maximum candidate spans returned per request.
    #[must_use]
    pub const fn with_max_spans(mut self, max_spans: usize) -> Self {
        self.max_spans = max_spans;
        self
    }
}

/// Independent loaded GLiNER2 privacy companion model. It never shares the Decide model handle.
pub struct GlinerPiiEngine {
    model: Mutex<SpanEngine>,
    max_input_bytes: usize,
    max_spans: usize,
    model_identity_blake3: [u8; 32],
}

impl std::fmt::Debug for GlinerPiiEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GlinerPiiEngine")
            .field("max_input_bytes", &self.max_input_bytes)
            .field("max_spans", &self.max_spans)
            .finish_non_exhaustive()
    }
}

impl GlinerPiiEngine {
    /// Loads a separately configured fp32 PII companion checkpoint from explicit local paths.
    ///
    /// # Errors
    /// Returns [`EngineError`] for missing or changing model/runtime artifacts or native load
    /// failures. It does not search caches or download model files.
    pub fn load(config: PiiEngineConfig) -> Result<Self, EngineError> {
        if config.intra_threads == 0
            || config.max_input_bytes == 0
            || config.max_input_bytes > polaris_proto::limits::MAX_PII_INPUT_BYTES
            || config.max_spans == 0
            || config.max_spans > 4_096
        {
            return Err(EngineError::InvalidConfig(
                "PII engine bounds must be within the protocol caps".to_owned(),
            ));
        }
        validate_cpu_device(std::env::var("GLINER2_DEVICE").ok().as_deref())?;
        let model_directory = canonical_file_or_directory(&config.model_directory, true)?;
        let ort_library = canonical_file_or_directory(&config.ort_library, false)?;
        check_fp32_model_files(&model_directory)?;
        initialize_runtime(&ort_library)?;
        let before_load =
            hash_pii_engine_inputs(&model_directory, &ort_library, config.intra_threads)?;
        let span_config = SpanConfig::new(&model_directory)
            .with_precision(Precision::Fp32)
            .with_intra_threads(config.intra_threads)
            .with_execution(ExecutionMode::Standard);
        let model = SpanEngine::new(span_config)
            .map_err(|error| EngineError::Inference(error.to_string()))?;
        let after_load =
            hash_pii_engine_inputs(&model_directory, &ort_library, config.intra_threads)?;
        if before_load != after_load {
            return Err(EngineError::InvalidConfig(
                "PII model inputs changed while the native engine was loading".to_owned(),
            ));
        }
        Ok(Self {
            model: Mutex::new(model),
            max_input_bytes: config.max_input_bytes,
            max_spans: config.max_spans,
            model_identity_blake3: after_load,
        })
    }

    /// BLAKE3 identity of the independent PII checkpoint, runtime and thread configuration.
    #[must_use]
    pub const fn model_identity_blake3(&self) -> [u8; 32] {
        self.model_identity_blake3
    }

    /// Detects semantic PII using the companion model on exact unnormalized input.
    ///
    /// # Errors
    /// Returns an error on an oversized request, native inference failure, malformed output, or a
    /// poisoned native session lock. No entity text or labels are returned.
    pub fn detect(&self, raw_text: &str) -> Result<Vec<PiiEntitySpan>, EngineError> {
        if raw_text.len() > self.max_input_bytes {
            return Err(EngineError::InputTooLarge {
                actual_bytes: raw_text.len(),
                limit_bytes: self.max_input_bytes,
            });
        }
        let tasks = gliner2_rs::privacy::Group::ALL
            .iter()
            .map(|group| group.task())
            .collect::<Vec<_>>();
        let mut model = self.model.lock().map_err(|_| EngineError::EnginePoisoned)?;
        let output = model
            .extract_long(raw_text, &tasks)
            .map_err(|error| EngineError::Inference(error.to_string()))?;
        checked_pii_spans(raw_text, &output.entities, self.max_spans)
    }
}

/// Text preprocessing selected by the worker configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextNormalization {
    /// Pass model text unchanged.
    Raw,
    /// Match `gliner2` 2.1.0 by appending `.` unless the text ends in `.`, `!` or `?`.
    Gliner2Python21,
}

/// Explicit local paths and bounds for a CPU fp32 GLiNER engine.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    model_directory: PathBuf,
    ort_library: PathBuf,
    normalization: TextNormalization,
    intra_threads: usize,
    max_input_bytes: usize,
}

impl EngineConfig {
    /// Creates a config with explicit model/runtime paths and the selected text contract.
    #[must_use]
    pub fn new(
        model_directory: PathBuf,
        ort_library: PathBuf,
        normalization: TextNormalization,
    ) -> Self {
        Self {
            model_directory,
            ort_library,
            normalization,
            intra_threads: 4,
            max_input_bytes: 262_144,
        }
    }

    /// Sets the ONNX intra-op thread count. Zero is rejected during engine loading.
    #[must_use]
    pub const fn with_intra_threads(mut self, intra_threads: usize) -> Self {
        self.intra_threads = intra_threads;
        self
    }

    /// Sets the maximum combined UTF-8 byte length of primary text and context.
    #[must_use]
    pub const fn with_max_input_bytes(mut self, max_input_bytes: usize) -> Self {
        self.max_input_bytes = max_input_bytes;
        self
    }
}

/// Failure to load or run the configured local inference engine.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// A required model artifact or runtime library is absent.
    #[error("required local file is missing: {0}")]
    FileMissing(PathBuf),
    /// A configured path could not be canonicalized.
    #[error("invalid configured path {path}: {message}")]
    InvalidPath { path: PathBuf, message: String },
    /// The ONNX Runtime dynamic library or environment could not be initialized.
    #[error("ONNX Runtime initialization failed: {0}")]
    RuntimeInitialization(String),
    /// Another ONNX Runtime path was configured earlier in this process.
    #[error("ONNX Runtime was already initialized with a different library path")]
    RuntimePathConflict,
    /// Runtime initialization happened outside this engine before the configured path was used.
    #[error("ONNX Runtime was initialized before the configured engine runtime")]
    RuntimeAlreadyInitialized,
    /// The engine config contains an invalid bound or thread count.
    #[error("invalid engine configuration: {0}")]
    InvalidConfig(String),
    /// The input question schema is invalid for classification.
    #[error("invalid decision schema: {0}")]
    InvalidSchema(String),
    /// The input is over the configured byte limit.
    #[error("decision input is {actual_bytes} bytes; the configured limit is {limit_bytes}")]
    InputTooLarge {
        /// Observed combined input size.
        actual_bytes: usize,
        /// Configured maximum input size.
        limit_bytes: usize,
    },
    /// The model did not return a complete, valid set of classification probabilities.
    #[error("invalid model output: {0}")]
    InvalidOutput(String),
    /// A GLiNER/ONNX inference operation failed.
    #[error("inference failed: {0}")]
    Inference(String),
    /// The model lock was poisoned by a failed inference thread.
    #[error("inference engine lock is poisoned")]
    EnginePoisoned,
}

/// One loaded local fp32 classifier. Native inference calls serialize through a mutex because
/// `gliner2-rs` uses mutable ONNX sessions; worker code must move calls to a blocking thread pool.
pub struct GlinerEngine {
    model: Mutex<SpanEngine>,
    normalization: TextNormalization,
    max_input_bytes: usize,
    model_identity_blake3: [u8; 32],
}

impl std::fmt::Debug for GlinerEngine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GlinerEngine")
            .field("normalization", &self.normalization)
            .field("max_input_bytes", &self.max_input_bytes)
            .finish_non_exhaustive()
    }
}

impl GlinerEngine {
    /// Loads only the exact local fp32 model and ONNX Runtime files in `config`.
    ///
    /// # Errors
    /// Returns [`EngineError`] for any missing, invalid, incompatible, or unloadable file.
    pub fn load(config: EngineConfig) -> Result<Self, EngineError> {
        if config.intra_threads == 0 || config.max_input_bytes == 0 {
            return Err(EngineError::InvalidConfig(
                "thread count and input-byte limit must be non-zero".to_owned(),
            ));
        }
        validate_cpu_device(std::env::var("GLINER2_DEVICE").ok().as_deref())?;
        let model_directory = canonical_file_or_directory(&config.model_directory, true)?;
        let ort_library = canonical_file_or_directory(&config.ort_library, false)?;
        check_fp32_model_files(&model_directory)?;
        initialize_runtime(&ort_library)?;
        let before_load = hash_engine_inputs(
            &model_directory,
            &ort_library,
            config.normalization,
            config.intra_threads,
        )?;

        let span_config = SpanConfig::new(&model_directory)
            .with_precision(Precision::Fp32)
            .with_intra_threads(config.intra_threads)
            .with_execution(ExecutionMode::Standard);
        let model = SpanEngine::new(span_config)
            .map_err(|error| EngineError::Inference(error.to_string()))?;
        let after_load = hash_engine_inputs(
            &model_directory,
            &ort_library,
            config.normalization,
            config.intra_threads,
        )?;
        if before_load != after_load {
            return Err(EngineError::InvalidConfig(
                "model inputs changed while the native engine was loading".to_owned(),
            ));
        }
        Ok(Self {
            model: Mutex::new(model),
            normalization: config.normalization,
            max_input_bytes: config.max_input_bytes,
            model_identity_blake3: after_load,
        })
    }

    /// BLAKE3 identity of the exact ONNX/tokenizer/runtime inputs and inference configuration.
    #[must_use]
    pub const fn model_identity_blake3(&self) -> [u8; 32] {
        self.model_identity_blake3
    }

    /// Runs one multi-question classification request using the configured local model.
    ///
    /// # Errors
    /// Returns [`EngineError`] when bounds, questions, inference, or model output are invalid.
    pub fn decide(
        &self,
        questions: &[Question],
        input: &DecisionInput,
    ) -> Result<Decision, EngineError> {
        let started = Instant::now();
        let task_plans = build_task_plans(questions)?;
        let text = prepare_text(input, self.max_input_bytes, self.normalization)?;

        let schema = task_plans
            .iter()
            .map(|plan| SchemaTask::classification(plan.prompt.clone(), plan.labels.clone()))
            .collect::<Vec<_>>();
        let mut model = self.model.lock().map_err(|_| EngineError::EnginePoisoned)?;
        let output = model
            .extract(&text, &schema)
            .map_err(|error| EngineError::Inference(error.to_string()))?;
        let mut answers = Vec::with_capacity(task_plans.len());
        for plan in &task_plans {
            answers.push(QuestionAnswer {
                question: plan.question_id.clone(),
                answer: answer_for(plan, &output.classifications)?,
            });
        }
        let elapsed_micros = started.elapsed().as_micros();
        let latency_us = if elapsed_micros > u128::from(u64::MAX) {
            u64::MAX
        } else {
            match u64::try_from(elapsed_micros) {
                Ok(value) => value,
                Err(_) => {
                    return Err(EngineError::InvalidOutput(
                        "inference latency conversion failed".to_owned(),
                    ));
                }
            }
        };
        Ok(Decision {
            answers,
            latency_us,
        })
    }
}

fn hash_engine_inputs(
    model_directory: &Path,
    ort_library: &Path,
    normalization: TextNormalization,
    intra_threads: usize,
) -> Result<[u8; 32], EngineError> {
    const MODEL_FILES: [&str; 9] = [
        "classifier_fp32.onnx",
        "count_lstm_fixed_fp32.onnx",
        "count_pred_argmax_fp32.onnx",
        "encoder_fp32.onnx",
        "schema_gather_fp32.onnx",
        "scorer_fp32.onnx",
        "span_rep_fp32.onnx",
        "token_gather_fp32.onnx",
        "tokenizer.json",
    ];
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"polaris-s1-engine-inputs-v1\0");
    hasher.update(match normalization {
        TextNormalization::Raw => b"raw\0",
        TextNormalization::Gliner2Python21 => b"gliner2-python-2.1\0",
    });
    let thread_bytes = u64::try_from(intra_threads)
        .map_err(|_| EngineError::InvalidConfig("thread count is not representable".to_owned()))?
        .to_le_bytes();
    hasher.update(&thread_bytes);
    for name in MODEL_FILES {
        hasher.update(name.as_bytes());
        hasher.update(b"\0");
        hash_file_into(&model_directory.join(name), &mut hasher)?;
    }
    hasher.update(b"onnxruntime-library\0");
    hash_file_into(ort_library, &mut hasher)?;
    Ok(*hasher.finalize().as_bytes())
}

fn hash_pii_engine_inputs(
    model_directory: &Path,
    ort_library: &Path,
    intra_threads: usize,
) -> Result<[u8; 32], EngineError> {
    const MODEL_FILES: [&str; 9] = [
        "classifier_fp32.onnx",
        "count_lstm_fixed_fp32.onnx",
        "count_pred_argmax_fp32.onnx",
        "encoder_fp32.onnx",
        "schema_gather_fp32.onnx",
        "scorer_fp32.onnx",
        "span_rep_fp32.onnx",
        "token_gather_fp32.onnx",
        "tokenizer.json",
    ];
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"polaris-s1-pii-companion-engine-v1\0");
    hasher.update(b"raw-utf8-input\0all-seven-pii-groups\0threshold-0.5\0");
    let thread_bytes = u64::try_from(intra_threads)
        .map_err(|_| {
            EngineError::InvalidConfig("PII thread count is not representable".to_owned())
        })?
        .to_le_bytes();
    hasher.update(&thread_bytes);
    for name in MODEL_FILES {
        hasher.update(name.as_bytes());
        hasher.update(b"\0");
        hash_file_into(&model_directory.join(name), &mut hasher)?;
    }
    hasher.update(b"onnxruntime-library\0");
    hash_file_into(ort_library, &mut hasher)?;
    Ok(*hasher.finalize().as_bytes())
}

fn checked_pii_spans(
    text: &str,
    entities: &[Entity],
    max_spans: usize,
) -> Result<Vec<PiiEntitySpan>, EngineError> {
    if entities.len() > max_spans {
        return Err(EngineError::InvalidOutput(
            "PII model returned more spans than the configured limit".to_owned(),
        ));
    }
    let mut ranked = Vec::new();
    ranked
        .try_reserve(entities.len())
        .map_err(|_| EngineError::InvalidOutput("PII span allocation failed".to_owned()))?;
    for entity in entities {
        if !entity.score.is_finite()
            || !(0.0..=1.0).contains(&entity.score)
            || entity.char_start >= entity.char_end
            || entity.char_end > text.len()
            || text.get(entity.char_start..entity.char_end).is_none()
        {
            return Err(EngineError::InvalidOutput(
                "PII model returned an invalid UTF-8 byte span".to_owned(),
            ));
        }
        let start = u32::try_from(entity.char_start).map_err(|_| {
            EngineError::InvalidOutput("PII span offset exceeds protocol".to_owned())
        })?;
        let end = u32::try_from(entity.char_end).map_err(|_| {
            EngineError::InvalidOutput("PII span offset exceeds protocol".to_owned())
        })?;
        ranked.push((start, end, entity.score));
    }
    ranked.sort_by(|left, right| {
        right
            .2
            .total_cmp(&left.2)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| left.0.cmp(&right.0))
    });
    let mut accepted = Vec::<(u32, u32, f32)>::new();
    accepted
        .try_reserve(ranked.len())
        .map_err(|_| EngineError::InvalidOutput("PII span allocation failed".to_owned()))?;
    for candidate in ranked {
        let overlaps = accepted
            .iter()
            .any(|current| candidate.0 < current.1 && current.0 < candidate.1);
        if !overlaps {
            accepted.push(candidate);
        }
    }
    accepted.sort_by_key(|candidate| (candidate.0, candidate.1));
    accepted
        .into_iter()
        .map(|(start, end, confidence)| {
            PiiEntitySpan::new(start, end, confidence).map_err(|error| {
                EngineError::InvalidOutput(format!("PII span validation failed: {error}"))
            })
        })
        .collect()
}

fn hash_file_into(path: &Path, hasher: &mut blake3::Hasher) -> Result<(), EngineError> {
    let metadata = std::fs::metadata(path).map_err(|error| EngineError::InvalidPath {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if !metadata.is_file() {
        return Err(EngineError::FileMissing(path.to_path_buf()));
    }
    hasher.update(&metadata.len().to_le_bytes());
    let mut file = std::fs::File::open(path).map_err(|error| EngineError::InvalidPath {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| EngineError::InvalidPath {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
        if read == 0 {
            break;
        }
        let chunk = buffer.get(..read).ok_or_else(|| {
            EngineError::InvalidConfig(
                "model digest read length exceeded its fixed buffer".to_owned(),
            )
        })?;
        hasher.update(chunk);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnswerKind {
    Choice,
    Score,
    Noul,
}

#[derive(Debug)]
struct TaskPlan {
    question_id: QuestionId,
    prompt: String,
    labels: Vec<String>,
    answer_kind: AnswerKind,
}

fn validate_cpu_device(device: Option<&str>) -> Result<(), EngineError> {
    if device == Some("cpu") {
        Ok(())
    } else {
        Err(EngineError::InvalidConfig(
            "GLINER2_DEVICE must be explicitly set to cpu".to_owned(),
        ))
    }
}

fn canonical_file_or_directory(path: &Path, directory: bool) -> Result<PathBuf, EngineError> {
    let canonical = path
        .canonicalize()
        .map_err(|error| EngineError::InvalidPath {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    let valid_kind = if directory {
        canonical.is_dir()
    } else {
        canonical.is_file()
    };
    if !valid_kind {
        return Err(EngineError::FileMissing(canonical));
    }
    Ok(canonical)
}

fn check_fp32_model_files(directory: &Path) -> Result<(), EngineError> {
    const FRAGMENTS: [&str; 8] = [
        "classifier",
        "count_lstm_fixed",
        "count_pred_argmax",
        "encoder",
        "schema_gather",
        "scorer",
        "span_rep",
        "token_gather",
    ];
    for fragment in FRAGMENTS {
        let filename = format!("{fragment}_fp32.onnx");
        let path = directory.join(filename);
        if !path.is_file() {
            return Err(EngineError::FileMissing(path));
        }
    }
    let tokenizer = directory.join("tokenizer.json");
    if !tokenizer.is_file() {
        return Err(EngineError::FileMissing(tokenizer));
    }
    Ok(())
}

fn initialize_runtime(path: &Path) -> Result<(), EngineError> {
    static RUNTIME: OnceLock<(PathBuf, Result<(), String>)> = OnceLock::new();
    let (configured_path, initialized) = RUNTIME.get_or_init(|| {
        let result = match ort::init_from(path) {
            Ok(builder) => {
                if builder.with_name("polaris-s1".to_owned()).commit() {
                    Ok(())
                } else {
                    Err(
                        "runtime environment already existed before explicit initialization"
                            .to_owned(),
                    )
                }
            }
            Err(error) => Err(error.to_string()),
        };
        (path.to_path_buf(), result)
    });
    if configured_path != path {
        return Err(EngineError::RuntimePathConflict);
    }
    match initialized {
        Ok(()) => Ok(()),
        Err(message) if message.starts_with("runtime environment already existed") => {
            Err(EngineError::RuntimeAlreadyInitialized)
        }
        Err(message) => Err(EngineError::RuntimeInitialization(message.clone())),
    }
}

fn combine_input(input: &DecisionInput, limit: usize) -> Result<String, EngineError> {
    let separator_bytes = if input.context.is_some() {
        2_usize
    } else {
        0_usize
    };
    let context_bytes = match &input.context {
        Some(context) => context.len(),
        None => 0,
    };
    let actual_bytes = input
        .text
        .len()
        .checked_add(separator_bytes)
        .and_then(|size| size.checked_add(context_bytes))
        .ok_or(EngineError::InputTooLarge {
            actual_bytes: usize::MAX,
            limit_bytes: limit,
        })?;
    if actual_bytes > limit {
        return Err(EngineError::InputTooLarge {
            actual_bytes,
            limit_bytes: limit,
        });
    }
    let mut combined = input.text.clone();
    if let Some(context) = &input.context {
        combined.push_str("\n\n");
        combined.push_str(context);
    }
    Ok(combined)
}

fn prepare_text(
    input: &DecisionInput,
    limit: usize,
    normalization: TextNormalization,
) -> Result<String, EngineError> {
    let mut text = combine_input(input, limit)?;
    if normalization == TextNormalization::Gliner2Python21 && !text.ends_with(['.', '!', '?']) {
        let normalized_size = text
            .len()
            .checked_add(1)
            .ok_or(EngineError::InputTooLarge {
                actual_bytes: usize::MAX,
                limit_bytes: limit,
            })?;
        if normalized_size > limit {
            return Err(EngineError::InputTooLarge {
                actual_bytes: normalized_size,
                limit_bytes: limit,
            });
        }
        text.push('.');
    }
    Ok(text)
}

fn build_task_plans(questions: &[Question]) -> Result<Vec<TaskPlan>, EngineError> {
    if questions.is_empty() || questions.len() > 64 {
        return Err(EngineError::InvalidSchema(
            "a request must contain between 1 and 64 questions".to_owned(),
        ));
    }
    const MAX_SCHEMA_BYTES: usize = 65_536;
    const MAX_PROMPT_BYTES: usize = 4_096;
    let mut prompts = HashSet::with_capacity(questions.len());
    let mut question_ids = HashSet::with_capacity(questions.len());
    let mut plans = Vec::with_capacity(questions.len());
    let mut schema_bytes = 0_usize;
    for question in questions {
        if !question_ids.insert(question.id.as_str()) {
            return Err(EngineError::InvalidSchema(
                "question IDs must be unique within one decision".to_owned(),
            ));
        }
        let (prompt, labels, answer_kind) = match &question.kind {
            QuestionKind::Choice { options } => {
                if options.is_empty() || options.len() > 128 {
                    return Err(EngineError::InvalidSchema(
                        "choice questions must contain 1 to 128 options".to_owned(),
                    ));
                }
                let mut unique_labels = HashSet::with_capacity(options.len());
                if options
                    .iter()
                    .any(|label| !unique_labels.insert(label.as_str()))
                {
                    return Err(EngineError::InvalidSchema(
                        "choice labels must be unique within each question".to_owned(),
                    ));
                }
                (
                    question.id.as_str().to_owned(),
                    options
                        .iter()
                        .map(|label| label.as_str().to_owned())
                        .collect::<Vec<_>>(),
                    AnswerKind::Choice,
                )
            }
            QuestionKind::Score { min, max } => {
                let width = i64::from(*max).checked_sub(i64::from(*min));
                if min > max || !matches!(width, Some(0..=256)) {
                    return Err(EngineError::InvalidSchema(
                        "score range must be ordered and span at most 256".to_owned(),
                    ));
                }
                let mut labels = Vec::new();
                for score in *min..=*max {
                    labels.push(score.to_string());
                }
                (question.id.as_str().to_owned(), labels, AnswerKind::Score)
            }
            QuestionKind::Noul { statement } => {
                if statement.trim().is_empty() || statement.len() > MAX_PROMPT_BYTES {
                    return Err(EngineError::InvalidSchema(
                        "noul statement must be non-empty and at most 4096 bytes".to_owned(),
                    ));
                }
                (
                    statement.clone(),
                    vec!["yes".to_owned(), "no".to_owned()],
                    AnswerKind::Noul,
                )
            }
        };
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(EngineError::InvalidSchema(
                "model prompt must be at most 4096 bytes".to_owned(),
            ));
        }
        let question_bytes = question.id.as_str().len();
        schema_bytes = schema_bytes
            .checked_add(question_bytes)
            .and_then(|size| size.checked_add(prompt.len()))
            .and_then(|size| {
                labels
                    .iter()
                    .try_fold(size, |total, label| total.checked_add(label.len()))
            })
            .ok_or_else(|| EngineError::InvalidSchema("schema byte size overflow".to_owned()))?;
        if schema_bytes > MAX_SCHEMA_BYTES {
            return Err(EngineError::InvalidSchema(
                "aggregate question schema exceeds 65536 bytes".to_owned(),
            ));
        }
        if !prompts.insert(prompt.clone()) {
            return Err(EngineError::InvalidSchema(
                "question prompts must be unique within one model call".to_owned(),
            ));
        }
        plans.push(TaskPlan {
            question_id: question.id.clone(),
            prompt,
            labels,
            answer_kind,
        });
    }
    Ok(plans)
}

fn answer_for(
    plan: &TaskPlan,
    classifications: &[gliner2_rs::Classification],
) -> Result<Answer, EngineError> {
    let mut scores = Vec::with_capacity(plan.labels.len());
    for label in &plan.labels {
        let mut matched = None;
        for classification in classifications.iter().filter(|classification| {
            classification.task == plan.prompt && classification.label == *label
        }) {
            if matched.is_some() {
                return Err(EngineError::InvalidOutput(
                    "model returned a duplicate task label".to_owned(),
                ));
            }
            matched = Some(classification.score);
        }
        let score = matched.ok_or_else(|| {
            EngineError::InvalidOutput(format!(
                "model omitted configured label `{label}` for question `{}`",
                plan.question_id.as_str()
            ))
        })?;
        scores.push(LabelScore {
            label: Label::new(label)
                .map_err(|error| EngineError::InvalidOutput(error.to_string()))?,
            probability: polaris_sdk::system_one::Probability::new(score)
                .map_err(|error| EngineError::InvalidOutput(error.to_string()))?,
        });
    }
    scores.sort_by_key(|score| std::cmp::Reverse(score.probability));
    let top = scores
        .first()
        .ok_or_else(|| EngineError::InvalidOutput("model returned no labels".to_owned()))?;
    let top_label = top.label.clone();
    let top_probability = top.probability;
    match plan.answer_kind {
        AnswerKind::Choice => Ok(Answer::Choice {
            selected: top_label,
            probs: scores,
        }),
        AnswerKind::Score => {
            let value = top_label.as_str().parse::<i32>().map_err(|error| {
                EngineError::InvalidOutput(format!("invalid ordinal model label: {error}"))
            })?;
            Ok(Answer::Score {
                value,
                confidence: top_probability,
            })
        }
        AnswerKind::Noul => {
            let yes = scores
                .iter()
                .find(|score| score.label.as_str() == "yes")
                .ok_or_else(|| {
                    EngineError::InvalidOutput("model omitted the `yes` probability".to_owned())
                })?;
            Ok(Answer::Noul {
                yes: yes.probability,
            })
        }
    }
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-system-one/crates/engine/src/lib.rs"]
mod tests;
