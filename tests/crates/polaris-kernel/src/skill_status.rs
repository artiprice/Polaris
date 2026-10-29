// Unit tests of crates/polaris-kernel/src/skill_status.rs (`tests`); compiled into that module via #[path].
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
