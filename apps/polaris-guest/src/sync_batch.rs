//! Fixed-protocol, bounded tar batch validation (descriptor: `specs/descriptors/sync_guest_batch.json`).

#[cfg(not(target_os = "linux"))]
use std::io::{self, Write};
use std::process::ExitCode;

pub fn run(root: &str, archive_path: &str) -> ExitCode {
    #[cfg(target_os = "linux")]
    {
        linux::run(root, archive_path)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, archive_path);
        let mut stderr = io::stderr().lock();
        if writeln!(stderr, "sync-batch is unsupported on this platform").is_ok() {
            ExitCode::from(2)
        } else {
            ExitCode::FAILURE
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::io::{self, Read, Write};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Component, Path, PathBuf};

    use polaris_sdk::ids::Cid;
    use polaris_sdk::registry::{SandboxBatchAction, SandboxBatchResult, SandboxSyncBatch};

    use super::ExitCode;

    const MAX_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;
    const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
    const MAX_MUTATIONS: usize = 4096;
    const MAX_MEMBERS: usize = MAX_MUTATIONS + 1;
    const MAX_PATH_BYTES: usize = 4096;

    pub(super) fn run(root: &str, archive_path: &str) -> ExitCode {
        match run_checked(root, archive_path) {
            Ok(result) => match serde_json::to_vec(&result) {
                Ok(bytes) => {
                    let mut stdout = io::stdout().lock();
                    if stdout
                        .write_all(&bytes)
                        .and_then(|()| stdout.write_all(b"\n"))
                        .is_ok()
                    {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::FAILURE
                    }
                }
                Err(_) => fail("cannot encode sync-batch result"),
            },
            Err(_) => fail("sync-batch rejected the request or workspace"),
        }
    }

    fn run_checked(root: &str, archive_path: &str) -> Result<SandboxBatchResult, BatchError> {
        let root = checked_root_under(root, Path::new("/sandbox"))?;
        super::super::sync_cas::prepare_workspace(root.to_str().ok_or(BatchError::Invalid)?)
            .map_err(|_| BatchError::Invalid)?;
        let staging = root.join(".polaris/sync/staging");
        let archive = checked_stage(&staging, archive_path)?;
        let owner = fs::symlink_metadata(&root)?.uid();
        let root_meta = fs::symlink_metadata(&root)?;
        let staging_meta = fs::symlink_metadata(&staging)?;
        let archive_meta = fs::symlink_metadata(&archive)?;
        if !staging_meta.is_dir()
            || staging_meta.file_type().is_symlink()
            || staging_meta.uid() != owner
            || staging_meta.permissions().mode() & 0o077 != 0
            || staging_meta.dev() != root_meta.dev()
            || !archive_meta.is_file()
            || archive_meta.file_type().is_symlink()
            || archive_meta.uid() != owner
            || archive_meta.nlink() != 1
            || archive_meta.permissions().mode() & 0o077 != 0
            || archive_meta.len() > MAX_ARCHIVE_BYTES
            || archive_meta.dev() != root_meta.dev()
        {
            return Err(BatchError::Invalid);
        }
        let (batch, payloads) = decode_archive(&archive)?;
        if batch.root != root.to_string_lossy() {
            return Err(BatchError::Invalid);
        }
        let result = super::super::sync_cas::commit_batch(&root, &batch, &payloads)
            .map_err(|_| BatchError::Invalid)?;
        // A failed archive cleanup means the host cannot know whether the commit completed. The
        // host must discard its snapshot and rescan on every nonzero helper exit.
        if let Err(error) = fs::remove_file(&archive) {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(BatchError::Io(error));
            }
        }
        Ok(result)
    }

    fn checked_root_under(root: &str, trusted_base: &Path) -> Result<PathBuf, BatchError> {
        if root.is_empty() || root.len() > MAX_PATH_BYTES || root.contains('\0') {
            return Err(BatchError::Invalid);
        }
        let input = Path::new(root);
        if !input.is_absolute()
            || input
                .components()
                .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
        {
            return Err(BatchError::Invalid);
        }
        let canonical = fs::canonicalize(input)?;
        let base = fs::canonicalize(trusted_base)?;
        if canonical != input
            || canonical == base
            || !canonical.starts_with(&base)
            || !fs::symlink_metadata(&canonical)?.is_dir()
        {
            return Err(BatchError::Invalid);
        }
        Ok(canonical)
    }

    fn checked_stage(staging: &Path, path: &str) -> Result<PathBuf, BatchError> {
        let candidate = Path::new(path);
        let name = candidate.file_name().and_then(|value| value.to_str());
        if candidate.parent() != Some(staging)
            || name.is_none_or(|value| value.is_empty() || value == "." || value == "..")
        {
            return Err(BatchError::Invalid);
        }
        Ok(candidate.to_path_buf())
    }

    fn decode_archive(
        path: &Path,
    ) -> Result<(SandboxSyncBatch, BTreeMap<String, Vec<u8>>), BatchError> {
        let file = fs::File::open(path)?;
        let mut archive = tar::Archive::new(file);
        let mut entries = archive.entries()?;
        let first = entries.next().ok_or(BatchError::Invalid)??;
        if !first.header().entry_type().is_file()
            || first.path()?.as_ref() != Path::new("manifest.json")
            || first.size() > MAX_MANIFEST_BYTES
        {
            return Err(BatchError::Invalid);
        }
        let mut manifest_bytes = Vec::new();
        first
            .take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut manifest_bytes)?;
        let manifest_size = u64::try_from(manifest_bytes.len()).map_err(|_| BatchError::Invalid)?;
        if manifest_size > MAX_MANIFEST_BYTES {
            return Err(BatchError::Invalid);
        }
        let batch: SandboxSyncBatch =
            serde_json::from_slice(&manifest_bytes).map_err(|_| BatchError::Invalid)?;
        if batch.version != 1 || batch.mutations.is_empty() || batch.mutations.len() > MAX_MUTATIONS
        {
            return Err(BatchError::Invalid);
        }
        let mut payloads = BTreeMap::new();
        let mut count = 1usize;
        let mut total = manifest_size;
        for entry in entries.by_ref() {
            count = count.checked_add(1).ok_or(BatchError::Invalid)?;
            if count > MAX_MEMBERS {
                return Err(BatchError::Invalid);
            }
            let entry = entry?;
            if !entry.header().entry_type().is_file() {
                return Err(BatchError::Invalid);
            }
            let name = entry
                .path()?
                .to_str()
                .ok_or(BatchError::Invalid)?
                .to_owned();
            if !valid_payload_name(&name) {
                return Err(BatchError::Invalid);
            }
            let size = entry.size();
            total = total.checked_add(size).ok_or(BatchError::Invalid)?;
            if total > MAX_ARCHIVE_BYTES || size > MAX_ARCHIVE_BYTES {
                return Err(BatchError::Invalid);
            }
            let capacity = usize::try_from(size).map_err(|_| BatchError::Invalid)?;
            let mut bytes = Vec::with_capacity(capacity);
            entry.take(size.saturating_add(1)).read_to_end(&mut bytes)?;
            if u64::try_from(bytes.len()).map_err(|_| BatchError::Invalid)? != size
                || payloads.insert(name, bytes).is_some()
            {
                return Err(BatchError::Invalid);
            }
        }
        validate_batch(&batch, &payloads)?;
        Ok((batch, payloads))
    }

    fn validate_batch(
        batch: &SandboxSyncBatch,
        payloads: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(), BatchError> {
        let mut paths = BTreeSet::new();
        let mut payload_names = BTreeSet::new();
        for mutation in &batch.mutations {
            let path = checked_relative(&mutation.relative_path)?;
            if !paths.insert(mutation.relative_path.as_str()) {
                return Err(BatchError::Invalid);
            }
            match &mutation.action {
                SandboxBatchAction::Delete => {}
                SandboxBatchAction::File { payload, hash, .. } => {
                    let bytes = payloads.get(payload).ok_or(BatchError::Invalid)?;
                    if !valid_payload_name(payload)
                        || !payload_names.insert(payload.as_str())
                        || Cid::from_bytes(*blake3::hash(bytes).as_bytes()) != *hash
                    {
                        return Err(BatchError::Invalid);
                    }
                }
                SandboxBatchAction::Link { target, hash } => {
                    if Cid::from_bytes(*blake3::hash(target.as_bytes()).as_bytes()) != *hash
                        || !safe_link_target(&path, target)
                    {
                        return Err(BatchError::Invalid);
                    }
                }
            }
        }
        if payload_names.len() != payloads.len()
            || payloads
                .keys()
                .any(|name| !payload_names.contains(name.as_str()))
        {
            return Err(BatchError::Invalid);
        }
        Ok(())
    }

    fn checked_relative(path: &str) -> Result<PathBuf, BatchError> {
        if path.is_empty()
            || path.len() > MAX_PATH_BYTES
            || path.starts_with('/')
            || path.contains(['\\', '\0'])
        {
            return Err(BatchError::Invalid);
        }
        let mut result = PathBuf::new();
        for component in path.split('/') {
            if component.is_empty()
                || component == "."
                || component == ".."
                || component.len() > 255
                || component.eq_ignore_ascii_case(".git")
                || component.eq_ignore_ascii_case(".polaris")
            {
                return Err(BatchError::Invalid);
            }
            result.push(component);
        }
        Ok(result)
    }

    fn valid_payload_name(name: &str) -> bool {
        name.strip_prefix("payload/").is_some_and(|index| {
            index.len() == 6 && index.bytes().all(|byte| byte.is_ascii_digit())
        })
    }

    fn safe_link_target(path: &Path, target: &str) -> bool {
        if target.is_empty()
            || target.len() > MAX_PATH_BYTES
            || target.starts_with('/')
            || target.contains(['\\', '\0'])
        {
            return false;
        }
        let mut depth = path
            .parent()
            .map_or(0usize, |parent| parent.components().count());
        for component in target.split('/') {
            match component {
                "" | "." => {}
                ".." if depth > 0 => depth -= 1,
                ".." => return false,
                _ if component.len() <= 255 => depth += 1,
                _ => return false,
            }
        }
        depth > 0
    }

    fn fail(message: &str) -> ExitCode {
        let mut stderr = io::stderr().lock();
        if writeln!(stderr, "{message}").is_ok() {
            ExitCode::from(2)
        } else {
            ExitCode::FAILURE
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Cursor;
        use std::time::{SystemTime, UNIX_EPOCH};

        use polaris_sdk::registry::{SandboxBatchMutation, SandboxFileState};

        struct TempDir(PathBuf);

        impl TempDir {
            fn new() -> Self {
                let stamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_nanos());
                let path = std::env::temp_dir()
                    .join(format!("polaris-sync-batch-{}-{stamp}", std::process::id()));
                fs::create_dir(&path).expect("create temp directory");
                Self(path)
            }

            fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        fn batch(path: &str, bytes: &[u8]) -> SandboxSyncBatch {
            SandboxSyncBatch {
                version: 1,
                root: "/sandbox/workspace".into(),
                mutations: vec![SandboxBatchMutation {
                    relative_path: path.into(),
                    expected: None::<SandboxFileState>,
                    action: SandboxBatchAction::File {
                        payload: "payload/000000".into(),
                        hash: Cid::from_bytes(*blake3::hash(bytes).as_bytes()),
                        executable: false,
                    },
                }],
            }
        }

        fn append<W: io::Write>(builder: &mut tar::Builder<W>, name: &str, bytes: &[u8]) {
            let mut header = tar::Header::new_gnu();
            header.set_size(u64::try_from(bytes.len()).expect("fixture size"));
            header.set_mode(0o600);
            header.set_entry_type(tar::EntryType::Regular);
            header.set_cksum();
            builder
                .append_data(&mut header, name, Cursor::new(bytes))
                .expect("append fixture");
        }

        fn archive(batch: &SandboxSyncBatch, bytes: &[u8]) -> Vec<u8> {
            let mut builder = tar::Builder::new(Vec::new());
            append(
                &mut builder,
                "manifest.json",
                &serde_json::to_vec(batch).expect("manifest"),
            );
            append(&mut builder, "payload/000000", bytes);
            builder.into_inner().expect("archive")
        }

        #[test]
        fn root_must_be_a_canonical_descendant_of_injected_test_base() {
            let temp = TempDir::new();
            let base = temp.path().join("sandbox");
            let workspace = base.join("workspace");
            fs::create_dir_all(&workspace).expect("workspace");
            assert_eq!(
                checked_root_under(workspace.to_str().expect("utf8"), &base).expect("root"),
                workspace
            );
            assert!(checked_root_under(base.to_str().expect("utf8"), &base).is_err());
            assert!(checked_root_under(temp.path().to_str().expect("utf8"), &base).is_err());
            let link = base.join("escape");
            std::os::unix::fs::symlink(temp.path(), &link).expect("link");
            assert!(checked_root_under(link.to_str().expect("utf8"), &base).is_err());
        }

        #[test]
        fn bounded_tar_decodes_exact_crlf_bytes_and_rejects_denied_paths() {
            let bytes = b"one\r\ntwo\r\n";
            let expected = batch("nested/file.txt", bytes);
            let encoded = archive(&expected, bytes);
            let temp = TempDir::new();
            let path = temp.path().join("batch.tar");
            fs::write(&path, encoded).expect("write fixture archive");
            let (decoded, payloads) = decode_archive(&path).expect("decode");
            assert_eq!(decoded, expected);
            assert_eq!(
                payloads.get("payload/000000").map(Vec::as_slice),
                Some(bytes.as_slice())
            );

            let denied = batch(".polaris/sync/lock", bytes);
            let denied_path = temp.path().join("denied.tar");
            fs::write(&denied_path, archive(&denied, bytes)).expect("write denied archive");
            assert!(decode_archive(&denied_path).is_err());
        }

        #[test]
        fn rejects_mismatched_payload_digest_and_noncanonical_payload_member() {
            let bytes = b"valid";
            let mut invalid = batch("file.txt", b"different");
            let bad_hash_path = TempDir::new();
            let bad_hash = bad_hash_path.path().join("bad-hash.tar");
            fs::write(&bad_hash, archive(&invalid, bytes)).expect("archive");
            assert!(decode_archive(&bad_hash).is_err());

            invalid.mutations[0].relative_path = "a/../outside".into();
            let traversal = bad_hash_path.path().join("traversal.tar");
            fs::write(&traversal, archive(&invalid, b"different")).expect("archive");
            assert!(decode_archive(&traversal).is_err());
        }
    }

    #[derive(Debug, thiserror::Error)]
    enum BatchError {
        #[error("invalid sync-batch request")]
        Invalid,
        #[error(transparent)]
        Io(#[from] io::Error),
    }
}
