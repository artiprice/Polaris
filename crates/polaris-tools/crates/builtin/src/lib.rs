//! # polaris-tools-builtin
//!
//! Bounded workspace inspection, diff proposal, and sandboxed execution tools.
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L3 — domain |
//! | Kind | lib |
//! | Phase | P1 |
//! | Parent facade | `polaris-tools` |
//! | Status | P1 inspection and opt-in P2 proposal/sandbox routes implemented; services must be explicitly injected |
//!
//! ## Responsibilities
//!
//! - Resolve existing workspace paths with lexical checks, reparse-point rejection, and canonical containment.
//! - Read, list, glob, grep, summarize and query workspace content under explicit limits.
//! - Route workspace changes through unverified `ProposeDiff` commands; never write workspace files directly.
//! - Execute explicit argv only through a caller-owned SDK sandbox backend and handle.
//! - Expand and recall use the composition root's durable CAS and session-scoped registry.
//! - Git, Fetch, and remote MCP tools belong to later phases.
//!
//! ## API
//!
//! - `struct ToolRegistry`
//! - `trait Tool`
//!
//! ## Internal dependencies
//!
//! `polaris-sdk`, `polaris-code-intel`, `polaris-security-automata`
//!
//! ## Planned external dependencies
//!
//! `ignore`, `globset`, `regex`, `serde`, `serde_json`, `encoding_rs`

mod patch;
mod path;
mod proposal;
mod query;
mod read;
mod registry;
mod search;
mod shell;

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Caller-supplied execution and output limits for one tool invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolLimits {
    /// Maximum source bytes read from one file.
    pub max_read_bytes: u64,
    /// Maximum result rows or matches returned.
    pub max_results: usize,
    /// Maximum serialized tool output bytes.
    pub max_output_bytes: u64,
    /// Maximum JSON arguments accepted for one tool call.
    pub max_input_bytes: u64,
    /// Regex automaton memory ceiling.
    pub regex_size_limit: usize,
    /// Wall-clock budget, checked between filesystem operations and matches.
    pub timeout: Duration,
}

impl ToolLimits {
    /// Validate all limits before constructing the registry.
    pub fn validate(&self) -> Result<(), polaris_sdk::registry::ToolError> {
        if self.max_read_bytes == 0
            || self.max_results == 0
            || self.max_output_bytes == 0
            || self.max_input_bytes == 0
            || self.regex_size_limit == 0
            || self.timeout.is_zero()
        {
            return Err(polaris_sdk::registry::ToolError::InvalidInput(
                "tool limits must be positive".to_owned(),
            ));
        }
        Ok(())
    }
}

/// JSON tool result with explicit output size and truncation metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOutput {
    /// Serialized tool-specific result value.
    pub value: Value,
    /// True when a configured result/output/time limit omitted data.
    pub truncated: bool,
    /// Serialized value size in bytes.
    pub bytes: u64,
}

pub use patch::{PatchError, apply_unified_patch};
pub use path::{PathError, WorkspacePath, resolve_in_workspace};
pub use proposal::{
    CasKernelProposalBackend, DiffProposal, EditFileRequest, NoProposalBackend, PreparedDiff,
    ProposalTokenSource, ProposalToolBackend, prepare_workspace_diff,
};
pub use query::{
    JsonQueryInput, JsonQueryOutput, RegexQueryInput, RegexQueryOutput, json_query, regex_query,
};
pub use read::{ReadFileInput, ReadFileOutput, read_file};
pub use registry::{
    CasRefResolver, RecallBackend, RefResolver, ToolRegistry, builtin_specs, p2_builtin_specs,
};
pub use search::{
    GlobInput, GrepInput, ListDirInput, SearchOutput, SearchRow, glob, grep, list_dir,
};
pub use shell::{
    CommandClass, NoShellBackend, SandboxShellBackend, ShellFlavor, ShellInput, ShellOutput,
    ShellToolBackend, classify_command,
};
