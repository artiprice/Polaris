//! Exact, advisory role-to-tool policy lookup.

use polaris_sdk::ids::AgentRole;
use std::collections::BTreeSet;

/// A canonical tool identifier used by the role advisory table.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ToolId(String);

impl ToolId {
    /// Validate a canonical lower-case tool id.
    ///
    /// # Errors
    /// Returns [`RoleTableError::InvalidToolId`] for an empty or non-canonical id.
    pub fn new(value: &str) -> Result<Self, RoleTableError> {
        if value.is_empty()
            || value.len() > 128
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte)
            })
        {
            return Err(RoleTableError::InvalidToolId);
        }
        Ok(Self(value.to_owned()))
    }

    /// Canonical tool name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A role's advisory tool and path policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolePolicy {
    role: AgentRole,
    allowed_tools: BTreeSet<ToolId>,
    network_tools: BTreeSet<ToolId>,
    execution_tools: BTreeSet<ToolId>,
    allowed_path_prefixes: Vec<String>,
    writable_path_prefixes: Vec<String>,
}

impl RolePolicy {
    /// Construct a role policy from exact tool names and normalized workspace-relative prefixes.
    ///
    /// An empty tool list denies all tool proposals for the role. A write requires a matching
    /// writable prefix in addition to a tool and general path match.
    ///
    /// # Errors
    /// Returns an error for invalid tools, path prefixes, effect grants, or duplicate roles.
    #[must_use]
    pub fn new(
        role: AgentRole,
        allowed_tools: BTreeSet<ToolId>,
        network_tools: BTreeSet<ToolId>,
        execution_tools: BTreeSet<ToolId>,
        allowed_path_prefixes: Vec<String>,
        writable_path_prefixes: Vec<String>,
    ) -> Result<Self, RoleTableError> {
        let allowed_path_prefixes = normalize_prefixes(allowed_path_prefixes)?;
        let writable_path_prefixes = normalize_prefixes(writable_path_prefixes)?;
        if network_tools
            .iter()
            .chain(execution_tools.iter())
            .any(|tool| !allowed_tools.contains(tool))
        {
            return Err(RoleTableError::InvalidEffectGrant);
        }
        if writable_path_prefixes.iter().any(|write| {
            !allowed_path_prefixes
                .iter()
                .any(|allowed| prefix_contains(allowed, write))
        }) {
            return Err(RoleTableError::InvalidPathPrefix);
        }
        Ok(Self {
            role,
            allowed_tools,
            network_tools,
            execution_tools,
            allowed_path_prefixes,
            writable_path_prefixes,
        })
    }
}

/// Structured facts for an advisory role check; these facts do not grant authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallFacts {
    /// Exact tool identifier.
    pub tool: ToolId,
    /// Workspace-relative path, if the tool refers to a path.
    pub workspace_path: Option<String>,
    /// True when the tool may write workspace files.
    pub writes_workspace: bool,
    /// True when the tool uses the network.
    pub network: bool,
    /// True when the tool executes a process.
    pub executes: bool,
}

/// Errors constructing the advisory role table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RoleTableError {
    /// Tool id does not follow the canonical ASCII identifier format.
    #[error("invalid canonical tool id")]
    InvalidToolId,
    /// Path prefix is absolute, traversing, empty, or otherwise ambiguous.
    #[error("invalid workspace-relative path prefix")]
    InvalidPathPrefix,
    /// A network or process permission names a tool outside the role's allowlist.
    #[error("tool effect grant is outside the role allowlist")]
    InvalidEffectGrant,
    /// More than one table row was supplied for a role.
    #[error("duplicate role policy")]
    DuplicateRole,
}

/// Immutable advisory role table. Missing roles deny tool calls by default.
#[derive(Debug, Clone, Default)]
pub struct RoleTable {
    policies: Vec<RolePolicy>,
}

impl RoleTable {
    /// Construct a table, rejecting duplicate rows instead of choosing one by order.
    ///
    /// # Errors
    /// Returns [`RoleTableError::DuplicateRole`] if a role occurs more than once.
    pub fn new(policies: Vec<RolePolicy>) -> Result<Self, RoleTableError> {
        for (index, policy) in policies.iter().enumerate() {
            if policies
                .iter()
                .skip(index.saturating_add(1))
                .any(|later| later.role == policy.role)
            {
                return Err(RoleTableError::DuplicateRole);
            }
        }
        Ok(Self { policies })
    }

    /// Whether a role's exact allowlist includes the proposed tool, path and effects.
    ///
    /// The result is advisory only. The command executor must independently enforce the signed
    /// capability token and sandbox policy.
    #[must_use]
    pub fn allows(&self, role: AgentRole, call: &ToolCallFacts) -> bool {
        let Some(policy) = self.policies.iter().find(|policy| policy.role == role) else {
            return false;
        };
        if !policy.allowed_tools.contains(&call.tool) {
            return false;
        }
        if call.network && !policy.network_tools.contains(&call.tool) {
            return false;
        }
        if call.executes && !policy.execution_tools.contains(&call.tool) {
            return false;
        }
        if let Some(path) = call.workspace_path.as_deref() {
            let Ok(path) = normalize_path(path) else {
                return false;
            };
            if !policy
                .allowed_path_prefixes
                .iter()
                .any(|prefix| path_matches(prefix, &path))
            {
                return false;
            }
            if call.writes_workspace
                && !policy
                    .writable_path_prefixes
                    .iter()
                    .any(|prefix| path_matches(prefix, &path))
            {
                return false;
            }
        } else if call.writes_workspace {
            return false;
        }
        true
    }
}

fn normalize_prefixes(prefixes: Vec<String>) -> Result<Vec<String>, RoleTableError> {
    prefixes
        .iter()
        .map(|prefix| normalize_path(prefix))
        .collect()
}

fn normalize_path(path: &str) -> Result<String, RoleTableError> {
    if path.is_empty()
        || path.len() > 4096
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains([':', '%', '?', '*', '"', '<', '>', '|'])
        || path.chars().any(char::is_control)
    {
        return Err(RoleTableError::InvalidPathPrefix);
    }
    let normalized = path.replace('\\', "/");
    if normalized
        .split('/')
        .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(RoleTableError::InvalidPathPrefix);
    }
    Ok(if cfg!(windows) {
        normalized.to_ascii_lowercase()
    } else {
        normalized
    })
}

fn prefix_contains(parent: &str, child: &str) -> bool {
    child == parent
        || child
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn path_matches(prefix: &str, path: &str) -> bool {
    prefix_contains(prefix, path)
}
