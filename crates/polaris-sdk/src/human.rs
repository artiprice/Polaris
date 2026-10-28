//! Versioned, bounded contracts for human-in-the-loop requests.
//!
//! These documents are CAS payloads. Events contain only their identifiers and digests so the
//! event envelope remains small and `Copy`.

use core::fmt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::{self, Write};
use uuid::Uuid;

use crate::{
    error::ParseError,
    ids::{Cid, FrameId, SessionId},
};

/// Current human request document schema.
pub const HUMAN_REQUEST_SCHEMA_VERSION: u16 = 1;
/// Maximum number of questions in one request.
pub const MAX_HUMAN_QUESTIONS: usize = 8;
/// Maximum number of named choices in one choice question, excluding None of the above.
pub const MAX_HUMAN_OPTIONS: usize = 16;
/// Maximum serialized request document size.
pub const MAX_HUMAN_REQUEST_BYTES: usize = 256 * 1024;
/// Maximum UTF-8 byte length of one free-text response or one notes field.
pub const MAX_HUMAN_TEXT_BYTES: usize = 16 * 1024;
/// Maximum serialized answer document size, including its envelope and JSON encoding.
pub const MAX_HUMAN_ANSWER_BYTES: usize = 128 * 1024;

/// Stable UUID identity for one human request, independent of S1 question identifiers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HumanRequestId(Uuid);

impl HumanRequestId {
    /// Generate a time-ordered request identifier.
    #[must_use]
    pub fn new_v7() -> Self {
        Self(Uuid::now_v7())
    }
    /// Construct an identifier from raw bits for deterministic replay and tests.
    #[must_use]
    pub const fn from_u128(value: u128) -> Self {
        Self(Uuid::from_u128(value))
    }
    /// Access the underlying UUID for wire conversion.
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl fmt::Debug for HumanRequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HumanRequestId({})", self.0)
    }
}
impl fmt::Display for HumanRequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// Stable identifier for a question inside one human request.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HumanQuestionId(Box<str>);
impl HumanQuestionId {
    /// Validate and create a question id.
    /// # Errors
    /// Returns `InvalidValue` for empty, oversized, whitespace or non-printable identifiers.
    pub fn new(value: &str) -> Result<Self, ParseError> {
        validate_key("human question id", value).map(|()| Self(value.into()))
    }
    /// Borrow the stable identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for HumanQuestionId {
    type Error = ParseError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}
impl From<HumanQuestionId> for String {
    fn from(value: HumanQuestionId) -> Self {
        value.0.into_string()
    }
}

/// Stable identifier for a named option in a choice question.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HumanOptionId(Box<str>);
impl HumanOptionId {
    /// Validate and create an option id.
    /// # Errors
    /// Returns `InvalidValue` for empty, oversized, whitespace or non-printable identifiers.
    pub fn new(value: &str) -> Result<Self, ParseError> {
        validate_key("human option id", value).map(|()| Self(value.into()))
    }
    /// Borrow the stable identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for HumanOptionId {
    type Error = ParseError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}
impl From<HumanOptionId> for String {
    fn from(value: HumanOptionId) -> Self {
        value.0.into_string()
    }
}

fn validate_key(what: &'static str, value: &str) -> Result<(), ParseError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|c| c.is_ascii_graphic() && !matches!(c, '/' | '\\'))
    {
        return Err(ParseError::InvalidValue {
            what,
            reason: "must be 1..=128 printable ASCII bytes without slash",
        });
    }
    Ok(())
}

/// A named choice with a stable machine-readable id and untrusted display label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanOption {
    /// Stable option identity.
    pub id: HumanOptionId,
    /// Display label, bounded by the request document limit.
    pub label: String,
}

/// Human-input question kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HumanQuestionKind {
    /// Select exactly one named option or the typed None-of-the-above choice.
    SingleChoice { options: Vec<HumanOption> },
    /// Select one or more named options, or explicitly select None of the above.
    MultiChoice { options: Vec<HumanOption> },
    /// Enter bounded UTF-8 text.
    FreeText,
}

