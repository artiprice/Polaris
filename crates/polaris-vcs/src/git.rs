//! gix repository access and the bounded git process fallback.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use polaris_storage_cas::{Cas, CasConfig};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::VcsError;
use crate::journal::{CheckpointSeq, Journal};
use crate::paths::{BranchName, RepoPath, RevSpec};

const MAX_GIT_OUTPUT: u64 = 8 * 1024 * 1024;
const GIT_TIMEOUT: Duration = Duration::from_secs(30);

/// One changed repository path from gix status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    /// Repository-relative path.
    pub path: RepoPath,
    /// Which repository area reported the change.
    pub kind: StatusKind,
}

/// The changed area reported by gix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    /// The index differs from HEAD.
    Staged,
    /// The worktree differs from the index.
    Worktree,
    /// The path is untracked or a detected rename destination.
    Untracked,
    /// A status kind not represented by this crate's compact API.
    Other,
}

/// A deterministic repository status snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoStatus {
    /// Changed paths in lexical order.
    pub entries: Vec<StatusEntry>,
}

/// A bounded unified diff, preserving git's binary patch bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffSet {
    /// Diff output bytes. Binary patches are not coerced through UTF-8.
    pub bytes: Vec<u8>,
}

impl DiffSet {
    /// Returns the patch as UTF-8 when it contains no invalid bytes.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.bytes).ok()
    }
}

/// The active state of a worktree handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clean;

/// A worktree with edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dirty;

/// A removed worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Removed;

/// A worktree handle with compile-time lifecycle state.
#[derive(Debug, Clone)]
pub struct Worktree<State> {
    root: PathBuf,
    path: PathBuf,
    branch: BranchName,
    marker: std::marker::PhantomData<State>,
}

impl Worktree<Clean> {
    /// The canonical path of this linked worktree.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Marks the handle dirty after the caller applies a change.
    #[must_use]
    pub fn mark_dirty(self) -> Worktree<Dirty> {
        Worktree {
            root: self.root,
            path: self.path,
            branch: self.branch,
            marker: std::marker::PhantomData,
        }
    }
}

impl Worktree<Dirty> {
    /// The filesystem path of the worktree.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The branch checked out in the worktree.
    #[must_use]
    pub fn branch(&self) -> &BranchName {
        &self.branch
    }

    /// Removes a dirty worktree only when `force` is explicit.
    ///
    /// # Errors
    /// Returns [`VcsError::Git`] when git cannot remove the worktree.
    pub async fn remove(self, force: bool) -> Result<Worktree<Removed>, VcsError> {
        verify_git_config(&self.root)?;
        if !force {
            let status = run_git(
                &self.root,
                vec![
                    OsString::from("-C"),
                    self.path.as_os_str().to_owned(),
                    OsString::from("status"),
                    OsString::from("--porcelain"),
                    OsString::from("--untracked-files=all"),
                ],
            )
            .await?;
            if !status.stdout.is_empty() {
                return Err(VcsError::Git(
                    "dirty worktree removal requires force".into(),
                ));
            }
        }
        let mut args = vec![OsString::from("worktree"), OsString::from("remove")];
        if force {
            args.push(OsString::from("--force"));
        }
        args.push(OsString::from("--"));
        args.push(self.path.as_os_str().to_owned());
        run_git(&self.root, args).await?;
        Ok(Worktree {
            root: self.root,
            path: self.path,
            branch: self.branch,
            marker: std::marker::PhantomData,
        })
    }
}

impl Worktree<Removed> {
    /// The worktree path that was removed.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// An opened git repository with CAS snapshots and a durable undo journal.
#[derive(Debug)]
pub struct Repo {
    pub(crate) root: PathBuf,
    pub(crate) cas: Cas,
    pub(crate) journal: Journal,
    repository: gix::Repository,
}

impl Repo {
    /// Opens a worktree and creates its private Polaris CAS and undo journal.
    ///
    /// # Errors
    /// Returns [`VcsError::NotARepo`] for paths outside a worktree and propagates storage errors.
    pub fn open(root: &Path) -> Result<Self, VcsError> {
        let repository =
            gix::discover(root).map_err(|error| VcsError::NotARepo(error.to_string()))?;
        let root = repository
            .workdir()
            .ok_or_else(|| VcsError::NotARepo(root.display().to_string()))?
            .to_path_buf();
        let polaris_dir = root.join(".polaris");
        std::fs::create_dir_all(&polaris_dir)?;
        let cas = Cas::open(CasConfig::new(polaris_dir.join("cas")))?;
        let journal = Journal::open(polaris_dir.join("vcs-undo.json"))?;
        Ok(Self {
            root,
            cas,
            journal,
            repository,
        })
    }

