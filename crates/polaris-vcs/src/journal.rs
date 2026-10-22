//! Durable CAS snapshots and compare-before-restore undo/rollback.

use std::fs::{self};
use std::io::Write;
use std::path::{Path, PathBuf};

use polaris_sdk::ids::Cid;
use polaris_storage_cas::Cas;
use serde::{Deserialize, Serialize};

use crate::paths::RepoPath;
use crate::VcsError;

/// Kernel-assigned action sequence number.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CheckpointSeq(u64);

impl CheckpointSeq {
    /// Wraps a sequence assigned by the kernel/event log.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The numeric sequence.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The before and sealed after state of a path in one agent action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSnapshot {
    /// Repository-relative path.
    pub path: String,
    /// CAS id of the exact pre-edit bytes, or `None` if the path was absent.
    pub before: Option<Cid>,
    /// Whether the pre-edit file was executable on Unix.
    pub before_executable: bool,
    /// BLAKE3 of the post-edit file, or `None` if the action removed it.
    pub after: Option<Cid>,
    /// Whether the post-edit file is executable on Unix.
    pub after_executable: bool,
}

/// Preview of one `/undo` operation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UndoPlan {
    /// Sequence to undo.
    pub sequence: Option<CheckpointSeq>,
    /// Paths and their expected before/after ids.
    pub files: Vec<FileSnapshot>,
    /// Paths whose current contents no longer match the recorded pre/post state.
    pub conflicts: Vec<String>,
}

/// Preview of `/rollback <checkpoint>`; constructing it changes no files.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RollbackPlan {
    /// The checkpoint that remains after rollback.
    pub checkpoint: CheckpointSeq,
    /// Net per-path state changes to apply.
    pub files: Vec<FileSnapshot>,
    /// Paths whose current contents no longer match the recorded post-action state.
    pub conflicts: Vec<String>,
}