/// One human-input question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanQuestion {
    /// Stable identifier within this request.
    pub id: HumanQuestionId,
    /// Prompt text, treated as untrusted presentation data.
    pub prompt: String,
    /// Whether an explicit unanswered response is rejected.
    pub required: bool,
    /// Input kind and any named choices.
    pub kind: HumanQuestionKind,
}

/// Complete immutable request content stored in CAS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanRequestPayload {
    /// Human-request schema version.
    pub schema: u16,
    /// Stable request identity.
    pub request_id: HumanRequestId,
    /// Owning session.
    pub session: SessionId,
    /// The frame waiting for this request.
    pub frame: FrameId,
    /// Originating frame step/turn.
    pub origin_step: u32,
    /// All questions, with required flags and stable option ids.
    pub questions: Vec<HumanQuestion>,
}

impl HumanRequestPayload {
    /// Build and validate a request document.
    /// # Errors
    /// Rejects zero or excess questions, duplicate ids/options, invalid kinds and oversized data.
    pub fn new(
        request_id: HumanRequestId,
        session: SessionId,
        frame: FrameId,
        origin_step: u32,
        questions: Vec<HumanQuestion>,
    ) -> Result<Self, HumanContractError> {
        if questions.is_empty() || questions.len() > MAX_HUMAN_QUESTIONS {
            return Err(HumanContractError::Invalid("question count"));
        }
        let payload = Self {
            schema: HUMAN_REQUEST_SCHEMA_VERSION,
            request_id,
            session,
            frame,
            origin_step,
            questions,
        };
        payload.validate()?;
        Ok(payload)
    }
    /// Revalidate a deserialized or recovered request document and enforce its encoded byte limit.
    /// # Errors
    /// Returns a typed invalid, unsupported-schema or too-large error.
    pub fn validate(&self) -> Result<(), HumanContractError> {
        if self.schema != HUMAN_REQUEST_SCHEMA_VERSION {
            return Err(HumanContractError::UnsupportedSchema(self.schema));
        }
        if self.questions.is_empty() || self.questions.len() > MAX_HUMAN_QUESTIONS {
            return Err(HumanContractError::Invalid("question count"));
        }
        let mut question_ids = BTreeSet::new();
        for question in &self.questions {
            if !question_ids.insert(question.id.clone()) {
                return Err(HumanContractError::Invalid("duplicate question id"));
            }
            if question.prompt.trim().is_empty() || question.prompt.len() > 16 * 1024 {
                return Err(HumanContractError::Invalid("question prompt length"));
            }
            match &question.kind {
                HumanQuestionKind::FreeText => {}
                HumanQuestionKind::SingleChoice { options }
                | HumanQuestionKind::MultiChoice { options } => {
                    validate_options(options)?;
                }
            }
        }
        let _bytes = bounded_json(self, MAX_HUMAN_REQUEST_BYTES)?;
        Ok(())
    }
    /// Serialize canonically for CAS insertion; the returned CID is the immutable revision.
    /// # Errors
    /// Fails when the document is invalid or JSON serialization fails.
    pub fn encode(&self) -> Result<Vec<u8>, HumanContractError> {
        self.validate()?;
        bounded_json(self, MAX_HUMAN_REQUEST_BYTES)
    }
}

fn validate_options(options: &[HumanOption]) -> Result<(), HumanContractError> {
    if options.is_empty() || options.len() > MAX_HUMAN_OPTIONS {
        return Err(HumanContractError::Invalid("choice option count"));
    }
    let mut ids = BTreeSet::new();
    for option in options {
        if !ids.insert(option.id.clone()) {
            return Err(HumanContractError::Invalid("duplicate option id"));
        }
        if option.label.trim().is_empty() || option.label.len() > 4096 {
            return Err(HumanContractError::Invalid("option label length"));
        }
    }
    Ok(())
}

/// Explicit selection, including the exclusive None-of-the-above sentinel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "option", rename_all = "snake_case")]
pub enum HumanSelection {
    /// A named option from the immutable request.
    Option(HumanOptionId),
    /// Explicitly choose the built-in None of the above option.
    NoneOfTheAbove,
}

