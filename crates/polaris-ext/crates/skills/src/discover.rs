use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cap_std::fs::Dir;
use polaris_sdk::contracts::ApprovalWitness;
use polaris_sdk::ids::Cid;

use crate::parse::MAX_SKILL_BYTES;
use crate::{SkillError, SkillName, parse_skill};

pub const MAX_SKILLS: usize = 2_000;
const MAX_ROOTS: usize = 16;
const MAX_WALK_ENTRIES: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillRootKind {
    UserGlobal,
    Repository,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRoot {
    pub path: PathBuf,
    pub kind: SkillRootKind,
}

impl SkillRoot {
    #[must_use]
    pub fn user_global(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: SkillRootKind::UserGlobal,
        }
    }

    #[must_use]
    pub fn repository(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: SkillRootKind::Repository,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSource {
    UserGlobal,
    RepoApproved,
    RepoUnapproved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMeta {
    pub name: SkillName,
    pub description: String,
    pub source: SkillSource,
    pub path: PathBuf,
    pub body_blake3: Cid,
    pub source_blake3: Cid,
    pub requires_tools: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub meta: SkillMeta,
    pub body: String,
}

#[derive(Debug)]
struct Candidate {
    path: PathBuf,
    directory: Arc<Dir>,
    kind: SkillRootKind,
    description: String,
    requires_tools: Vec<String>,
    body_blake3: Cid,
    source_blake3: Cid,
}

#[derive(Debug, Default)]
struct NameCandidates {
    user: Option<Candidate>,
    repo: Option<Candidate>,
}

#[derive(Debug, Default)]
pub struct SkillRegistry {
    candidates: BTreeMap<SkillName, NameCandidates>,
    approvals: BTreeMap<SkillName, ApprovalWitness>,
    active: Vec<SkillMeta>,
}

impl SkillRegistry {
    /// Indexes bounded direct-child skill directories. Bodies are discarded after hashing.
    ///
    /// Configured roots are opened with ambient authority and form trust boundaries. Once opened,
    /// all skill enumeration and reads are relative to pinned directory handles.
    /// Approval witnesses may be persisted by the trusted caller and supplied on reload.
    pub fn discover(roots: &[SkillRoot]) -> Result<Self, SkillError> {
        Self::discover_with_approvals(roots, &[])
    }

    pub fn discover_with_approvals(
        roots: &[SkillRoot],
        approvals: &[ApprovalWitness],
    ) -> Result<Self, SkillError> {
        if roots.len() > MAX_ROOTS || approvals.len() > MAX_SKILLS {
            return Err(SkillError::LimitExceeded);
        }
        let mut registry = Self::default();
        let mut walked = 0_usize;
        for skill_root in roots {
            let root_meta = match fs::symlink_metadata(&skill_root.path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(SkillError::Io(error.to_string())),
            };
            if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
                return Err(SkillError::InvalidRoot(skill_root.path.clone()));
            }
            let root = fs::canonicalize(&skill_root.path)
                .map_err(|error| SkillError::Io(error.to_string()))?;
            let root_dir = Dir::open_ambient_dir(&root, cap_std::ambient_authority())
                .map_err(|error| SkillError::Io(error.to_string()))?;
            for entry in root_dir
                .entries()
                .map_err(|error| SkillError::Io(error.to_string()))?
            {
                walked = walked.checked_add(1).ok_or(SkillError::LimitExceeded)?;
                if walked > MAX_WALK_ENTRIES {
                    return Err(SkillError::LimitExceeded);
                }
                let entry = entry.map_err(|error| SkillError::Io(error.to_string()))?;
                let entry_type = entry
                    .file_type()
                    .map_err(|error| SkillError::Io(error.to_string()))?;
                if !entry_type.is_dir() || entry_type.is_symlink() {
                    continue;
                }
                let skill_directory = Arc::new(
                    entry
                        .open_dir()
                        .map_err(|error| SkillError::Io(error.to_string()))?,
                );
                let skill_directory_name = entry.file_name();
                for skill_entry in skill_directory
                    .entries()
                    .map_err(|error| SkillError::Io(error.to_string()))?
                {
                    walked = walked.checked_add(1).ok_or(SkillError::LimitExceeded)?;
                    if walked > MAX_WALK_ENTRIES {
                        return Err(SkillError::LimitExceeded);
                    }
                    let skill_entry =
                        skill_entry.map_err(|error| SkillError::Io(error.to_string()))?;
                    if skill_entry.file_name() != "SKILL.md" {
                        continue;
                    }
                    let skill_entry_type = skill_entry
                        .file_type()
                        .map_err(|error| SkillError::Io(error.to_string()))?;
                    if !skill_entry_type.is_file() || skill_entry_type.is_symlink() {
                        continue;
                    }
                    let path = root.join(&skill_directory_name).join("SKILL.md");
                    let bytes = read_bounded(&skill_directory, &path)?;
                    let text = std::str::from_utf8(&bytes)
                        .map_err(|_| SkillError::InvalidUtf8(path.clone()))?;
                    let parsed = parse_skill(text)?;
                    let name = parsed.name.clone();
                    let candidate = Candidate {
                        path,
                        directory: Arc::clone(&skill_directory),
                        kind: skill_root.kind,
                        description: parsed.description,
                        requires_tools: parsed.requires_tools,
                        body_blake3: parsed.body_blake3,
                        source_blake3: parsed.source_blake3,
                    };
                    let slot = registry.candidates.entry(name.clone()).or_default();
                    let target = match skill_root.kind {
                        SkillRootKind::UserGlobal => &mut slot.user,
                        SkillRootKind::Repository => &mut slot.repo,
                    };
                    if target.replace(candidate).is_some() {
                        return Err(SkillError::NameCollision(name));
                    }
                    if registry.candidates.len() > MAX_SKILLS {
                        return Err(SkillError::LimitExceeded);
                    }
                }
            }
        }
        for witness in approvals {
            for (name, choices) in &registry.candidates {
                if let Some(repo) = &choices.repo
                    && approval_evidence(name, repo.source_blake3) == witness.evidence()
                {
                    registry.approvals.insert(name.clone(), witness.clone());
                }
            }
        }
        registry.rebuild_active();
        Ok(registry)
    }

    #[must_use]
    pub fn list(&self) -> &[SkillMeta] {
        &self.active
    }

    /// Digest to include in an explicit approval request for a repository skill.
    pub fn approval_evidence(&self, name: &SkillName) -> Result<Cid, SkillError> {
        let repo = self
            .candidates
            .get(name)
            .and_then(|candidate| candidate.repo.as_ref())
            .ok_or_else(|| SkillError::Unknown(name.clone()))?;
        Ok(approval_evidence(name, repo.source_blake3))
    }

    /// Applies a caller-verified witness and returns the witness for trusted persistence.
    pub fn approve_repo_skill(
        &mut self,
        name: &SkillName,
        witness: &ApprovalWitness,
    ) -> Result<(), SkillError> {
        let expected = self.approval_evidence(name)?;
        if witness.evidence() != expected {
            return Err(SkillError::ApprovalMismatch(name.clone()));
        }
        if self
            .candidates
            .get(name)
            .and_then(|candidate| candidate.repo.as_ref())
            .is_none()
        {
            return Err(SkillError::Unknown(name.clone()));
        }
        self.approvals.insert(name.clone(), witness.clone());
        self.rebuild_active();
        Ok(())
    }

    /// Approval witnesses contain only trusted approver identity, timestamp, and content digest.
    #[must_use]
    pub fn approval_witnesses(&self) -> Vec<ApprovalWitness> {
        self.approvals.values().cloned().collect()
    }

    pub fn load(&self, name: &SkillName) -> Result<Skill, SkillError> {
        let candidate = self.active_candidate(name)?;
        let source = match candidate.kind {
            SkillRootKind::UserGlobal => SkillSource::UserGlobal,
            SkillRootKind::Repository if self.approvals.contains_key(name) => {
                SkillSource::RepoApproved
            }
            SkillRootKind::Repository => return Err(SkillError::NotApproved(name.clone())),
        };
        let bytes = read_bounded(&candidate.directory, &candidate.path)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| SkillError::InvalidUtf8(candidate.path.clone()))?;
        let parsed = parse_skill(text)?;
        if parsed.source_blake3 != candidate.source_blake3 {
            if source == SkillSource::RepoApproved {
                return Err(SkillError::ApprovalStale(name.clone()));
            }
            return Err(SkillError::StaleIndex(name.clone()));
        }
        Ok(Skill {
            meta: SkillMeta {
                name: parsed.name,
                description: parsed.description,
                source,
                path: candidate.path.clone(),
                body_blake3: parsed.body_blake3,
                source_blake3: parsed.source_blake3,
                requires_tools: parsed.requires_tools,
            },
            body: parsed.body,
        })
    }

    pub fn missing_tools(
        &self,
        name: &SkillName,
        available: &BTreeSet<String>,
    ) -> Result<Vec<String>, SkillError> {
        let meta = self.active_meta(name)?;
        Ok(meta
            .requires_tools
            .iter()
            .filter(|tool| !available.contains(*tool))
            .cloned()
            .collect())
    }

    fn active_candidate(&self, name: &SkillName) -> Result<&Candidate, SkillError> {
        let choices = self
            .candidates
            .get(name)
            .ok_or_else(|| SkillError::Unknown(name.clone()))?;
        if self.approvals.contains_key(name)
            && let Some(repo) = &choices.repo
        {
            return Ok(repo);
        }
        choices
            .user
            .as_ref()
            .or(choices.repo.as_ref())
            .ok_or_else(|| SkillError::Unknown(name.clone()))
    }

    fn active_meta(&self, name: &SkillName) -> Result<&SkillMeta, SkillError> {
        self.active
            .binary_search_by(|meta| meta.name.cmp(name))
            .ok()
            .and_then(|index| self.active.get(index))
            .ok_or_else(|| SkillError::Unknown(name.clone()))
    }

    fn rebuild_active(&mut self) {
        self.active.clear();
        for (name, choices) in &self.candidates {
            let repo_approved = self.approvals.contains_key(name) && choices.repo.is_some();
            let candidate = if repo_approved {
                choices.repo.as_ref()
            } else {
                choices.user.as_ref().or(choices.repo.as_ref())
            };
            if let Some(candidate) = candidate {
                self.active.push(SkillMeta {
                    name: name.clone(),
                    description: candidate.description.clone(),
                    source: match candidate.kind {
                        SkillRootKind::UserGlobal => SkillSource::UserGlobal,
                        SkillRootKind::Repository if repo_approved => SkillSource::RepoApproved,
                        SkillRootKind::Repository => SkillSource::RepoUnapproved,
                    },
                    path: candidate.path.clone(),
                    body_blake3: candidate.body_blake3,
                    source_blake3: candidate.source_blake3,
                    requires_tools: candidate.requires_tools.clone(),
                });
            }
        }
    }
}

fn read_bounded(directory: &Dir, path: &Path) -> Result<Vec<u8>, SkillError> {
    let path_before = directory
        .symlink_metadata("SKILL.md")
        .map_err(|error| SkillError::Io(error.to_string()))?;
    if path_before.is_symlink() || !path_before.is_file() {
        return Err(SkillError::PathEscape(path.to_path_buf()));
    }
    if path_before.len() > u64::try_from(MAX_SKILL_BYTES).map_err(|_| SkillError::LimitExceeded)? {
        return Err(SkillError::TooLarge);
    }
    read_bounded_open(directory, path)
}

fn read_bounded_open(directory: &Dir, path: &Path) -> Result<Vec<u8>, SkillError> {
    let file = directory
        .open("SKILL.md")
        .map_err(|error| SkillError::Io(error.to_string()))?;
    let opened = file
        .metadata()
        .map_err(|error| SkillError::Io(error.to_string()))?;
    if !opened.is_file() {
        return Err(SkillError::ChangedDuringRead(path.to_path_buf()));
    }
    let mut bytes = Vec::new();
    file.take(
        u64::try_from(MAX_SKILL_BYTES)
            .map_err(|_| SkillError::LimitExceeded)?
            .saturating_add(1),
    )
    .read_to_end(&mut bytes)
    .map_err(|error| SkillError::Io(error.to_string()))?;
    if bytes.len() > MAX_SKILL_BYTES || u64::try_from(bytes.len()).ok() != Some(opened.len()) {
        return Err(SkillError::ChangedDuringRead(path.to_path_buf()));
    }
    Ok(bytes)
}

fn approval_evidence(name: &SkillName, source: Cid) -> Cid {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"polaris-skill-approval-v1\0");
    hasher.update(name.as_str().as_bytes());
    hasher.update(source.as_bytes());
    Cid::from_bytes(*hasher.finalize().as_bytes())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempTree(PathBuf);

    impl TempTree {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "polaris-skill-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create temporary test tree");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn put_skill(&self, root: &Path, directory: &str, name: &str, body: &str) {
            let folder = root.join(directory);
            fs::create_dir_all(&folder).expect("create skill folder");
            let source = format!("---\nname: {name}\ndescription: {name} test skill\n---\n{body}");
            fs::write(folder.join("SKILL.md"), source).expect("write skill fixture");
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discovery_keeps_repo_skills_unapproved_and_global_skill_wins() {
        let tree = TempTree::new("precedence");
        let global = tree.path().join("global");
        let repo = tree.path().join("repo");
        fs::create_dir_all(&global).expect("create global root");
        fs::create_dir_all(&repo).expect("create repo root");
        tree.put_skill(&global, "git-bisect", "git-bisect", "global body");
        tree.put_skill(&repo, "git-bisect", "git-bisect", "repo body");
        tree.put_skill(&repo, "db-migrate", "db-migrate", "migration body");

        let registry = SkillRegistry::discover(&[
            SkillRoot::user_global(&global),
            SkillRoot::repository(&repo),
        ])
        .expect("bounded roots should index");

        assert_eq!(registry.list().len(), 2);
        let global_name = SkillName::new("git-bisect").expect("valid test name");
        assert_eq!(
            registry
                .load(&global_name)
                .expect("global skill loads")
                .body,
            "global body"
        );
        let repo_name = SkillName::new("db-migrate").expect("valid test name");
        let repo_meta = registry
            .list()
            .iter()
            .find(|meta| meta.name == repo_name)
            .expect("repo-only skill should be indexed");
        assert_eq!(repo_meta.source, SkillSource::RepoUnapproved);
        assert!(matches!(
            registry.load(&repo_name),
            Err(SkillError::NotApproved(_))
        ));
    }

    #[test]
    fn changed_skill_body_requires_rediscovery_and_repo_approval_again() {
        let tree = TempTree::new("stale");
        let global = tree.path().join("global");
        fs::create_dir_all(&global).expect("create global root");
        tree.put_skill(&global, "skill", "skill", "original body");
        let registry = SkillRegistry::discover(&[SkillRoot::user_global(&global)])
            .expect("skill should index");
        let name = SkillName::new("skill").expect("valid test name");
        let path = global.join("skill").join("SKILL.md");
        fs::remove_file(&path).expect("remove the indexed file before replacement");
        fs::write(
            path,
            "---\nname: skill\ndescription: skill test skill\n---\nchanged body",
        )
        .expect("mutate skill after index");
        assert!(matches!(
            registry.load(&name),
            Err(SkillError::StaleIndex(_))
        ));
    }

    #[test]
    fn duplicate_names_within_a_trust_tier_are_rejected() {
        let tree = TempTree::new("duplicate");
        tree.put_skill(tree.path(), "one", "same", "one");
        tree.put_skill(tree.path(), "two", "same", "two");
        assert!(matches!(
            SkillRegistry::discover(&[SkillRoot::repository(tree.path())]),
            Err(SkillError::NameCollision(_))
        ));
    }

    #[test]
    fn repository_approval_persists_by_source_digest_and_stales_after_edit() {
        let tree = TempTree::new("approval");
        tree.put_skill(tree.path(), "repo-skill", "repo-skill", "original body");
        let root = SkillRoot::repository(tree.path());
        let mut registry =
            SkillRegistry::discover(std::slice::from_ref(&root)).expect("repo skill indexes");
        let name = SkillName::new("repo-skill").expect("valid skill name");
        let evidence = registry
            .approval_evidence(&name)
            .expect("approval evidence exists");
        let witness_source = format!(
            "approver: polaris-kernel-verifier\nevidence: {}\nat: '2026-10-23T00:00:00Z'\n",
            evidence.to_hex()
        );
        let witness: ApprovalWitness =
            serde_saphyr::from_str(&witness_source).expect("trusted caller witness fixture parses");
        registry
            .approve_repo_skill(&name, &witness)
            .expect("matching witness approves exact source");
        assert_eq!(
            registry
                .load(&name)
                .expect("approved skill loads")
                .meta
                .source,
            SkillSource::RepoApproved
        );

        let persisted = registry.approval_witnesses();
        let reloaded = SkillRegistry::discover_with_approvals(&[root], &persisted)
            .expect("persisted witness restores exact approval");
        assert_eq!(
            reloaded
                .load(&name)
                .expect("reloaded approval loads")
                .meta
                .source,
            SkillSource::RepoApproved
        );

        fs::write(
            tree.path().join("repo-skill/SKILL.md"),
            "---\nname: repo-skill\ndescription: repo-skill test skill\n---\nchanged body",
        )
        .expect("edit approved source");
        assert!(matches!(
            reloaded.load(&name),
            Err(SkillError::ApprovalStale(_))
        ));
    }

    #[test]
    fn missing_roots_are_empty_and_file_roots_fail_closed() {
        let tree = TempTree::new("roots");
        let missing = tree.path().join("missing");
        assert!(
            SkillRegistry::discover(&[SkillRoot::repository(missing)])
                .expect("missing optional roots are empty")
                .list()
                .is_empty()
        );
        let file = tree.path().join("file");
        fs::write(&file, "not a directory").expect("create invalid root file");
        assert!(matches!(
            SkillRegistry::discover(&[SkillRoot::repository(file)]),
            Err(SkillError::InvalidRoot(_))
        ));
    }

    #[test]
    fn discovery_rejects_oversized_skill_files_before_parsing() {
        let tree = TempTree::new("oversized-file");
        tree.put_skill(tree.path(), "skill", "skill", "small body");
        fs::write(
            tree.path().join("skill/SKILL.md"),
            vec![b'x'; MAX_SKILL_BYTES.saturating_add(1)],
        )
        .expect("write bounded oversized fixture");

        assert!(matches!(
            SkillRegistry::discover(&[SkillRoot::user_global(tree.path())]),
            Err(SkillError::TooLarge)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn discovery_never_follows_symlinked_skill_directories_or_files() {
        use std::os::unix::fs::symlink;

        let tree = TempTree::new("symlink");
        let root = tree.path().join("root");
        let outside = tree.path().join("outside");
        fs::create_dir_all(&root).expect("create skill root");
        fs::create_dir_all(outside.join("external")).expect("create external skill");
        fs::write(
            outside.join("external/SKILL.md"),
            "---\nname: external\ndescription: external skill fixture\n---\nexternal body",
        )
        .expect("write external skill");
        fs::create_dir_all(root.join("file-link")).expect("create file link directory");
        symlink(
            outside.join("external/SKILL.md"),
            root.join("file-link/SKILL.md"),
        )
        .expect("create SKILL.md symlink");
        symlink(outside.join("external"), root.join("directory-link"))
            .expect("create skill directory symlink");

        let registry = SkillRegistry::discover(&[SkillRoot::repository(&root)])
            .expect("symlink entries are ignored");
        assert!(registry.list().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn lazy_load_rejects_file_replaced_by_symlink_escape() {
        use std::os::unix::fs::symlink;

        let tree = TempTree::new("load-symlink-replacement");
        let root = tree.path().join("root");
        let skill = root.join("skill");
        let outside = tree.path().join("outside");
        fs::create_dir_all(&skill).expect("create skill directory");
        fs::create_dir_all(&outside).expect("create outside directory");
        tree.put_skill(&root, "skill", "skill", "approved body");
        tree.put_skill(&outside, "external", "skill", "outside body");
        let registry = SkillRegistry::discover(&[SkillRoot::user_global(&root)])
            .expect("registry should index the in-root skill");
        let name = SkillName::new("skill").expect("valid test name");

        fs::remove_file(skill.join("SKILL.md")).expect("remove indexed file");
        symlink(outside.join("external/SKILL.md"), skill.join("SKILL.md"))
            .expect("replace indexed file with an outside symlink");

        assert!(matches!(
            registry.load(&name),
            Err(SkillError::PathEscape(_))
        ));
        let candidate = registry
            .active_candidate(&name)
            .expect("indexed candidate remains available");
        assert!(read_bounded_open(&candidate.directory, &candidate.path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn lazy_load_stays_confined_when_configured_root_path_is_replaced() {
        use std::os::unix::fs::symlink;

        let tree = TempTree::new("root-symlink-replacement");
        let root = tree.path().join("root");
        let moved_root = tree.path().join("retained-root");
        let outside = tree.path().join("outside");
        fs::create_dir_all(&root).expect("create configured root");
        fs::create_dir_all(&outside).expect("create outside root");
        tree.put_skill(&root, "skill", "skill", "indexed body");
        tree.put_skill(&outside, "skill", "skill", "outside body");
        let registry = SkillRegistry::discover(&[SkillRoot::user_global(&root)])
            .expect("index configured root");
        let name = SkillName::new("skill").expect("valid test name");

        fs::rename(&root, &moved_root).expect("move the already-open configured root");
        symlink(&outside, &root).expect("replace configured root with outside symlink");

        assert_eq!(
            registry
                .load(&name)
                .expect("read through pinned skill handle")
                .body,
            "indexed body"
        );
    }

    #[cfg(windows)]
    #[test]
    fn pinned_skill_directory_blocks_rename_until_registry_is_dropped() {
        let tree = TempTree::new("pinned-skill-directory");
        let root = tree.path().join("root");
        let skill = root.join("skill");
        let renamed_skill = root.join("retained-skill");
        fs::create_dir_all(&root).expect("create configured root");
        tree.put_skill(&root, "skill", "skill", "indexed body");
        let registry = SkillRegistry::discover(&[SkillRoot::user_global(&root)])
            .expect("index configured root");
        let name = SkillName::new("skill").expect("valid test name");

        let rename_while_pinned = fs::rename(&skill, &renamed_skill);
        assert!(
            rename_while_pinned.is_err(),
            "Windows must deny renaming a directory while its capability is held"
        );
        assert_eq!(
            registry
                .load(&name)
                .expect("pinned skill remains loadable")
                .body,
            "indexed body"
        );

        drop(registry);
        fs::rename(&skill, &renamed_skill)
            .expect("directory rename succeeds after the capability is released");
    }

    #[test]
    fn discovery_scales_through_the_two_thousand_skill_limit() {
        use std::time::Instant;

        let tree = TempTree::new("scale");
        let root = tree.path().join("skills");
        fs::create_dir_all(&root).expect("create scale root");
        let mut created = 0_usize;
        for target in [10_usize, 200, MAX_SKILLS] {
            while created < target {
                let name = format!("skill-{created:04}");
                let directory = root.join(&name);
                fs::create_dir_all(&directory).expect("create skill directory");
                fs::write(
                    directory.join("SKILL.md"),
                    format!(
                        "---\nname: {name}\ndescription: bounded scale fixture {name}\n---\nbody {name}"
                    ),
                )
                .expect("write bounded skill fixture");
                created = created.saturating_add(1);
            }
            let start = Instant::now();
            let registry = SkillRegistry::discover(&[SkillRoot::user_global(&root)])
                .expect("registry accepts configured scale bound");
            let elapsed = start.elapsed();
            assert_eq!(registry.list().len(), target);
            eprintln!(
                "SKILL_SCALE skills={target} index_ms={}",
                elapsed.as_millis()
            );
        }
    }
}
