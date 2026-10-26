use thiserror::Error;

/// Failure while parsing source, scanning rules, validating budgets or using a sandbox.
#[derive(Debug, Error)]
pub enum HackerError {
    /// Source could not be parsed or exceeded parser limits.
    #[error("source analysis failed: {0}")]
    Parse(#[from] polaris_code_intel::CodeIntelError),
    /// Automata scanning failed, including when the caller-provided result cap was exceeded.
    #[error("rule scanning failed: {0}")]
    Automata(#[from] polaris_security_automata::AutomataError),
    /// A request exceeded local bounds or omitted a required field.
    #[error("invalid audit request: {0}")]
    InvalidRequest(&'static str),
    /// The sandbox backend failed. Cleanup failure is appended when both operations fail.
    #[error("sandbox operation failed: {0}")]
    Sandbox(String),
}