    /// Computes a deterministic status from gix without changing the user's index.
    ///
    /// # Errors
    /// Returns [`VcsError::Gix`] when gix cannot complete the status walk.
    pub fn status(&self) -> Result<RepoStatus, VcsError> {
        verify_git_config(&self.root)?;
        let platform = self
            .repository
            .status(gix::progress::Discard)
            .map_err(|error| VcsError::Gix(error.to_string()))?
            .untracked_files(gix::status::UntrackedFiles::Files);
        let mut iterator = platform
            .into_iter(Vec::<gix::bstr::BString>::new())
            .map_err(|error| VcsError::Gix(error.to_string()))?;
        let mut entries = Vec::new();
        for item in &mut iterator {
            let item = item.map_err(|error| VcsError::Gix(error.to_string()))?;
            let path = std::str::from_utf8(item.location())
                .map_err(|error| VcsError::Gix(error.to_string()))?;
            if path == ".polaris" || path.starts_with(".polaris/") {
                continue;
            }
            let path = RepoPath::parse(path)?;
            let kind = match item {
                gix::status::Item::TreeIndex(_) => StatusKind::Staged,
                gix::status::Item::IndexWorktree(
                    gix::status::index_worktree::Item::DirectoryContents { .. },
                ) => StatusKind::Untracked,
                gix::status::Item::IndexWorktree(gix::status::index_worktree::Item::Rewrite {
                    ..
                }) => StatusKind::Untracked,
                gix::status::Item::IndexWorktree(
                    gix::status::index_worktree::Item::Modification { .. },
                ) => StatusKind::Worktree,
            };
            entries.push(StatusEntry { path, kind });
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        entries.dedup_by(|left, right| left.path == right.path && left.kind == right.kind);
        Ok(RepoStatus { entries })
    }

    /// Produces a bounded diff from HEAD to the current index and worktree. Paths always follow `--`.
    ///
    /// # Errors
    /// Returns [`VcsError::Git`] for process, timeout, output-limit, or git errors.
    pub async fn diff(&self, paths: &[RepoPath]) -> Result<DiffSet, VcsError> {
        verify_git_config(&self.root)?;
        let mut args = vec![
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--binary"),
            OsString::from("--no-color"),
            OsString::from("--find-renames"),
            OsString::from("HEAD"),
            OsString::from("--"),
        ];
        args.extend(paths.iter().map(|path| OsString::from(path.as_str())));
        let mut bytes = run_git(&self.root, args).await?.stdout;
        let status = self.status()?;
        let selected_untracked = status.entries.iter().filter(|entry| {
            entry.kind == StatusKind::Untracked
                && (paths.is_empty() || paths.iter().any(|path| path == &entry.path))
        });
        for entry in selected_untracked {
            let absolute = self.root.join(entry.path.as_str());
            let metadata = std::fs::symlink_metadata(&absolute)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                continue;
            }
            let empty = tempfile::NamedTempFile::new()?;
            let args = vec![
                OsString::from("diff"),
                OsString::from("--no-index"),
                OsString::from("--binary"),
                OsString::from("--no-color"),
                OsString::from("--no-ext-diff"),
                OsString::from("--no-textconv"),
                OsString::from("--"),
                empty.path().as_os_str().to_owned(),
                absolute.as_os_str().to_owned(),
            ];
            let mut untracked = run_git_allow_diff_exit(&self.root, args).await?.stdout;
            bytes.append(&mut untracked);
            if u64::try_from(bytes.len()).map_or(true, |size| size > MAX_GIT_OUTPUT) {
                return Err(VcsError::Git("git stdout exceeded 8 MiB output cap".into()));
            }
        }
        Ok(DiffSet { bytes })
    }

