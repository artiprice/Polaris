//! Fixed-protocol, compare-and-swap filesystem mutations for the sandbox workspace.
//!
//! File bytes are uploaded by the host into the private staging directory. This process holds
//! the per-workspace advisory lock while it rechecks the destination and performs a same-filesystem
//! rename or conditional unlink. The lock coordinates cooperating Polaris writers; a process that
//! ignores the lock can still race filesystem operations.

use std::io::{self, Read, Write};
use std::process::ExitCode;

use polaris_sdk::registry::{CommitOutcome, SandboxCommitRequest};

const MAX_REQUEST_BYTES: u64 = 64 * 1024;

/// Create or validate only the private staging hierarchy for `sync-cas` uploads.
pub fn run_init(root: &str) -> ExitCode {
    match init_workspace(root) {
        Ok(()) => {
            let mut stdout = io::stdout().lock();
            match writeln!(stdout, "ready") {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => ExitCode::FAILURE,
            }
        }
        Err(_) => fail("sync-init rejected the workspace or staging directory"),
    }
}

fn init_workspace(root: &str) -> Result<(), CommitError> {
    #[cfg(target_os = "linux")]
    {
        linux::init_workspace(root)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = root;
        Err(CommitError::Unsupported)
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn prepare_workspace(root: &str) -> Result<(), String> {
    init_workspace(root).map_err(|error| error.to_string())
}

/// Read one bounded request and execute it. Exit status: 0 applied, 3 conflict, 2 invalid/error.
pub fn run() -> ExitCode {
    let mut input = Vec::new();
    let read_result = io::stdin()
        .lock()
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut input);
    if read_result.is_err() || u64::try_from(input.len()).map_or(true, |n| n > MAX_REQUEST_BYTES) {
        return fail("request exceeds the input limit or could not be read");
    }
    let request: SandboxCommitRequest = match serde_json::from_slice(&input) {
        Ok(request) => request,
        Err(_) => return fail("invalid sync-cas request"),
    };
    match commit(&request) {
        Ok(CommitOutcome::Applied) => {
            let mut stdout = io::stdout().lock();
            match writeln!(stdout, "applied") {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => ExitCode::FAILURE,
            }
        }
        Ok(CommitOutcome::Conflict) => {
            let mut stdout = io::stdout().lock();
            match writeln!(stdout, "conflict") {
                Ok(()) => ExitCode::from(3),
                Err(_) => ExitCode::FAILURE,
            }
        }
        Err(_) => fail("sync-cas rejected the request or filesystem operation"),
    }
}

fn fail(message: &str) -> ExitCode {
    let mut stderr = io::stderr().lock();
    match writeln!(stderr, "{message}") {
        Ok(()) => ExitCode::from(2),
        Err(_) => ExitCode::FAILURE,
    }
}

#[derive(Debug, thiserror::Error)]
enum CommitError {
    #[cfg(target_os = "linux")]
    #[error("invalid request")]
    Invalid,
    #[cfg(not(target_os = "linux"))]
    #[error("unsupported platform")]
    Unsupported,
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[cfg(target_os = "linux")]
mod linux {
    use std::fs::{self, OpenOptions};
    use std::io::{self, Read};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Component, Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    #[cfg(test)]
    use std::sync::{Arc, Barrier};

    use fs4::FileExt;
    use polaris_sdk::ids::Cid;
    use polaris_sdk::registry::{
        CommitOutcome, SandboxCommitAction, SandboxCommitRequest, SandboxFileState,
    };

    use super::CommitError;

    const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
    const MAX_PATH_BYTES: usize = 4096;
    const O_NOFOLLOW: i32 = 0o400000;
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    pub(super) fn commit(request: &SandboxCommitRequest) -> Result<CommitOutcome, CommitError> {
        let root = checked_root(&request.root)?;
        let relative = checked_relative_path(&request.relative_path)?;
        ensure_not_reserved(&relative)?;

        let polaris = root.join(".polaris");
        let sync_dir = polaris.join("sync");
        let staging = sync_dir.join("staging");
        let owner = fs::symlink_metadata(&root)?.uid();
        ensure_private_dir(&polaris, owner)?;
        ensure_private_dir(&sync_dir, owner)?;
        ensure_private_dir(&staging, owner)?;

        let lock_path = sync_dir.join("lock");
        let lock = open_private_lock(&lock_path, owner)?;
        FileExt::lock(&lock)?;

        let target = checked_target(&root, &relative)?;
        let actual = observe(&target)?;
        if actual.as_ref() != request.expected.as_ref() {
            FileExt::unlock(&lock)?;
            return Ok(CommitOutcome::Conflict);
        }

        match &request.action {
            SandboxCommitAction::Delete => {
                if actual.is_some() {
                    fs::remove_file(&target)?;
                }
            }
            SandboxCommitAction::File {
                staged_path,
                hash,
                executable,
            } => {
                let staged = checked_stage(&staging, staged_path)?;
                let metadata = fs::symlink_metadata(&staged)?;
                if !metadata.file_type().is_file()
                    || metadata.len() > MAX_FILE_BYTES
                    || metadata.nlink() != 1
                {
                    return Err(CommitError::Invalid);
                }
                if &hash_file(&staged)? != hash {
                    return Err(CommitError::Invalid);
                }
                ensure_target_parents(&root, &relative)?;
                recheck_target_parents(&root, &relative)?;
                fs::set_permissions(
                    &staged,
                    fs::Permissions::from_mode(if *executable { 0o755 } else { 0o644 }),
                )?;
                fs::rename(&staged, &target)?;
            }
            SandboxCommitAction::Link { target: link, hash } => {
                if blake3::hash(link.as_bytes()).as_bytes() != hash.as_bytes()
                    || !safe_link_target(relative.parent().unwrap_or_else(|| Path::new("")), link)
                {
                    return Err(CommitError::Invalid);
                }
                ensure_target_parents(&root, &relative)?;
                recheck_target_parents(&root, &relative)?;
                let temporary = unique_temp(&target)?;
                std::os::unix::fs::symlink(link, &temporary)?;
                if let Err(error) = fs::rename(&temporary, &target) {
                    let _ = fs::remove_file(&temporary);
                    return Err(CommitError::Io(error));
                }
            }
        }
        FileExt::unlock(&lock)?;
        Ok(CommitOutcome::Applied)
    }

    pub(super) fn init_workspace(root: &str) -> Result<(), CommitError> {
        let root = checked_root(root)?;
        let owner = fs::symlink_metadata(&root)?.uid();
        let polaris = root.join(".polaris");
        let sync_dir = polaris.join("sync");
        let staging = sync_dir.join("staging");
        ensure_private_dir(&polaris, owner)?;
        ensure_private_dir(&sync_dir, owner)?;
        ensure_private_dir(&staging, owner)?;
        Ok(())
    }

    fn checked_root(root: &str) -> Result<PathBuf, CommitError> {
        if root.is_empty()
            || root.len() > MAX_PATH_BYTES
            || !root.starts_with('/')
            || root.contains('\0')
        {
            return Err(CommitError::Invalid);
        }
        let path = PathBuf::from(root);
        if path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        {
            return Err(CommitError::Invalid);
        }
        let canonical = fs::canonicalize(&path)?;
        if canonical != path || !fs::symlink_metadata(&canonical)?.is_dir() {
            return Err(CommitError::Invalid);
        }
        Ok(canonical)
    }

    fn checked_relative_path(path: &str) -> Result<PathBuf, CommitError> {
        if path.is_empty()
            || path.len() > MAX_PATH_BYTES
            || path.starts_with('/')
            || path.contains(['\\', '\0'])
        {
            return Err(CommitError::Invalid);
        }
        let mut out = PathBuf::new();
        for part in path.split('/') {
            if part.is_empty() || part == "." || part == ".." || part.len() > 255 {
                return Err(CommitError::Invalid);
            }
            out.push(part);
        }
        Ok(out)
    }

    fn ensure_not_reserved(relative: &Path) -> Result<(), CommitError> {
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(CommitError::Invalid);
            };
            let name = name.to_str().ok_or(CommitError::Invalid)?;
            if name.eq_ignore_ascii_case(".git") || name.eq_ignore_ascii_case(".polaris") {
                return Err(CommitError::Invalid);
            }
        }
        Ok(())
    }

    fn checked_target(root: &Path, relative: &Path) -> Result<PathBuf, CommitError> {
        let mut current = root.to_path_buf();
        let components: Vec<_> = relative.components().collect();
        for (index, component) in components.iter().enumerate() {
            let Component::Normal(name) = component else {
                return Err(CommitError::Invalid);
            };
            current.push(name);
            if index + 1 == components.len() {
                break;
            }
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_dir() => {}
                Ok(_) => return Err(CommitError::Invalid),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(CommitError::Io(error)),
            }
        }
        Ok(current)
    }

    fn ensure_target_parents(root: &Path, relative: &Path) -> Result<(), CommitError> {
        let mut current = root.to_path_buf();
        let components: Vec<_> = relative.components().collect();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            let Component::Normal(name) = component else {
                return Err(CommitError::Invalid);
            };
            current.push(name);
            match fs::create_dir(&current) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(CommitError::Io(error)),
            }
            let metadata = fs::symlink_metadata(&current)?;
            if !metadata.file_type().is_dir() {
                return Err(CommitError::Invalid);
            }
        }
        Ok(())
    }

    fn recheck_target_parents(root: &Path, relative: &Path) -> Result<(), CommitError> {
        let mut current = root.to_path_buf();
        let components: Vec<_> = relative.components().collect();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            let Component::Normal(name) = component else {
                return Err(CommitError::Invalid);
            };
            current.push(name);
            let metadata = fs::symlink_metadata(&current)?;
            if !metadata.file_type().is_dir() {
                return Err(CommitError::Invalid);
            }
        }
        Ok(())
    }

    fn observe(path: &Path) -> Result<Option<SandboxFileState>, CommitError> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(CommitError::Io(error)),
        };
        if metadata.file_type().is_symlink() {
            let target = fs::read_link(path)?
                .into_os_string()
                .into_string()
                .map_err(|_| CommitError::Invalid)?;
            if target.len() > MAX_PATH_BYTES {
                return Err(CommitError::Invalid);
            }
            return Ok(Some(SandboxFileState {
                hash: cid(blake3::hash(target.as_bytes()).as_bytes()),
                executable: false,
                link_target: Some(target),
            }));
        }
        if !metadata.file_type().is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err(CommitError::Invalid);
        }
        Ok(Some(SandboxFileState {
            hash: hash_file(path)?,
            executable: metadata.permissions().mode() & 0o111 != 0,
            link_target: None,
        }))
    }

    fn hash_file(path: &Path) -> Result<Cid, CommitError> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err(CommitError::Invalid);
        }
        let mut options = OpenOptions::new();
        options.read(true).custom_flags(O_NOFOLLOW);
        let mut file = options.open(path)?;
        let mut hasher = blake3::Hasher::new();
        let mut count = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            count = count
                .checked_add(u64::try_from(read).map_err(|_| CommitError::Invalid)?)
                .ok_or(CommitError::Invalid)?;
            if count > MAX_FILE_BYTES {
                return Err(CommitError::Invalid);
            }
            hasher.update(&buffer[..read]);
        }
        Ok(cid(hasher.finalize().as_bytes()))
    }

    fn cid(bytes: &[u8; 32]) -> Cid {
        Cid::from_bytes(*bytes)
    }

    fn ensure_private_dir(path: &Path, expected_owner: u32) -> Result<(), CommitError> {
        match fs::create_dir(path) {
            Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(0o700))?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(CommitError::Io(error)),
        }
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_dir()
            || metadata.permissions().mode() & 0o077 != 0
            || metadata.uid() != expected_owner
        {
            return Err(CommitError::Invalid);
        }
        Ok(())
    }

    fn open_private_lock(path: &Path, expected_owner: u32) -> Result<fs::File, CommitError> {
        match fs::symlink_metadata(path) {
            Ok(metadata)
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.uid() != expected_owner
                    || metadata.permissions().mode() & 0o077 != 0 =>
            {
                return Err(CommitError::Invalid);
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(CommitError::Io(error)),
        }
        let file = match OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(O_NOFOLLOW)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(path)?;
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.uid() != expected_owner
                    || metadata.permissions().mode() & 0o077 != 0
                {
                    return Err(CommitError::Invalid);
                }
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(O_NOFOLLOW)
                    .open(path)?
            }
            Err(error) => return Err(CommitError::Io(error)),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != expected_owner
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(CommitError::Invalid);
        }
        Ok(file)
    }

    fn reject_existing_symlink(path: &Path) -> Result<(), CommitError> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => Err(CommitError::Invalid),
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(CommitError::Io(error)),
        }
    }

    fn checked_stage(staging: &Path, staged_path: &str) -> Result<PathBuf, CommitError> {
        if staged_path.len() > MAX_PATH_BYTES || staged_path.contains(['\\', '\0']) {
            return Err(CommitError::Invalid);
        }
        let candidate = Path::new(staged_path);
        let name = candidate.file_name().ok_or(CommitError::Invalid)?;
        if candidate.parent() != Some(staging)
            || name
                .to_str()
                .map_or(true, |n| n.is_empty() || n == "." || n == "..")
        {
            return Err(CommitError::Invalid);
        }
        reject_existing_symlink(candidate)?;
        Ok(candidate.to_path_buf())
    }

    fn unique_temp(target: &Path) -> Result<PathBuf, CommitError> {
        let parent = target.parent().ok_or(CommitError::Invalid)?;
        let name = target
            .file_name()
            .ok_or(CommitError::Invalid)?
            .to_string_lossy();
        for _ in 0..16 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(
                ".{name}.polaris-sync-{}-{sequence}",
                std::process::id()
            ));
            if fs::symlink_metadata(&candidate).is_err() {
                return Ok(candidate);
            }
        }
        Err(CommitError::Invalid)
    }

    fn safe_link_target(parent: &Path, target: &str) -> bool {
        if target.is_empty()
            || target.len() > MAX_PATH_BYTES
            || target.starts_with('/')
            || target.contains(['\\', '\0'])
        {
            return false;
        }
        let mut depth = parent.components().count();
        for part in target.split('/') {
            match part {
                "" | "." => {}
                ".." if depth > 0 => depth -= 1,
                ".." => return false,
                _ if part.len() <= 255 => depth += 1,
                _ => return false,
            }
        }
        true
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        struct TempWorkspace(PathBuf);

        impl TempWorkspace {
            fn new() -> Self {
                let stamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos());
                let path = std::env::temp_dir()
                    .join(format!("polaris-sync-cas-{}-{stamp}", std::process::id()));
                fs::create_dir(&path).expect("create temp workspace");
                Self(path)
            }

            fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempWorkspace {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        fn prepare_staging(root: &Path) -> PathBuf {
            let path = root.join(".polaris/sync/staging");
            init_workspace(&root.to_string_lossy()).expect("initialize staging");
            path
        }

        #[test]
        fn sync_init_creates_only_private_staging_hierarchy_and_is_idempotent() {
            let workspace = TempWorkspace::new();
            let root = workspace.path();
            init_workspace(&root.to_string_lossy()).expect("initialize staging");
            init_workspace(&root.to_string_lossy()).expect("validate existing staging");

            for path in [
                root.join(".polaris"),
                root.join(".polaris/sync"),
                root.join(".polaris/sync/staging"),
            ] {
                let metadata = fs::symlink_metadata(path).expect("directory exists");
                assert!(metadata.file_type().is_dir());
                assert_eq!(metadata.permissions().mode() & 0o077, 0);
            }
            assert!(!root.join(".polaris/sync/lock").exists());
        }

        #[test]
        fn sync_init_rejects_unsafe_existing_directory_without_chmod() {
            let workspace = TempWorkspace::new();
            let root = workspace.path();
            let staging = root.join(".polaris/sync/staging");
            fs::create_dir_all(&staging).expect("create unsafe staging");
            fs::set_permissions(&staging, fs::Permissions::from_mode(0o755))
                .expect("make staging unsafe");

            assert!(init_workspace(&root.to_string_lossy()).is_err());
            assert_eq!(
                fs::metadata(&staging)
                    .expect("staging remains")
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
        }

        #[test]
        fn sync_init_rejects_noncanonical_or_missing_root() {
            let workspace = TempWorkspace::new();
            let root = workspace.path();
            let noncanonical = format!("{}/missing/..", root.display());
            assert!(init_workspace(&noncanonical).is_err());
            assert!(init_workspace("relative/path").is_err());
            assert!(!root.join(".polaris").exists());
        }

        fn request(
            root: &Path,
            relative_path: &str,
            expected: Option<SandboxFileState>,
            action: SandboxCommitAction,
        ) -> SandboxCommitRequest {
            SandboxCommitRequest {
                root: root.to_string_lossy().into_owned(),
                relative_path: relative_path.to_owned(),
                expected,
                action,
            }
        }

        fn state(bytes: &[u8], executable: bool) -> SandboxFileState {
            SandboxFileState {
                hash: cid(blake3::hash(bytes).as_bytes()),
                executable,
                link_target: None,
            }
        }

        #[test]
        fn staged_file_is_hash_checked_and_atomically_installed() {
            let workspace = TempWorkspace::new();
            let root = workspace.path();
            let staging = prepare_staging(root);
            let staged = staging.join("stage-a");
            fs::write(&staged, b"new\r\nbytes").expect("write staged bytes");
            fs::create_dir(root.join("nested")).expect("create nested target dir");
            fs::write(root.join("nested/file.txt"), b"old").expect("write old target");
            let req = request(
                root,
                "nested/file.txt",
                Some(state(b"old", false)),
                SandboxCommitAction::File {
                    staged_path: staged.to_string_lossy().into_owned(),
                    hash: cid(blake3::hash(b"new\r\nbytes").as_bytes()),
                    executable: false,
                },
            );
            assert_eq!(commit(&req).expect("commit"), CommitOutcome::Applied);
            assert_eq!(
                fs::read(root.join("nested/file.txt")).expect("read file"),
                b"new\r\nbytes"
            );
            assert!(!staged.exists());

            let executable_stage = staging.join("stage-executable");
            fs::write(&executable_stage, b"#!/bin/sh\n").expect("write executable stage");
            let executable = request(
                root,
                "nested/script",
                None,
                SandboxCommitAction::File {
                    staged_path: executable_stage.to_string_lossy().into_owned(),
                    hash: cid(blake3::hash(b"#!/bin/sh\n").as_bytes()),
                    executable: true,
                },
            );
            assert_eq!(
                commit(&executable).expect("install executable"),
                CommitOutcome::Applied
            );
            assert_ne!(
                fs::metadata(root.join("nested/script"))
                    .expect("executable metadata")
                    .permissions()
                    .mode()
                    & 0o111,
                0
            );
        }

        #[test]
        fn changed_destination_and_staged_hash_mismatch_preserve_destination() {
            let workspace = TempWorkspace::new();
            let root = workspace.path();
            fs::write(root.join("file"), b"intervening").expect("write intervening file");
            let staging = prepare_staging(root);
            let staged = staging.join("stage-b");
            fs::write(&staged, b"candidate").expect("write staged bytes");
            let req = request(
                root,
                "file",
                Some(state(b"old", false)),
                SandboxCommitAction::File {
                    staged_path: staged.to_string_lossy().into_owned(),
                    hash: cid(blake3::hash(b"candidate").as_bytes()),
                    executable: false,
                },
            );
            assert_eq!(commit(&req).expect("conflict"), CommitOutcome::Conflict);
            assert_eq!(
                fs::read(root.join("file")).expect("read preserved file"),
                b"intervening"
            );

            let bad_hash = request(
                root,
                "file",
                Some(state(b"intervening", false)),
                SandboxCommitAction::File {
                    staged_path: staged.to_string_lossy().into_owned(),
                    hash: cid(blake3::hash(b"incorrect").as_bytes()),
                    executable: false,
                },
            );
            assert!(commit(&bad_hash).is_err());
            assert_eq!(
                fs::read(root.join("file")).expect("read preserved file"),
                b"intervening"
            );
        }

        #[test]
        fn conditional_delete_and_link_install_preserve_exact_state() {
            let workspace = TempWorkspace::new();
            let root = workspace.path();
            fs::write(root.join("gone"), b"exact").expect("write file");
            let delete = request(
                root,
                "gone",
                Some(state(b"exact", false)),
                SandboxCommitAction::Delete,
            );
            assert_eq!(commit(&delete).expect("delete"), CommitOutcome::Applied);
            assert!(!root.join("gone").exists());

            fs::write(root.join("changed-after-plan"), b"intervening").expect("write changed file");
            let stale_delete = request(
                root,
                "changed-after-plan",
                Some(state(b"planned", false)),
                SandboxCommitAction::Delete,
            );
            assert_eq!(
                commit(&stale_delete).expect("stale delete"),
                CommitOutcome::Conflict
            );
            assert_eq!(
                fs::read(root.join("changed-after-plan")).expect("preserved changed file"),
                b"intervening"
            );

            let link_target = "inside.txt";
            let link = request(
                root,
                "link",
                None,
                SandboxCommitAction::Link {
                    target: link_target.to_owned(),
                    hash: cid(blake3::hash(link_target.as_bytes()).as_bytes()),
                },
            );
            assert_eq!(commit(&link).expect("install link"), CommitOutcome::Applied);
            assert_eq!(
                fs::read_link(root.join("link")).expect("read link"),
                PathBuf::from(link_target)
            );

            let parent_relative_link = "../inside.txt";
            let nested_link = request(
                root,
                "nested/link",
                None,
                SandboxCommitAction::Link {
                    target: parent_relative_link.to_owned(),
                    hash: cid(blake3::hash(parent_relative_link.as_bytes()).as_bytes()),
                },
            );
            assert_eq!(
                commit(&nested_link).expect("install relative link"),
                CommitOutcome::Applied
            );
            assert_eq!(
                fs::read_link(root.join("nested/link")).expect("read relative link"),
                PathBuf::from(parent_relative_link)
            );
        }

        #[test]
        fn concurrent_creates_serialize_and_only_one_compare_and_swap_applies() {
            let workspace = TempWorkspace::new();
            let root = workspace.path();
            let staging = prepare_staging(root);
            let staged_a = staging.join("stage-a");
            let staged_b = staging.join("stage-b");
            fs::write(&staged_a, b"first").expect("write first stage");
            fs::write(&staged_b, b"second").expect("write second stage");
            let first = request(
                root,
                "new-file",
                None,
                SandboxCommitAction::File {
                    staged_path: staged_a.to_string_lossy().into_owned(),
                    hash: cid(blake3::hash(b"first").as_bytes()),
                    executable: false,
                },
            );
            let second = request(
                root,
                "new-file",
                None,
                SandboxCommitAction::File {
                    staged_path: staged_b.to_string_lossy().into_owned(),
                    hash: cid(blake3::hash(b"second").as_bytes()),
                    executable: false,
                },
            );
            let barrier = Arc::new(Barrier::new(3));
            let first_barrier = Arc::clone(&barrier);
            let first_thread = std::thread::spawn(move || {
                first_barrier.wait();
                commit(&first)
            });
            let second_barrier = Arc::clone(&barrier);
            let second_thread = std::thread::spawn(move || {
                second_barrier.wait();
                commit(&second)
            });
            barrier.wait();
            let first_result = first_thread
                .join()
                .expect("first thread")
                .expect("first CAS");
            let second_result = second_thread
                .join()
                .expect("second thread")
                .expect("second CAS");
            assert_ne!(first_result, second_result);
            assert!(matches!(
                first_result,
                CommitOutcome::Applied | CommitOutcome::Conflict
            ));
            assert!(matches!(
                second_result,
                CommitOutcome::Applied | CommitOutcome::Conflict
            ));
            let content = fs::read(root.join("new-file")).expect("committed file");
            assert!(content == b"first" || content == b"second");
        }

        #[test]
        fn reserved_paths_symlink_parents_and_escaping_links_are_rejected() {
            let workspace = TempWorkspace::new();
            let root = workspace.path();
            assert!(
                commit(&request(
                    root,
                    ".GIT/config",
                    None,
                    SandboxCommitAction::Delete
                ))
                .is_err()
            );
            assert!(
                commit(&request(
                    root,
                    "../escape",
                    None,
                    SandboxCommitAction::Delete
                ))
                .is_err()
            );
            let escaping = request(
                root,
                "link",
                None,
                SandboxCommitAction::Link {
                    target: "../outside".to_owned(),
                    hash: cid(blake3::hash(b"../outside").as_bytes()),
                },
            );
            assert!(commit(&escaping).is_err());

            let outside = TempWorkspace::new();
            std::os::unix::fs::symlink(outside.path(), root.join("jump")).expect("symlink parent");
            let staging = prepare_staging(root);
            let staged = staging.join("stage-c");
            fs::write(&staged, b"payload").expect("write staged bytes");
            let through_link = request(
                root,
                "jump/file",
                None,
                SandboxCommitAction::File {
                    staged_path: staged.to_string_lossy().into_owned(),
                    hash: cid(blake3::hash(b"payload").as_bytes()),
                    executable: false,
                },
            );
            assert!(commit(&through_link).is_err());
            assert!(!outside.path().join("file").exists());
        }

        #[test]
        fn staged_symlink_and_hardlink_are_rejected() {
            let workspace = TempWorkspace::new();
            let root = workspace.path();
            let staging = prepare_staging(root);
            let outside = TempWorkspace::new();
            let outside_file = outside.path().join("outside");
            fs::write(&outside_file, b"payload").expect("write outside");
            let staged_link = staging.join("link-stage");
            std::os::unix::fs::symlink(&outside_file, &staged_link).expect("stage symlink");
            let symlink_request = request(
                root,
                "dest",
                None,
                SandboxCommitAction::File {
                    staged_path: staged_link.to_string_lossy().into_owned(),
                    hash: cid(blake3::hash(b"payload").as_bytes()),
                    executable: false,
                },
            );
            assert!(commit(&symlink_request).is_err());
            assert!(!root.join("dest").exists());

            let staged_hardlink = staging.join("hardlink-stage");
            fs::hard_link(&outside_file, &staged_hardlink).expect("stage hardlink");
            let hardlink_request = request(
                root,
                "dest",
                None,
                SandboxCommitAction::File {
                    staged_path: staged_hardlink.to_string_lossy().into_owned(),
                    hash: cid(blake3::hash(b"payload").as_bytes()),
                    executable: true,
                },
            );
            assert!(commit(&hardlink_request).is_err());
            assert_eq!(
                fs::metadata(&outside_file)
                    .expect("outside metadata")
                    .permissions()
                    .mode()
                    & 0o111,
                0
            );
            assert!(!root.join("dest").exists());
        }
    }
}

fn commit(request: &SandboxCommitRequest) -> Result<CommitOutcome, CommitError> {
    #[cfg(target_os = "linux")]
    {
        linux::commit(request)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = request;
        Err(CommitError::Unsupported)
    }
}
