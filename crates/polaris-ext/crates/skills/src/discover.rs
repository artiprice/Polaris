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
#[path = "../../../../../tests/crates/polaris-ext/crates/skills/src/discover.rs"]
mod tests;