    /// Creates an isolated linked worktree under `.polaris/worktrees`.
    ///
    /// # Errors
    /// Returns [`VcsError::Git`] for duplicate branches or git worktree failures.
    pub async fn worktree_add(
        &self,
        name: &BranchName,
        base: &RevSpec,
    ) -> Result<Worktree<Clean>, VcsError> {
        verify_git_config(&self.root)?;
        let worktrees = self.root.join(".polaris").join("worktrees");
        std::fs::create_dir_all(&worktrees)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| VcsError::Git(error.to_string()))?
            .as_nanos();
        let key = blake3::hash(format!("{}:{stamp}", name.as_str()).as_bytes())
            .to_hex()
            .to_string();
        let path = worktrees.join(
            key.get(..16)
                .ok_or_else(|| VcsError::Git("generated worktree id is malformed".into()))?,
        );
        let args = vec![
            OsString::from("worktree"),
            OsString::from("add"),
            OsString::from("-b"),
            OsString::from(name.as_str()),
            path.as_os_str().to_owned(),
            OsString::from(base.as_str()),
        ];
        run_git(&self.root, args).await?;
        Ok(Worktree {
            root: self.root.clone(),
            path,
            branch: name.clone(),
            marker: std::marker::PhantomData,
        })
    }

    /// Returns the last agent action's guarded undo preview.
    pub fn undo_plan(&self) -> Result<crate::UndoPlan, VcsError> {
        self.journal.undo_plan(&self.root)
    }

    /// Persists the byte-exact pre-edit state for one kernel-assigned action sequence.
    pub fn snapshot_before_edit(
        &self,
        sequence: CheckpointSeq,
        paths: &[RepoPath],
    ) -> Result<(), VcsError> {
        self.journal
            .snapshot_before_edit(&self.root, &self.cas, sequence, paths)
    }

    /// Seals the resulting post-edit state used by later compare-and-swap undo.
    pub fn seal_edit(&self, sequence: CheckpointSeq) -> Result<(), VcsError> {
        self.journal.seal_edit(&self.root, sequence)
    }

    /// Restores the last sealed agent action without replacing intervening user edits.
    pub fn undo(&self) -> Result<crate::UndoReport, VcsError> {
        self.journal.undo(&self.root, &self.cas)
    }

    /// Plans a dry-run rollback to a checkpoint sequence.
    pub fn rollback_plan(&self, to: CheckpointSeq) -> Result<crate::RollbackPlan, VcsError> {
        self.journal.rollback_plan(&self.root, to)
    }

    /// Restores all sealed changes after `to`, refusing paths that no longer match the latest
    /// recorded post-edit state.
    pub fn rollback(&self, to: CheckpointSeq) -> Result<crate::UndoReport, VcsError> {
        self.journal.rollback(&self.root, &self.cas, to)
    }
}

#[derive(Debug)]
struct GitOutput {
    stdout: Vec<u8>,
}

async fn run_git(root: &Path, args: Vec<OsString>) -> Result<GitOutput, VcsError> {
    run_git_with_exit(root, args, false).await
}

async fn run_git_allow_diff_exit(root: &Path, args: Vec<OsString>) -> Result<GitOutput, VcsError> {
    run_git_with_exit(root, args, true).await
}