/// One question's submitted response. Optional questions still require an explicit response record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HumanResponse {
    /// One choice selection.
    Single { selection: HumanSelection },
    /// A non-empty choice set; NoneOfTheAbove must be the only selection.
    Multiple { selections: Vec<HumanSelection> },
    /// Free text, which may be empty only when the question is optional.
    FreeText { text: String },
    /// Explicitly unanswered; accepted only for optional questions.
    Unanswered,
}

/// One response plus optional per-question notes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanQuestionAnswer {
    /// Stable question identifier.
    pub question_id: HumanQuestionId,
    /// Explicit answer state.
    pub response: HumanResponse,
    /// Optional notes attached to this question.
    pub notes: Option<String>,
}

/// Atomic answer batch stored in CAS and bound to one immutable request revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumanAnswerPayload {
    /// Answer schema version.
    pub schema: u16,
    /// Request being answered.
    pub request_id: HumanRequestId,
    /// Immutable request revision (the request CAS digest).
    pub revision: Cid,
    /// Owning session.
    pub session: SessionId,
    /// Requesting frame.
    pub frame: FrameId,
    /// Exactly one explicit response record per request question.
    pub answers: Vec<HumanQuestionAnswer>,
}

impl HumanAnswerPayload {
    /// Build an answer and validate it against the immutable request.
    /// # Errors
    /// Rejects missing, duplicate, unknown, mistyped, oversized or required-unanswered fields.
    pub fn new(
        request: &HumanRequestPayload,
        revision: Cid,
        answers: Vec<HumanQuestionAnswer>,
    ) -> Result<Self, HumanContractError> {
        let payload = Self {
            schema: HUMAN_REQUEST_SCHEMA_VERSION,
            request_id: request.request_id,
            revision,
            session: request.session,
            frame: request.frame,
            answers,
        };
        payload.validate_for(request, revision)?;
        Ok(payload)
    }
    /// Validate this complete answer batch against its request and expected revision.
    /// # Errors
    /// Returns a typed validation, stale-revision, encoding or aggregate-size error.
    pub fn validate_for(
        &self,
        request: &HumanRequestPayload,
        expected_revision: Cid,
    ) -> Result<(), HumanContractError> {
        request.validate()?;
        if self.schema != HUMAN_REQUEST_SCHEMA_VERSION {
            return Err(HumanContractError::UnsupportedSchema(self.schema));
        }
        if self.request_id != request.request_id
            || self.session != request.session
            || self.frame != request.frame
        {
            return Err(HumanContractError::Invalid("request owner mismatch"));
        }
        if self.revision != expected_revision {
            return Err(HumanContractError::StaleRevision);
        }
        if self.answers.len() != request.questions.len() {
            return Err(HumanContractError::Invalid(
                "one explicit response is required per question",
            ));
        }
        let mut answers = BTreeSet::new();
        for answer in &self.answers {
            if !answers.insert(answer.question_id.clone()) {
                return Err(HumanContractError::Invalid("duplicate question answer"));
            }
            let question = request
                .questions
                .iter()
                .find(|question| question.id == answer.question_id)
                .ok_or(HumanContractError::Invalid("unknown question id"))?;
            if let Some(notes) = &answer.notes
                && notes.len() > MAX_HUMAN_TEXT_BYTES
            {
                return Err(HumanContractError::TooLarge {
                    limit: MAX_HUMAN_TEXT_BYTES,
                    size: notes.len(),
                });
            }
            validate_response(question, &answer.response)?;
        }
        for question in &request.questions {
            if !answers.contains(&question.id) {
                return Err(HumanContractError::Invalid("missing question answer"));
            }
        }
        let _bytes = bounded_json(self, MAX_HUMAN_ANSWER_BYTES)?;
        Ok(())
    }
    /// Serialize a validated answer batch for CAS storage.
    /// # Errors
    /// Fails when validation or JSON serialization fails.
    pub fn encode_for(
        &self,
        request: &HumanRequestPayload,
        expected_revision: Cid,
    ) -> Result<Vec<u8>, HumanContractError> {
        self.validate_for(request, expected_revision)?;
        bounded_json(self, MAX_HUMAN_ANSWER_BYTES)
    }
}

