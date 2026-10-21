//! System-One (fast classifier) primitives: Choice / Score / Noul questions and their answers.
//! Serde uses internally tagged enums (`"type": …`); the chatlog's `untagged` form is rejected as
//! ambiguous to decode.

use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::error::ParseError;

/// A probability: finite and within `0.0..=1.0` (NaN rejected), with a total order.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct Probability(f32);

impl Probability {
    /// Validates a probability.
    ///
    /// # Errors
    /// [`ParseError::InvalidProbability`] for NaN, infinities and values outside `0.0..=1.0`.
    pub fn new(value: f32) -> Result<Self, ParseError> {
        if value.is_finite() && (0.0..=1.0).contains(&value) {
            // -0.0 passes the range check but orders below 0.0 under `total_cmp`; store +0.0.
            Ok(Self(if value == 0.0 { 0.0 } else { value }))
        } else {
            Err(ParseError::InvalidProbability)
        }
    }

    /// The value.
    #[must_use]
    pub const fn get(self) -> f32 {
        self.0
    }
}

impl TryFrom<f32> for Probability {
    type Error = ParseError;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Probability> for f32 {
    fn from(value: Probability) -> Self {
        value.0
    }
}

impl PartialEq for Probability {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Probability {}

impl PartialOrd for Probability {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Probability {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }
}

impl Hash for Probability {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

impl fmt::Debug for Probability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Probability({})", self.0)
    }
}

macro_rules! short_name {
    ($(#[$doc:meta])* $name:ident, $what:literal, $max:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(Box<str>);

        impl $name {
            /// Longest accepted value in bytes.
            pub const MAX_LEN: usize = $max;

            /// Validates and wraps the value: non-empty, at most [`Self::MAX_LEN`] bytes, no
            /// control characters.
            ///
            /// # Errors
            /// [`ParseError::InvalidValue`] when a rule is violated.
            pub fn new(value: &str) -> Result<Self, ParseError> {
                if value.trim().is_empty() {
                    return Err(ParseError::InvalidValue { what: $what, reason: "empty" });
                }
                if value.len() > Self::MAX_LEN {
                    return Err(ParseError::InvalidValue { what: $what, reason: "too long" });
                }
                if value.chars().any(char::is_control) {
                    return Err(ParseError::InvalidValue { what: $what, reason: "contains a control character" });
                }
                Ok(Self(value.into()))
            }

            /// The text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = ParseError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(&value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0.into_string()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

short_name!(
    /// Identifier of a question inside a decision schema (e.g. `intent`, `needs_sandbox`).
    QuestionId,
    "question id",
    64
);
short_name!(
    /// A class label of a Choice question (e.g. `refactor`, `0`..`10` for ordinal scores).
    Label,
    "label",
    128
);

/// What is asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuestionKind {
    /// Pick one label out of `options`.
    Choice {
        /// Candidate labels, in schema order.
        options: Vec<Label>,
    },
    /// An integer score within `min..=max` (ordinal labels such as `"0"`..`"10"`).
    Score {
        /// Lowest score.
        min: i32,
        /// Highest score.
        max: i32,
    },
    /// A yes/no judgement of `statement` ("noul" in System One's vocabulary).
    Noul {
        /// The statement to judge.
        statement: String,
    },
}

/// One question of a decision schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// Stable id used to look the answer up.
    pub id: QuestionId,
    /// The question.
    pub kind: QuestionKind,
}

/// A label with its probability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelScore {
    /// The label.
    pub label: Label,
    /// Its probability.
    pub probability: Probability,
}

/// The engine's answer to one question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Answer {
    /// Answer to a Choice question.
    Choice {
        /// The top label.
        selected: Label,
        /// Every candidate with its probability, descending.
        probs: Vec<LabelScore>,
    },
    /// Answer to a Score question.
    Score {
        /// The score.
        value: i32,
        /// Confidence of the score.
        confidence: Probability,
    },
    /// Answer to a Noul question.
    Noul {
        /// Probability that the statement holds.
        yes: Probability,
    },
}

/// A question id with its answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionAnswer {
    /// The question answered.
    pub question: QuestionId,
    /// The answer.
    pub answer: Answer,
}

/// Result of one `DecisionEngine::decide` call (all questions of a schema, batched).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    /// Answers in schema order.
    pub answers: Vec<QuestionAnswer>,
    /// Engine latency in microseconds.
    pub latency_us: u64,
}

/// Text the questions are asked about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionInput {
    /// The primary text (already PII-masked by the caller).
    pub text: String,
    /// Optional surrounding context.
    pub context: Option<String>,
}