async fn run_git_with_exit(
    root: &Path,
    args: Vec<OsString>,
    allow_diff_exit: bool,
) -> Result<GitOutput, VcsError> {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    command.env(
        "GIT_CONFIG_GLOBAL",
        if cfg!(windows) { "NUL" } else { "/dev/null" },
    );
    command.env("GIT_OPTIONAL_LOCKS", "0");
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_PAGER", "cat");
    command.env("GIT_CONFIG_COUNT", "7");
    command.env("GIT_CONFIG_KEY_0", "core.autocrlf");
    command.env("GIT_CONFIG_VALUE_0", "false");
    command.env("GIT_CONFIG_KEY_1", "core.longpaths");
    command.env("GIT_CONFIG_VALUE_1", "true");
    command.env("GIT_CONFIG_KEY_2", "core.protectNTFS");
    command.env("GIT_CONFIG_VALUE_2", "true");
    command.env("GIT_CONFIG_KEY_3", "core.hooksPath");
    command.env(
        "GIT_CONFIG_VALUE_3",
        if cfg!(windows) { "NUL" } else { "/dev/null" },
    );
    command.env("GIT_CONFIG_KEY_4", "core.pager");
    command.env("GIT_CONFIG_VALUE_4", "cat");
    command.env("GIT_CONFIG_KEY_5", "diff.external");
    command.env("GIT_CONFIG_VALUE_5", "");
    command.env("GIT_CONFIG_KEY_6", "protocol.file.allow");
    command.env("GIT_CONFIG_VALUE_6", "never");
    command.args([
        "-c",
        "color.ui=false",
        "-c",
        "diff.external=",
        "-c",
        "core.quotepath=false",
    ]);
    command.args(args);
    let mut child = command
        .spawn()
        .map_err(|error| VcsError::Git(error.to_string()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| VcsError::Git("stdout pipe unavailable".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| VcsError::Git("stderr pipe unavailable".into()))?;
    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_GIT_OUTPUT.saturating_add(1))
            .read_to_end(&mut bytes)
            .await
            .map(|_| bytes)
    });
    let stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr
            .take(64 * 1024)
            .read_to_end(&mut bytes)
            .await
            .map(|_| bytes)
    });
    let status = tokio::time::timeout(GIT_TIMEOUT, child.wait())
        .await
        .map_err(|_| VcsError::Git("git command exceeded 30 second timeout".into()))?
        .map_err(|error| VcsError::Git(error.to_string()))?;
    let stdout = stdout_task
        .await
        .map_err(|error| VcsError::Git(error.to_string()))?
        .map_err(|error| VcsError::Git(error.to_string()))?;
    let stderr = stderr_task
        .await
        .map_err(|error| VcsError::Git(error.to_string()))?
        .map_err(|error| VcsError::Git(error.to_string()))?;
    if match u64::try_from(stdout.len()) {
        Ok(size) => size > MAX_GIT_OUTPUT,
        Err(_) => true,
    } {
        return Err(VcsError::Git("git stdout exceeded 8 MiB output cap".into()));
    }
    if !status.success() && !(allow_diff_exit && status.code() == Some(1)) {
        let message = String::from_utf8_lossy(&stderr)
            .chars()
            .take(2048)
            .collect::<String>();
        return Err(VcsError::Git(message));
    }
    Ok(GitOutput { stdout })
}

fn verify_git_config(root: &Path) -> Result<(), VcsError> {
    let git_dir = gix::discover(root)
        .map_err(|error| VcsError::Gix(error.to_string()))?
        .git_dir()
        .to_path_buf();
    let mut config_paths = vec![git_dir.join("config"), git_dir.join("config.worktree")];
    let common_dir = git_dir.join("commondir");
    if let Ok(value) = std::fs::read_to_string(&common_dir) {
        let common = PathBuf::from(value.trim());
        let common = if common.is_absolute() {
            common
        } else {
            git_dir.join(common)
        };
        config_paths.push(common.join("config"));
    } else if std::fs::symlink_metadata(&common_dir)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(VcsError::Git(
            "linked worktree common-dir pointer is a symlink".into(),
        ));
    }
    for config in config_paths {
        let contents = match std::fs::read_to_string(config) {
            Ok(contents) => contents.to_ascii_lowercase(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(VcsError::Io(error)),
        };
        if contents.lines().any(|line| {
            let line = line.trim();
            !line.starts_with('#')
                && !line.starts_with(';')
                && (line.starts_with("[filter")
                    || line.starts_with("[include")
                    || line.starts_with("[includeif")
                    || line.contains("smudge")
                    || line.contains("clean")
                    || line.contains("process"))
        }) {
            return Err(VcsError::Git(
                "repository config enables filters or includes; refusing external execution".into(),
            ));
        }
    }
    Ok(())
}

fn _os_str(_: &OsStr) {}
