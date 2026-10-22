//! Bounded immutable remote manifest snapshots for paged host sync.

use std::io::{self, Write};
use std::process::ExitCode;

use polaris_sdk::registry::{SandboxSnapshotEntry, SandboxSnapshotPage};

const MAX_REQUEST_LIMIT: u32 = 4096;

/// Create a snapshot on the first call, or return a bound page from an existing generation.
pub fn run(root: &str, generation: &str, offset: &str, limit: &str) -> ExitCode {
    let offset = match offset.parse::<u64>() {
        Ok(value) => value,
        Err(_) => return fail("invalid sync-snapshot offset"),
    };
    let limit = match limit.parse::<u32>() {
        Ok(value) if (1..=MAX_REQUEST_LIMIT).contains(&value) => value,
        _ => return fail("invalid sync-snapshot page size"),
    };
    let generation = if generation == "-" {
        None
    } else {
        Some(generation)
    };
    match snapshot_page(root, generation, offset, limit) {
        Ok(page) => match serde_json::to_vec(&page) {
            Ok(bytes) => {
                let mut stdout = io::stdout().lock();
                match stdout
                    .write_all(&bytes)
                    .and_then(|()| stdout.write_all(b"\n"))
                {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(_) => ExitCode::FAILURE,
                }
            }
            Err(_) => fail("sync-snapshot response encoding failed"),
        },
        Err(_) => fail("sync-snapshot rejected the request or workspace"),
    }
}