/// Paths restored by an undo or rollback.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UndoReport {
    /// Paths restored.
    pub restored: Vec<RepoPath>,
    /// Paths already in the requested state after an interrupted restore.
    pub already_restored: Vec<RepoPath>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct JournalData {
    actions: Vec<ActionRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActionRecord {
    sequence: CheckpointSeq,
    sealed: bool,
    consumed: bool,
    files: Vec<FileSnapshot>,
}

/// The on-disk journal. It contains only CAS ids and path state, never file contents.
#[derive(Debug)]
pub(crate) struct Journal {
    path: PathBuf,
    data: std::sync::Mutex<JournalData>,
}

impl Journal {
    pub(crate) fn open(path: PathBuf) -> Result<Self, VcsError> {
        let data = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| VcsError::Journal(error.to_string()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => JournalData::default(),
            Err(error) => return Err(VcsError::Io(error)),
        };
        Ok(Self { path, data: std::sync::Mutex::new(data) })
    }

    pub(crate) fn snapshot_before_edit(
        &self,
        root: &Path,
        cas: &Cas,
        sequence: CheckpointSeq,
        paths: &[RepoPath],
    ) -> Result<(), VcsError> {
        let mut data = self.data.lock().map_err(|_| VcsError::Journal("journal lock poisoned".into()))?;
        if data.actions.iter().any(|action| action.sequence == sequence) {
            return Err(VcsError::Journal(format!("sequence {} already exists", sequence.get())));
        }
        let mut files = Vec::with_capacity(paths.len());
        for path in paths {
            let absolute = absolute(root, path)?;
            let state = read_state(&absolute)?;
            let before = match state {
                Some((bytes, executable)) => Some((cas.put(&bytes)?, executable)),
                None => None,
            };
            files.push(FileSnapshot {
                path: path.as_str().to_owned(),
                before: before.as_ref().map(|(cid, _)| *cid),
                before_executable: before.is_some_and(|(_, executable)| executable),
                after: None,
                after_executable: false,
            });
        }
        data.actions.push(ActionRecord { sequence, sealed: false, consumed: false, files });
        data.actions.sort_by_key(|action| action.sequence);
        self.persist(&data)
    }

    pub(crate) fn seal_edit(&self, root: &Path, sequence: CheckpointSeq) -> Result<(), VcsError> {
        let mut data = self.data.lock().map_err(|_| VcsError::Journal("journal lock poisoned".into()))?;
        let action = data.actions.iter_mut().find(|action| action.sequence == sequence)
            .ok_or(VcsError::UnknownCheckpoint(sequence.get()))?;
        if action.sealed || action.consumed {
            return Err(VcsError::Journal(format!("sequence {} is already sealed or consumed", sequence.get())));
        }
        for file in &mut action.files {
            let path = RepoPath::parse(&file.path)?;
            let state = read_state(&absolute(root, &path)?)?;
            file.after = state.as_ref().map(|(bytes, _)| Cid::from_bytes(*blake3::hash(bytes).as_bytes()));
            file.after_executable = state.is_some_and(|(_, executable)| executable);
        }
        action.sealed = true;
        self.persist(&data)
    }

    pub(crate) fn undo_plan(&self, root: &Path) -> Result<UndoPlan, VcsError> {
        let data = self.data.lock().map_err(|_| VcsError::Journal("journal lock poisoned".into()))?;
        let action = data.actions.iter().rev().find(|action| action.sealed && !action.consumed);
        match action {
            Some(action) => {
                let conflicts = plan_conflicts(root, &action.files, true)?;
                Ok(UndoPlan { sequence: Some(action.sequence), files: action.files.clone(), conflicts })
            }
            None => Ok(UndoPlan::default()),
        }
    }

    pub(crate) fn undo(&self, root: &Path, cas: &Cas) -> Result<UndoReport, VcsError> {
        let mut data = self.data.lock().map_err(|_| VcsError::Journal("journal lock poisoned".into()))?;
        let index = data.actions.iter().rposition(|action| action.sealed && !action.consumed)
            .ok_or_else(|| VcsError::Journal("there is no sealed action to undo".into()))?;
        let action = data.actions.get(index).ok_or_else(|| VcsError::Journal("undo action disappeared".into()))?;
        let report = restore_files(root, cas, &action.files)?;
        if let Some(action) = data.actions.get_mut(index) {
            action.consumed = true;
        }
        self.persist(&data)?;
        Ok(report)
    }

    pub(crate) fn rollback_plan(&self, root: &Path, to: CheckpointSeq) -> Result<RollbackPlan, VcsError> {
        let data = self.data.lock().map_err(|_| VcsError::Journal("journal lock poisoned".into()))?;
        if to.get() != 0 && !data.actions.iter().any(|action| action.sequence == to) {
            return Err(VcsError::UnknownCheckpoint(to.get()));
        }
        let mut net = std::collections::BTreeMap::<String, FileSnapshot>::new();
        for action in data.actions.iter().filter(|action| action.sealed && !action.consumed && action.sequence > to) {
            for file in &action.files {
                let slot = net.entry(file.path.clone()).or_insert_with(|| file.clone());
                slot.after = file.after;
                slot.after_executable = file.after_executable;
            }
        }
        let mut files: Vec<FileSnapshot> = net.into_values().collect();
        files.reverse();
        let conflicts = plan_conflicts(root, &files, false)?;
        Ok(RollbackPlan { checkpoint: to, files, conflicts })
    }

    pub(crate) fn rollback(&self, root: &Path, cas: &Cas, to: CheckpointSeq) -> Result<UndoReport, VcsError> {
        let plan = self.rollback_plan(root, to)?;
        let mut data = self.data.lock().map_err(|_| VcsError::Journal("journal lock poisoned".into()))?;
        let report = restore_files(root, cas, &plan.files)?;
        for action in data.actions.iter_mut().filter(|action| action.sealed && action.sequence > to) {
            action.consumed = true;
        }
        self.persist(&data)?;
        Ok(report)
    }

    fn persist(&self, data: &JournalData) -> Result<(), VcsError> {
        let bytes = serde_json::to_vec(data).map_err(|error| VcsError::Journal(error.to_string()))?;
        let parent = self.path.parent().ok_or_else(|| VcsError::Journal("journal path has no parent".into()))?;
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(&self.path).map(|_| ()).map_err(|error| VcsError::Io(error.error))
    }
}

fn plan_conflicts(root: &Path, files: &[FileSnapshot], allow_before_state: bool) -> Result<Vec<String>, VcsError> {
    let mut conflicts = Vec::new();
    for file in files {
        let path = RepoPath::parse(&file.path)?;
        let current = read_state(&absolute(root, &path)?)?;
        let current_id = current.as_ref().map(|(bytes, _)| Cid::from_bytes(*blake3::hash(bytes).as_bytes()));
        let current_exec = current.is_some_and(|(_, executable)| executable);
        let matches_after = current_id == file.after && current_exec == file.after_executable;
        let matches_before = allow_before_state && current_id == file.before && current_exec == file.before_executable;
        if !matches_after && !matches_before {
            conflicts.push(file.path.clone());
        }
    }
    Ok(conflicts)
}

fn restore_files(root: &Path, cas: &Cas, files: &[FileSnapshot]) -> Result<UndoReport, VcsError> {
    for file in files {
        let path = RepoPath::parse(&file.path)?;
        let current = read_state(&absolute(root, &path)?)?;
        let current_id = current.as_ref().map(|(bytes, _)| Cid::from_bytes(*blake3::hash(bytes).as_bytes()));
        let current_exec = current.is_some_and(|(_, executable)| executable);
        let already = current_id == file.before && current_exec == file.before_executable;
        if !already && (current_id != file.after || current_exec != file.after_executable) {
            return Err(VcsError::Conflict(file.path.clone()));
        }
    }
    let mut report = UndoReport::default();
    for file in files {
        let path = RepoPath::parse(&file.path)?;
        let absolute_path = absolute(root, &path)?;
        let current = read_state(&absolute_path)?;
        let current_id = current.as_ref().map(|(bytes, _)| Cid::from_bytes(*blake3::hash(bytes).as_bytes()));
        let current_exec = current.is_some_and(|(_, executable)| executable);
        if current_id == file.before && current_exec == file.before_executable {
            report.already_restored.push(path);
            continue;
        }
        install_guarded(&absolute_path, file.after, file.after_executable, file.before, file.before_executable, cas)?;
        report.restored.push(path);
    }
    Ok(report)
}

fn install_guarded(
    path: &Path,
    expected_id: Option<Cid>,
    expected_exec: bool,
    desired_id: Option<Cid>,
    desired_exec: bool,
    cas: &Cas,
) -> Result<(), VcsError> {
    let parent = path.parent().ok_or_else(|| VcsError::Journal("repository path has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let path_key = blake3::hash(path.as_os_str().to_string_lossy().as_bytes()).to_hex().to_string();
    let quarantine = parent.join(format!(".polaris-undo-{}-{}.tmp", std::process::id(), path_key));
    let present = fs::symlink_metadata(path).is_ok();
    if present {
        if quarantine.exists() {
            return Err(VcsError::Journal("restore quarantine path already exists".into()));
        }
        fs::rename(path, &quarantine)?;
        let current = read_state(&quarantine)?;
        let current_id = current.as_ref().map(|(bytes, _)| Cid::from_bytes(*blake3::hash(bytes).as_bytes()));
        let current_exec = current.is_some_and(|(_, executable)| executable);
        if current_id != expected_id || current_exec != expected_exec {
            if !path.exists() {
                fs::rename(&quarantine, path)?;
            }
            return Err(VcsError::Conflict(path.display().to_string()));
        }
    } else if expected_id.is_some() {
        return Err(VcsError::Conflict(path.display().to_string()));
    }
    if let Some(cid) = desired_id {
        let bytes = cas.get(&cid)?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        set_executable(temp.path(), desired_exec)?;
        temp.persist_noclobber(path).map_err(|error| VcsError::Io(error.error))?;
    }
    if present {
        fs::remove_file(&quarantine)?;
    }
    Ok(())
}

fn absolute(root: &Path, path: &RepoPath) -> Result<PathBuf, VcsError> {
    let root = root.canonicalize()?;
    let mut absolute = root.clone();
    for component in path.components() {
        absolute.push(component);
        match fs::symlink_metadata(&absolute) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(VcsError::InvalidRef(path.as_str().to_owned()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(VcsError::Io(error)),
        }
    }
    if !absolute.starts_with(&root) {
        return Err(VcsError::InvalidRef(path.as_str().to_owned()));
    }
    Ok(absolute)
}

fn read_state(path: &Path) -> Result<Option<(Vec<u8>, bool)>, VcsError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(VcsError::Io(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(VcsError::InvalidRef(path.display().to_string()));
    }
    let bytes = fs::read(path)?;
    Ok(Some((bytes, executable(&metadata))))
}

#[cfg(unix)]
fn executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_metadata: &fs::Metadata) -> bool {
    false
}

fn set_executable(path: &Path, value: bool) -> Result<(), VcsError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if value { 0o755 } else { 0o644 };
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (path, value);
    Ok(())
}