fn validate_response(
    question: &HumanQuestion,
    response: &HumanResponse,
) -> Result<(), HumanContractError> {
    match (&question.kind, response) {
        (_, HumanResponse::Unanswered) if question.required => {
            Err(HumanContractError::Invalid("required question unanswered"))
        }
        (_, HumanResponse::Unanswered) => Ok(()),
        (HumanQuestionKind::FreeText, HumanResponse::FreeText { text }) => {
            if text.len() > MAX_HUMAN_TEXT_BYTES {
                return Err(HumanContractError::TooLarge {
                    limit: MAX_HUMAN_TEXT_BYTES,
                    size: text.len(),
                });
            }
            if question.required && text.trim().is_empty() {
                return Err(HumanContractError::Invalid("required free text is empty"));
            }
            Ok(())
        }
        (HumanQuestionKind::SingleChoice { options }, HumanResponse::Single { selection }) => {
            validate_selection(selection, options)
        }
        (HumanQuestionKind::MultiChoice { options }, HumanResponse::Multiple { selections }) => {
            if selections.is_empty() {
                return Err(HumanContractError::Invalid(
                    "multi-choice response is empty",
                ));
            }
            let none_count = selections
                .iter()
                .filter(|selection| matches!(selection, HumanSelection::NoneOfTheAbove))
                .count();
            if none_count != 0 && selections.len() != 1 {
                return Err(HumanContractError::Invalid(
                    "NoneOfTheAbove must be exclusive",
                ));
            }
            let mut selected = BTreeSet::new();
            for selection in selections {
                if let HumanSelection::Option(id) = selection
                    && !selected.insert(id.clone())
                {
                    return Err(HumanContractError::Invalid("duplicate selected option"));
                }
                validate_selection(selection, options)?;
            }
            Ok(())
        }
        _ => Err(HumanContractError::Invalid(
            "response kind does not match question",
        )),
    }
}

fn validate_selection(
    selection: &HumanSelection,
    options: &[HumanOption],
) -> Result<(), HumanContractError> {
    match selection {
        HumanSelection::NoneOfTheAbove => Ok(()),
        HumanSelection::Option(id) if options.iter().any(|option| option.id == *id) => Ok(()),
        HumanSelection::Option(_) => Err(HumanContractError::Invalid("unknown option id")),
    }
}

fn bounded_json(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, HumanContractError> {
    let mut writer = BoundedJsonWriter {
        bytes: Vec::with_capacity(limit),
        limit,
        exceeded: None,
    };
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(writer.bytes),
        Err(_) => match writer.exceeded {
            Some(size) => Err(HumanContractError::TooLarge { limit, size }),
            None => Err(HumanContractError::Encoding),
        },
    }
}

struct BoundedJsonWriter {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: Option<usize>,
}

impl Write for BoundedJsonWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let Some(size) = self.bytes.len().checked_add(bytes.len()) else {
            return Err(io::Error::other("encoded payload size overflow"));
        };
        if size > self.limit {
            self.exceeded = Some(size);
            return Err(io::Error::other("encoded payload exceeds limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Validation failures for human-request and answer documents.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HumanContractError {
    /// A payload violates a structural or semantic rule.
    #[error("invalid human request or answer: {0}")]
    Invalid(&'static str),
    /// An encoded payload exceeds its aggregate byte ceiling.
    #[error("human payload too large ({size} > {limit})")]
    TooLarge {
        /// Configured maximum size.
        limit: usize,
        /// Actual encoded or field size.
        size: usize,
    },
    /// Payload schema is newer than this SDK supports.
    #[error("unsupported human request schema {0}")]
    UnsupportedSchema(u16),
    /// Answer revision differs from the immutable request digest.
    #[error("human request revision is stale")]
    StaleRevision,
    /// JSON serialization failed.
    #[error("human payload encoding failed")]
    Encoding,
}