#[derive(Debug, thiserror::Error)]
enum SnapshotError {
    #[cfg(not(target_os = "linux"))]
    #[error("unsupported platform")]
    Unsupported,
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Io(#[from] io::Error),
    #[cfg(target_os = "linux")]
    #[error("invalid snapshot request or workspace")]
    Invalid,
    #[cfg(target_os = "linux")]
    #[error("snapshot limit exceeded")]
    Limit,
    #[cfg(target_os = "linux")]
    #[error("invalid cached snapshot")]
    Cache,
}

#[cfg(target_os = "linux")]
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotCache {
    root: String,
    created_unix_seconds: u64,
    generation: String,
    manifest_id: polaris_sdk::ids::Cid,
    entries: Vec<SandboxSnapshotEntry>,
}

#[cfg(target_os = "linux")]
fn snapshot_page(
    root: &str,
    generation: Option<&str>,
    offset: u64,
    limit: u32,
) -> Result<SandboxSnapshotPage, SnapshotError> {
    linux::snapshot_page(root, generation, offset, limit)
}

#[cfg(not(target_os = "linux"))]
fn snapshot_page(
    _root: &str,
    _generation: Option<&str>,
    _offset: u64,
    _limit: u32,
) -> Result<SandboxSnapshotPage, SnapshotError> {
    Err(SnapshotError::Unsupported)
}

fn fail(message: &str) -> ExitCode {
    let mut stderr = io::stderr().lock();
    match writeln!(stderr, "{message}") {
        Ok(()) => ExitCode::from(2),
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::BTreeMap;
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::{Component, Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use fs4::FileExt;
    use polaris_sdk::ids::Cid;
    use polaris_sdk::registry::{SandboxSnapshotEntry, SandboxSnapshotPage};

    use super::{SnapshotCache, SnapshotError};

    const O_NOFOLLOW: i32 = 0o400000;
    const MAX_PATH_BYTES: usize = 4096;
    const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
    const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;
    const MAX_WALK_ENTRIES: u64 = 1_000_000;
    const MAX_MANIFEST_ENTRIES: usize = 100_000;
    const MAX_CACHE_BYTES: u64 = 64 * 1024 * 1024;
    const MAX_PAGE_BYTES: usize = 4 * 1024 * 1024;
    const SNAPSHOT_TTL_SECONDS: u64 = 600;

    pub(super) fn snapshot_page(
        root: &str,
        generation: Option<&str>,
        offset: u64,
        limit: u32,
    ) -> Result<SandboxSnapshotPage, SnapshotError> {
        let root = checked_root(root)?;
        let owner = fs::symlink_metadata(&root)?.uid();
        let staging = root.join(".polaris/sync/staging");
        super::super::sync_cas::prepare_workspace(root.to_str().ok_or(SnapshotError::Invalid)?)
            .map_err(|_| SnapshotError::Invalid)?;
        ensure_private_dir(&staging, owner)?;
        let snapshots = staging.join("snapshots");
        ensure_private_dir(&snapshots, owner)?;

        let cache = match generation {
            None => {
                if offset != 0 {
                    return Err(SnapshotError::Invalid);
                }
                create_snapshot(&root, &snapshots, owner)?
            }
            Some(generation) => read_snapshot(&root, &snapshots, generation)?,
        };
        build_page(cache, offset, limit)
    }

    fn checked_root(root: &str) -> Result<PathBuf, SnapshotError> {
        if root.is_empty()
            || root.len() > MAX_PATH_BYTES
            || !root.starts_with('/')
            || root.contains('\0')
        {
            return Err(SnapshotError::Invalid);
        }
        let path = PathBuf::from(root);
        if path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        {
            return Err(SnapshotError::Invalid);
        }
        let canonical = fs::canonicalize(&path)?;
        if canonical != path || !fs::symlink_metadata(&canonical)?.is_dir() {
            return Err(SnapshotError::Invalid);
        }
        Ok(canonical)
    }

    fn create_snapshot(
        root: &Path,
        snapshots: &Path,
        owner: u32,
    ) -> Result<SnapshotCache, SnapshotError> {
        let sync = root.join(".polaris/sync");
        let lock_path = sync.join("lock");
        let lock = open_private_lock(&lock_path, owner)?;
        FileExt::lock(&lock)?;
        prune_snapshot_cache(snapshots)?;
        let result = scan(root, owner);
        FileExt::unlock(&lock)?;
        let entries = result?;
        let manifest_id = content_id(&entries);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SnapshotError::Invalid)?
            .as_secs();
        let seed = [root.as_os_str().as_encoded_bytes(), manifest_id.as_bytes()].concat();
        let generation = blake3::hash(&seed).to_hex().to_string();
        let cache = SnapshotCache {
            root: root.to_string_lossy().into_owned(),
            created_unix_seconds: now,
            generation,
            manifest_id,
            entries,
        };
        let encoded = serde_json::to_vec(&cache).map_err(|_| SnapshotError::Cache)?;
        if u64::try_from(encoded.len()).map_or(true, |size| size > MAX_CACHE_BYTES) {
            return Err(SnapshotError::Limit);
        }
        let path = snapshots.join(format!("{}.json", cache.generation));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(mut file) => {
                file.write_all(&encoded)?;
                file.sync_all()?;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let existing = read_snapshot(root, snapshots, &cache.generation)?;
                if existing.manifest_id != cache.manifest_id {
                    return Err(SnapshotError::Cache);
                }
                return Ok(existing);
            }
            Err(error) => return Err(SnapshotError::Io(error)),
        }
        Ok(cache)
    }

    fn read_snapshot(
        root: &Path,
        snapshots: &Path,
        generation: &str,
    ) -> Result<SnapshotCache, SnapshotError> {
        if generation.len() != 64 || !generation.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(SnapshotError::Invalid);
        }
        let path = snapshots.join(format!("{generation}.json"));
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_CACHE_BYTES
            || metadata.uid() != fs::symlink_metadata(root)?.uid()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(SnapshotError::Cache);
        }
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_CACHE_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if u64::try_from(bytes.len()).map_or(true, |size| size > MAX_CACHE_BYTES) {
            return Err(SnapshotError::Limit);
        }
        let cache: SnapshotCache =
            serde_json::from_slice(&bytes).map_err(|_| SnapshotError::Cache)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SnapshotError::Invalid)?
            .as_secs();
        if cache.root != root.to_string_lossy()
            || cache.generation != generation
            || now.saturating_sub(cache.created_unix_seconds) > SNAPSHOT_TTL_SECONDS
            || content_id(&cache.entries) != cache.manifest_id
        {
            return Err(SnapshotError::Cache);
        }
        Ok(cache)
    }

    fn prune_snapshot_cache(snapshots: &Path) -> Result<(), SnapshotError> {
        let now = SystemTime::now();
        let mut retained = 0_usize;
        let mut visited = 0_usize;
        for entry in fs::read_dir(snapshots)? {
            visited = visited.checked_add(1).ok_or(SnapshotError::Limit)?;
            if visited > 64 {
                return Err(SnapshotError::Limit);
            }
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(SnapshotError::Cache);
            }
            let age = metadata
                .modified()
                .ok()
                .and_then(|time| now.duration_since(time).ok());
            if age.is_some_and(|duration| duration.as_secs() > SNAPSHOT_TTL_SECONDS) {
                fs::remove_file(entry.path())?;
            } else {
                retained = retained.checked_add(1).ok_or(SnapshotError::Limit)?;
                if retained >= 16 {
                    return Err(SnapshotError::Limit);
                }
            }
        }
        Ok(())
    }

    fn scan(root: &Path, owner: u32) -> Result<Vec<SandboxSnapshotEntry>, SnapshotError> {
        let mut stack = vec![(root.to_path_buf(), String::new())];
        let mut entries = BTreeMap::<String, SandboxSnapshotEntry>::new();
        let mut walked = 0_u64;
        let mut total = 0_u64;
        let mut serialized_estimate = 0_usize;
        while let Some((directory, prefix)) = stack.pop() {
            for child in fs::read_dir(&directory)? {
                walked = walked.checked_add(1).ok_or(SnapshotError::Limit)?;
                if walked > MAX_WALK_ENTRIES {
                    return Err(SnapshotError::Limit);
                }
                let child = child?;
                let name = child
                    .file_name()
                    .into_string()
                    .map_err(|_| SnapshotError::Invalid)?;
                if name.is_empty() || name.len() > 255 || name.contains(['/', '\\', '\0']) {
                    return Err(SnapshotError::Invalid);
                }
                let relative = if prefix.is_empty() {
                    name
                } else {
                    format!("{prefix}/{name}")
                };
                if relative.len() > MAX_PATH_BYTES {
                    return Err(SnapshotError::Limit);
                }
                if denied(&relative) {
                    continue;
                }
                let metadata = fs::symlink_metadata(child.path())?;
                if metadata.uid() != owner {
                    return Err(SnapshotError::Invalid);
                }
                if metadata.file_type().is_dir() {
                    stack.push((child.path(), relative));
                    continue;
                }
                let entry = if metadata.file_type().is_symlink() {
                    let target = fs::read_link(child.path())?
                        .into_os_string()
                        .into_string()
                        .map_err(|_| SnapshotError::Invalid)?;
                    if !safe_link_target(&relative, &target) {
                        continue;
                    }
                    let size = u64::try_from(target.len()).map_err(|_| SnapshotError::Limit)?;
                    total = total.checked_add(size).ok_or(SnapshotError::Limit)?;
                    SandboxSnapshotEntry {
                        relative_path: relative.clone(),
                        size,
                        hash: cid(blake3::hash(target.as_bytes()).as_bytes()),
                        executable: false,
                        link_target: Some(target),
                    }
                } else if metadata.file_type().is_file() {
                    if metadata.len() > MAX_FILE_BYTES {
                        return Err(SnapshotError::Limit);
                    }
                    total = total
                        .checked_add(metadata.len())
                        .ok_or(SnapshotError::Limit)?;
                    let (hash, size) = hash_file(&child.path())?;
                    let after = fs::symlink_metadata(child.path())?;
                    if !after.file_type().is_file()
                        || after.len() != size
                        || after.mtime() != metadata.mtime()
                        || after.mtime_nsec() != metadata.mtime_nsec()
                        || after.mode() & 0o111 != metadata.mode() & 0o111
                    {
                        return Err(SnapshotError::Invalid);
                    }
                    SandboxSnapshotEntry {
                        relative_path: relative.clone(),
                        size,
                        hash,
                        executable: metadata.mode() & 0o111 != 0,
                        link_target: None,
                    }
                } else {
                    continue;
                };
                if total > MAX_TOTAL_BYTES || entries.len() >= MAX_MANIFEST_ENTRIES {
                    return Err(SnapshotError::Limit);
                }
                serialized_estimate = serialized_estimate
                    .checked_add(relative.len().saturating_add(256))
                    .ok_or(SnapshotError::Limit)?;
                if serialized_estimate > usize::try_from(MAX_CACHE_BYTES).unwrap_or(usize::MAX) {
                    return Err(SnapshotError::Limit);
                }
                entries.insert(relative, entry);
            }
        }
        Ok(entries.into_values().collect())
    }

    fn hash_file(path: &Path) -> Result<(Cid, u64), SnapshotError> {
        let mut options = OpenOptions::new();
        options.read(true).custom_flags(O_NOFOLLOW);
        let mut file = options.open(path)?;
        let mut hasher = blake3::Hasher::new();
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            size = size
                .checked_add(u64::try_from(count).map_err(|_| SnapshotError::Limit)?)
                .ok_or(SnapshotError::Limit)?;
            if size > MAX_FILE_BYTES {
                return Err(SnapshotError::Limit);
            }
            hasher.update(&buffer[..count]);
        }
        Ok((cid(hasher.finalize().as_bytes()), size))
    }

    fn content_id(entries: &[SandboxSnapshotEntry]) -> Cid {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"polaris-sync-manifest-v1\0");
        for entry in entries {
            let path = entry.relative_path.as_bytes();
            hasher.update(&(path.len() as u64).to_be_bytes());
            hasher.update(path);
            hasher.update(&entry.size.to_be_bytes());
            hasher.update(entry.hash.as_bytes());
            hasher.update(&[u8::from(entry.executable)]);
            match &entry.link_target {
                Some(target) => {
                    hasher.update(&[1]);
                    hasher.update(&(target.len() as u64).to_be_bytes());
                    hasher.update(target.as_bytes());
                }
                None => {
                    hasher.update(&[0]);
                }
            }
        }
        cid(hasher.finalize().as_bytes())
    }

    fn build_page(
        cache: SnapshotCache,
        offset: u64,
        limit: u32,
    ) -> Result<SandboxSnapshotPage, SnapshotError> {
        let start = usize::try_from(offset).map_err(|_| SnapshotError::Limit)?;
        if start > cache.entries.len() {
            return Err(SnapshotError::Invalid);
        }
        let hard_end = start
            .checked_add(usize::try_from(limit).map_err(|_| SnapshotError::Limit)?)
            .ok_or(SnapshotError::Limit)?
            .min(cache.entries.len());
        let mut entries = Vec::new();
        let mut bytes = 512_usize;
        for entry in cache.entries.iter().skip(start).take(hard_end - start) {
            let encoded = serde_json::to_vec(entry).map_err(|_| SnapshotError::Cache)?;
            let next_bytes = bytes
                .checked_add(encoded.len())
                .and_then(|value| value.checked_add(1))
                .ok_or(SnapshotError::Limit)?;
            if next_bytes > MAX_PAGE_BYTES {
                if entries.is_empty() {
                    return Err(SnapshotError::Limit);
                }
                break;
            }
            bytes = next_bytes;
            entries.push(entry.clone());
        }
        let next = start
            .checked_add(entries.len())
            .ok_or(SnapshotError::Limit)?;
        let next_offset = if next < cache.entries.len() {
            Some(u64::try_from(next).map_err(|_| SnapshotError::Limit)?)
        } else {
            None
        };
        Ok(SandboxSnapshotPage {
            generation: cache.generation,
            manifest_id: cache.manifest_id,
            total_entries: u64::try_from(cache.entries.len()).map_err(|_| SnapshotError::Limit)?,
            offset,
            entries,
            next_offset,
        })
    }

    fn ensure_private_dir(path: &Path, owner: u32) -> Result<(), SnapshotError> {
        match fs::create_dir(path) {
            Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(0o700))?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(SnapshotError::Io(error)),
        }
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_dir()
            || metadata.permissions().mode() & 0o077 != 0
            || metadata.uid() != owner
        {
            return Err(SnapshotError::Invalid);
        }
        Ok(())
    }

    fn open_private_lock(path: &Path, owner: u32) -> Result<File, SnapshotError> {
        match fs::symlink_metadata(path) {
            Ok(metadata)
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.uid() != owner
                    || metadata.permissions().mode() & 0o077 != 0 =>
            {
                return Err(SnapshotError::Invalid);
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(SnapshotError::Io(error)),
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
                    || metadata.uid() != owner
                    || metadata.permissions().mode() & 0o077 != 0
                {
                    return Err(SnapshotError::Invalid);
                }
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .custom_flags(O_NOFOLLOW)
                    .open(path)?
            }
            Err(error) => return Err(SnapshotError::Io(error)),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != owner
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(SnapshotError::Invalid);
        }
        Ok(file)
    }

    fn denied(path: &str) -> bool {
        path.split('/')
            .any(|part| part.eq_ignore_ascii_case(".git") || part.eq_ignore_ascii_case(".polaris"))
            || path.split('/').any(|part| {
                let lower = part.to_ascii_lowercase();
                lower == ".env"
                    || lower.starts_with(".env.")
                    || lower.ends_with(".pem")
                    || lower.ends_with(".key")
                    || lower.ends_with(".pfx")
                    || lower.ends_with(".p12")
                    || lower == ".netrc"
                    || lower == ".npmrc"
                    || lower == ".pypirc"
            })
    }

    fn safe_link_target(path: &str, target: &str) -> bool {
        if target.is_empty()
            || target.len() > MAX_PATH_BYTES
            || target.starts_with('/')
            || target.contains(['\\', '\0'])
        {
            return false;
        }
        let mut depth = path.split('/').count().saturating_sub(1);
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

    fn cid(bytes: &[u8; 32]) -> Cid {
        Cid::from_bytes(*bytes)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::time::{SystemTime, UNIX_EPOCH};

        struct Workspace(PathBuf);

        impl Workspace {
            fn new() -> Self {
                let stamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_nanos());
                let path = std::env::temp_dir().join(format!(
                    "polaris-sync-snapshot-{}-{stamp}",
                    std::process::id()
                ));
                fs::create_dir(&path).expect("create snapshot workspace");
                Self(path)
            }

            fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Workspace {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        #[test]
        fn paged_snapshot_is_bound_to_one_immutable_manifest_generation() {
            let workspace = Workspace::new();
            fs::write(workspace.path().join("a"), b"first").expect("write a");
            fs::write(workspace.path().join("b"), b"old second").expect("write b");
            fs::write(workspace.path().join("c"), b"third").expect("write c");
            let root = workspace.path().to_string_lossy().into_owned();

            let first = snapshot_page(&root, None, 0, 1).expect("create snapshot page");
            assert_eq!(first.total_entries, 3);
            assert_eq!(first.offset, 0);
            assert_eq!(first.entries[0].relative_path, "a");
            assert_eq!(first.next_offset, Some(1));
            fs::write(workspace.path().join("b"), b"new second").expect("change b");

            let second = snapshot_page(&root, Some(&first.generation), 1, 1)
                .expect("read original snapshot page");
            assert_eq!(second.generation, first.generation);
            assert_eq!(second.manifest_id, first.manifest_id);
            assert_eq!(second.entries[0].relative_path, "b");
            assert_eq!(
                second.entries[0].hash,
                cid(blake3::hash(b"old second").as_bytes())
            );
            assert_eq!(second.next_offset, Some(2));
        }

        #[test]
        fn snapshot_denies_reserved_trees_and_never_follows_escaping_links() {
            let workspace = Workspace::new();
            fs::write(workspace.path().join("ok"), b"visible").expect("write visible file");
            fs::write(workspace.path().join(".env"), b"secret").expect("write denied file");
            fs::create_dir(workspace.path().join(".git")).expect("create git metadata");
            fs::write(workspace.path().join(".git/HEAD"), b"private").expect("write metadata");
            let outside = Workspace::new();
            fs::write(outside.path().join("outside"), b"not visible").expect("write outside");
            std::os::unix::fs::symlink(
                outside.path().join("outside"),
                workspace.path().join("escape"),
            )
            .expect("create escaping symlink");
            let root = workspace.path().to_string_lossy().into_owned();
            let page = snapshot_page(&root, None, 0, 32).expect("snapshot");
            let paths: Vec<&str> = page
                .entries
                .iter()
                .map(|entry| entry.relative_path.as_str())
                .collect();
            assert_eq!(paths, ["ok"]);
        }

        #[test]
        fn snapshot_rejects_expired_or_malformed_generation_tokens() {
            let workspace = Workspace::new();
            let root = workspace.path().to_string_lossy().into_owned();
            assert!(snapshot_page(&root, Some("../escape"), 0, 1).is_err());
            assert!(snapshot_page(&root, None, 1, 1).is_err());
        }
    }
}
