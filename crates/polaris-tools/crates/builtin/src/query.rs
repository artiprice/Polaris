//! Pure JSON Pointer and regular-expression query helpers.

use polaris_security_automata::{UserRegexLimits, validate_user_regex};
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Input for selecting a JSON Pointer from caller-provided JSON text.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct JsonQueryInput {
    /// JSON document text.
    pub json: String,
    /// RFC 6901 JSON Pointer (empty string selects the root value).
    #[serde(default)]
    pub pointer: String,
}

/// Selected JSON value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JsonQueryOutput {
    /// Selected value.
    pub value: Value,
}

/// Input for matching a bounded regex against caller-provided text.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RegexQueryInput {
    /// Input text, bounded by the registry's max input size.
    pub text: String,
    /// Regex pattern checked by the shared automata guard.
    pub pattern: String,
    /// Case-insensitive matching.
    #[serde(default)]
    pub case_insensitive: bool,
}

/// Bounded list of regex matches with byte offsets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegexQueryOutput {
    /// Matched text and half-open UTF-8 byte ranges.
    pub matches: Vec<RegexMatch>,
    /// True when additional matches were omitted by the limit.
    pub truncated: bool,
}

/// One regex match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegexMatch {
    /// Matched text.
    pub text: String,
    /// Start byte in the original input.
    pub start: usize,
    /// End byte in the original input.
    pub end: usize,
}

/// Select a JSON Pointer value from JSON text.
pub fn json_query(
    input: &JsonQueryInput,
) -> Result<JsonQueryOutput, polaris_sdk::registry::ToolError> {
    let document: Value = serde_json::from_str(&input.json).map_err(|_| {
        polaris_sdk::registry::ToolError::InvalidInput("invalid JSON document".to_owned())
    })?;
    let value = document
        .pointer(&input.pointer)
        .cloned()
        .ok_or_else(|| polaris_sdk::registry::ToolError::NotFound("JSON pointer".to_owned()))?;
    Ok(JsonQueryOutput { value })
}

/// Find regex matches in bounded input text.
pub fn regex_query(
    input: &RegexQueryInput,
    max_results: usize,
    regex_size_limit: usize,
) -> Result<RegexQueryOutput, polaris_sdk::registry::ToolError> {
    if max_results == 0 || regex_size_limit == 0 {
        return Err(polaris_sdk::registry::ToolError::InvalidInput(
            "query limits must be positive".to_owned(),
        ));
    }
    let validated = validate_user_regex(
        &input.pattern,
        &UserRegexLimits {
            nfa_size_bytes: regex_size_limit,
            ..UserRegexLimits::default()
        },
    )
    .map_err(|_| {
        polaris_sdk::registry::ToolError::InvalidInput("regex rejected by safety limits".to_owned())
    })?;
    let regex = RegexBuilder::new(validated.as_str())
        .case_insensitive(input.case_insensitive)
        .size_limit(regex_size_limit)
        .build()
        .map_err(|_| {
            polaris_sdk::registry::ToolError::InvalidInput("regex could not be compiled".to_owned())
        })?;
    let mut matches = Vec::new();
    let mut truncated = false;
    for found in regex.find_iter(&input.text) {
        if matches.len() >= max_results {
            truncated = true;
            break;
        }
        matches.push(RegexMatch {
            text: found.as_str().to_owned(),
            start: found.start(),
            end: found.end(),
        });
    }
    Ok(RegexQueryOutput { matches, truncated })
}
