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
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn status_compares_requirements_to_the_real_local_builtin_catalog() {
        let tree = tempfile::tempdir().expect("create temporary status roots");
        let global = tree.path().join("global");
        let repository = tree.path().join("repository");
        let repository_root = repository.join(".polaris");
        let skill_dir = repository_root.join("skills/repo-skill");
        fs::create_dir_all(&skill_dir).expect("create repository skill folder");
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: repo-skill\ndescription: test tool requirements\nrequires_tools: [read_file, future_tool]\n---\nbody",
        )
        .expect("write repository skill");
        let dirs = Dirs {
            global,
            repo: Some(repository_root),
        };

        let report = inspect_skill_status(&dirs);
        assert_eq!(report.discovery, SkillDiscoveryStatus::Available);
        assert_eq!(report.enablement, SkillEnablementStatus::Unsupported);
        assert_eq!(
            report.tool_registry_scope,
            SkillToolRegistryScope::LocalBuiltin
        );
        assert_eq!(report.discovered_skills, 1);
        assert_eq!(report.skills[0].skill, "repo-skill");
        assert_eq!(report.skills[0].trust, "repo_unapproved");
        assert_eq!(
            report.skills[0].required_tools,
            ["read_file", "future_tool"]
        );
        assert_eq!(report.skills[0].missing_tools, ["future_tool"]);
        assert_eq!(report.discovery_error, None);
    }

    #[test]
    fn invalid_skill_root_is_reported_unavailable_without_fabricated_entries() {
        let tree = tempfile::tempdir().expect("create temporary status root");
        let repository_root = tree.path().join(".polaris");
        fs::create_dir_all(&repository_root).expect("create repository config root");
        let file_root = repository_root.join("skills");
        fs::write(&file_root, "file").expect("create invalid skill root");
        let report = inspect_skill_status(&Dirs {
            global: tree.path().join("global"),
            repo: Some(repository_root),
        });

        assert_eq!(report.discovery, SkillDiscoveryStatus::Unavailable);
        assert_eq!(report.discovered_skills, 0);
        assert!(report.skills.is_empty());
        assert_eq!(
            report.discovery_error,
            Some("configured skill roots could not be indexed")
        );
    }
}
