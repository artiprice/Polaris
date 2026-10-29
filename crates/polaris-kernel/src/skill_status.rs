//! Headless skill diagnostics against the kernel's current local builtin tool catalog.

use std::collections::BTreeSet;

use polaris_config::Dirs;
use polaris_ext::skills::{SkillRegistry, SkillRoot, SkillSource};
use serde::Serialize;

/// Whether the configured skill roots could be indexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillDiscoveryStatus {
    /// The bounded skill registry was indexed successfully.
    Available,
    /// At least one configured root could not be safely indexed.
    Unavailable,
}

/// Runtime support for applying session-level `$skills(...)` directives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillEnablementStatus {
    /// Directive state exists, but runtime skill selection is not yet wired to it.
    Unsupported,
}

/// Current tool-catalog scope used to compare skill requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillToolRegistryScope {
    /// Names from the same builtin catalog used by the local kernel ToolRegistry.
    LocalBuiltin,
}

/// Tool requirements for one discovered skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillRequirementStatus {
    /// Validated skill name.
    pub skill: String,
    /// Trust tier currently assigned by discovery.
    pub trust: String,
    /// Tool names declared by the skill.
    pub required_tools: Vec<String>,
    /// Declared names absent from the current local builtin catalog.
    pub missing_tools: Vec<String>,
}

/// Serializable status data for the headless CLI and future UI consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillStatusSnapshot {
    /// Result of bounded discovery over configured user and repository roots.
    pub discovery: SkillDiscoveryStatus,
    /// `$skills` directives are parsed and persisted, but do not yet control runtime injection.
    pub enablement: SkillEnablementStatus,
    /// Catalog against which `missing_tools` was computed.
    pub tool_registry_scope: SkillToolRegistryScope,
    /// Count of active discovered skill names.
    pub discovered_skills: usize,
    /// Requirements for every indexed active skill.
    pub skills: Vec<SkillRequirementStatus>,
    /// Safe summary when discovery fails; detailed filesystem errors are omitted from status.
    pub discovery_error: Option<&'static str>,
}

/// Index configured roots and compare their declared tools to the local kernel catalog.
///
/// Missing optional directories yield an available, empty result. Invalid roots and parse or IO
/// errors fail discovery closed and produce an unavailable report without changing doctor/setup
/// readiness. The local catalog is the same `builtin_specs()` source consumed by `ToolRegistry`.
#[must_use]
pub fn inspect_skill_status(dirs: &Dirs) -> SkillStatusSnapshot {
    let mut roots = vec![SkillRoot::user_global(dirs.global.join("skills"))];
    if let Some(repo) = &dirs.repo {
        roots.push(SkillRoot::repository(repo.join("skills")));
    }
    let registry = match SkillRegistry::discover(&roots) {
        Ok(registry) => registry,
        Err(_) => {
            return SkillStatusSnapshot {
                discovery: SkillDiscoveryStatus::Unavailable,
                enablement: SkillEnablementStatus::Unsupported,
                tool_registry_scope: SkillToolRegistryScope::LocalBuiltin,
                discovered_skills: 0,
                skills: Vec::new(),
                discovery_error: Some("configured skill roots could not be indexed"),
            };
        }
    };
    let available_tools = local_builtin_tool_names();
    let skills = registry
        .list()
        .iter()
        .map(|meta| SkillRequirementStatus {
            skill: meta.name.to_string(),
            trust: match meta.source {
                SkillSource::UserGlobal => "user_global",
                SkillSource::RepoApproved => "repo_approved",
                SkillSource::RepoUnapproved => "repo_unapproved",
            }
            .to_owned(),
            required_tools: meta.requires_tools.clone(),
            missing_tools: meta
                .requires_tools
                .iter()
                .filter(|tool| !available_tools.contains(*tool))
                .cloned()
                .collect(),
        })
        .collect::<Vec<_>>();
    SkillStatusSnapshot {
        discovery: SkillDiscoveryStatus::Available,
        enablement: SkillEnablementStatus::Unsupported,
        tool_registry_scope: SkillToolRegistryScope::LocalBuiltin,
        discovered_skills: skills.len(),
        skills,
        discovery_error: None,
    }
}

fn local_builtin_tool_names() -> BTreeSet<String> {
    polaris_tools::builtin::builtin_specs()
        .into_iter()
        .map(|spec| spec.name.into_string())
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/crates/polaris-kernel/src/skill_status.rs"]
mod tests;
