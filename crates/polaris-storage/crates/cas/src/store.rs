//! The blob store: `<root>/<ab>/<cd>/<64 lowercase hex>` files in codec format v1.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use polaris_sdk::ids::{Cid, ShortRef};

use crate::atomic::{RetryPolicy, write_atomic};
use crate::codec::{decode_blob, encode_blob};
use crate::content_id;
use crate::error::CasError;

/// Store configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CasConfig {
    /// Store root (keep it short on Windows: blob paths add `ab/cd/` + 64 characters).
    pub root: PathBuf,
    /// zstd level.
    pub zstd_level: i32,
    /// Blobs shorter than this are stored raw.
    pub min_compress_bytes: usize,
    /// Largest blob accepted by `put` and produced by `get`.
    pub max_blob_bytes: u64,
    /// Re-hash on `get` and report [`CasError::Corrupt`] on mismatch.
    pub verify_on_get: bool,
    /// Retry budget for files held by other processes.
    pub retry: RetryPolicy,
}

impl CasConfig {
    /// Defaults: zstd level 3, raw below 64 bytes, 256 MiB cap, verification on.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            zstd_level: 3,
            min_compress_bytes: 64,
            max_blob_bytes: 256 * 1024 * 1024,
            verify_on_get: true,
            retry: RetryPolicy::default(),
        }
    }
}

/// Garbage-collection policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcPolicy {
    /// Blobs modified more recently than this are kept even when not live.
    pub grace: Duration,
    /// Report only; remove nothing.
    pub dry_run: bool,
}

/// What a GC run found (and removed, unless dry-run).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcReport {
    /// Blobs examined.
    pub scanned: u64,
    /// Blobs removed (or that would be removed in a dry run).
    pub removed: u64,
    /// Bytes of removed blobs.
    pub bytes_freed: u64,
    /// Blobs kept because they are live.
    pub kept_live: u64,
    /// Blobs kept because they are younger than the grace period.
    pub kept_young: u64,
}

/// Content-addressable blob store.
#[derive(Debug, Clone)]
pub struct Cas {
    cfg: CasConfig,
}

fn size_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

impl Cas {
    /// Opens (creating if needed) a store.
    ///
    /// # Errors
    /// [`CasError::Io`] when the root cannot be created.
    pub fn open(cfg: CasConfig) -> Result<Self, CasError> {
        fs::create_dir_all(&cfg.root)?;
        Ok(Self { cfg })
    }

    /// Directory holding blobs whose ids start with `first` and `second`.
    fn shard(&self, first: u8, second: u8) -> PathBuf {
        self.cfg
            .root
            .join(format!("{first:02x}"))
            .join(format!("{second:02x}"))
    }

    fn shard_of(&self, bytes: &[u8]) -> PathBuf {
        let mut it = bytes.iter().copied();
        let first = it.next().unwrap_or(0);
        let second = it.next().unwrap_or(0);
        self.shard(first, second)
    }

    /// Where the blob for `cid` is (or would be) stored.
    #[must_use]
    pub fn blob_path(&self, cid: &Cid) -> PathBuf {
        self.shard_of(cid.as_bytes()).join(cid.to_hex())
    }

    /// Stores `bytes`; idempotent (an existing blob is not rewritten).
    ///
    /// # Errors
    /// [`CasError::TooLarge`] over `max_blob_bytes`; [`CasError::Io`] on filesystem failure.
    pub fn put(&self, bytes: &[u8]) -> Result<Cid, CasError> {
        let size = size_u64(bytes.len());
        if size > self.cfg.max_blob_bytes {
            return Err(CasError::TooLarge {
                limit: self.cfg.max_blob_bytes,
                size,
            });
        }
        let cid = content_id(bytes);
        self.store(&cid, bytes)?;
        Ok(cid)
    }

    fn store(&self, cid: &Cid, bytes: &[u8]) -> Result<(), CasError> {
        let path = self.blob_path(cid);
        if path.is_file() {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let blob = encode_blob(bytes, self.cfg.zstd_level, self.cfg.min_compress_bytes)?;
        write_atomic(&path, &blob, &self.cfg.retry)?;
        Ok(())
    }

    /// Stores a file's content. Hashes by streaming first, so an existing blob is never re-read.
    ///
    /// # Errors
    /// [`CasError::TooLarge`] over `max_blob_bytes`; [`CasError::Io`] on filesystem failure or
    /// when the file changes between hashing and reading.
    pub fn put_file(&self, path: &Path) -> Result<Cid, CasError> {
        let size = fs::metadata(path)?.len();
        if size > self.cfg.max_blob_bytes {
            return Err(CasError::TooLarge {
                limit: self.cfg.max_blob_bytes,
                size,
            });
        }
        let read_limit = self.cfg.max_blob_bytes.saturating_add(1);
        let cid = crate::content_id_reader(fs::File::open(path)?, self.cfg.max_blob_bytes)?;
        if self.blob_path(&cid).is_file() {
            return Ok(cid);
        }
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(read_limit)
            .read_to_end(&mut bytes)?;
        let read_bytes = size_u64(bytes.len());
        if read_bytes > self.cfg.max_blob_bytes {
            return Err(CasError::TooLarge {
                limit: self.cfg.max_blob_bytes,
                size: read_bytes,
            });
        }
        if content_id(&bytes) != cid {
            return Err(CasError::Io(io::Error::other(
                "file changed while it was being stored",
            )));
        }
        self.store(&cid, &bytes)?;
        Ok(cid)
    }

    /// Reads a blob.
    ///
    /// # Errors
    /// [`CasError::NotFound`], [`CasError::Corrupt`] (verification), [`CasError::TooLarge`],
    /// [`CasError::Decompress`] / [`CasError::Format`] for damaged blobs, [`CasError::Io`].
    pub fn get(&self, cid: &Cid) -> Result<Bytes, CasError> {
        self.get_bounded(cid, self.cfg.max_blob_bytes)
    }

    /// Reads a blob whose decoded size is bounded by both the store limit and `max_bytes`.
    ///
    /// # Errors
    /// [`CasError::NotFound`], [`CasError::Corrupt`], [`CasError::TooLarge`],
    /// [`CasError::Decompress`] / [`CasError::Format`] for damaged blobs, or [`CasError::Io`].
    pub fn get_bounded(&self, cid: &Cid, max_bytes: u64) -> Result<Bytes, CasError> {
        let max_bytes = self.cfg.max_blob_bytes.min(max_bytes);
        let file = match fs::File::open(self.blob_path(cid)) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(CasError::NotFound(*cid)),
            Err(e) => return Err(e.into()),
        };
        // Every format-v1 blob produced by encode_blob is at most raw bytes + its two-byte
        // header. Bound the encoded read too, including growth after the metadata check.
        let encoded_limit = max_bytes.saturating_add(2);
        let size = file.metadata()?.len();
        if size > encoded_limit {
            return Err(CasError::TooLarge {
                limit: encoded_limit,
                size,
            });
        }
        let mut blob = Vec::new();
        file.take(encoded_limit.saturating_add(1))
            .read_to_end(&mut blob)?;
        let size = size_u64(blob.len());
        if size > encoded_limit {
            return Err(CasError::TooLarge {
                limit: encoded_limit,
                size,
            });
        }
        let bytes = decode_blob(&blob, max_bytes)?;
        if self.cfg.verify_on_get && content_id(&bytes) != *cid {
            return Err(CasError::Corrupt { cid: *cid });
        }
        Ok(Bytes::from(bytes))
    }

    /// `true` when a blob for `cid` exists.
    ///
    /// # Errors
    /// [`CasError::Io`] when the filesystem cannot answer.
    pub fn contains(&self, cid: &Cid) -> Result<bool, CasError> {
        match fs::metadata(self.blob_path(cid)) {
            Ok(meta) => Ok(meta.is_file()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Stored ids whose first 48 bits equal `short`.
    fn matching(&self, short: &ShortRef) -> Result<Vec<Cid>, CasError> {
        let dir = self.shard_of(short.as_bytes());
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut found = Vec::new();
        for entry in entries {
            let entry = entry?;
            if let Ok(cid) = Cid::from_hex(&entry.file_name().to_string_lossy())
                && ShortRef::of(&cid) == *short
            {
                found.push(cid);
            }
        }
        Ok(found)
    }

    /// The full id for a short reference.
    ///
    /// # Errors
    /// [`CasError::UnknownShortRef`] when nothing matches, [`CasError::Ambiguous`] when several
    /// blobs match, [`CasError::Io`].
    pub fn resolve(&self, short: &ShortRef) -> Result<Cid, CasError> {
        let found = self.matching(short)?;
        match found.as_slice() {
            [] => Err(CasError::UnknownShortRef(*short)),
            [only] => Ok(*only),
            _ => Err(CasError::Ambiguous(*short)),
        }
    }

    /// The short reference of a stored blob, refusing it when another blob shares the prefix (the
    /// fold layer then uses the full id).
    ///
    /// # Errors
    /// [`CasError::NotFound`] if `cid` is not stored, [`CasError::ShortRefCollision`] on a shared
    /// prefix, [`CasError::Io`].
    pub fn short_ref_checked(&self, cid: &Cid) -> Result<ShortRef, CasError> {
        let short = ShortRef::of(cid);
        let found = self.matching(&short)?;
        if !found.contains(cid) {
            return Err(CasError::NotFound(*cid));
        }
        if found.len() > 1 {
            return Err(CasError::ShortRefCollision(short));
        }
        Ok(short)
    }

    /// Removes blobs that are neither in `live` nor younger than `policy.grace`.
    ///
    /// # Errors
    /// [`CasError::Io`] when the store cannot be listed or a blob cannot be removed.
    pub fn gc(
        &self,
        live: impl IntoIterator<Item = Cid>,
        policy: GcPolicy,
    ) -> Result<GcReport, CasError> {
        let live: HashSet<Cid> = live.into_iter().collect();
        let now = SystemTime::now();
        let mut report = GcReport::default();
        for first in fs::read_dir(&self.cfg.root)? {
            let first = first?;
            if !first.file_type()?.is_dir() {
                continue;
            }
            for second in fs::read_dir(first.path())? {
                let second = second?;
                if !second.file_type()?.is_dir() {
                    continue;
                }
                for blob in fs::read_dir(second.path())? {
                    let blob = blob?;
                    let Ok(cid) = Cid::from_hex(&blob.file_name().to_string_lossy()) else {
                        continue;
                    };
                    report.scanned = report.scanned.saturating_add(1);
                    if live.contains(&cid) {
                        report.kept_live = report.kept_live.saturating_add(1);
                        continue;
                    }
                    let meta = blob.metadata()?;
                    let age = meta
                        .modified()
                        .ok()
                        .and_then(|m| now.duration_since(m).ok())
                        .unwrap_or(Duration::ZERO);
                    if age < policy.grace {
                        report.kept_young = report.kept_young.saturating_add(1);
                        continue;
                    }
                    report.removed = report.removed.saturating_add(1);
                    report.bytes_freed = report.bytes_freed.saturating_add(meta.len());
                    if !policy.dry_run {
                        fs::remove_file(blob.path())?;
                    }
                }
            }
        }
        Ok(report)
    }
}

impl polaris_sdk::content::ContentStore for Cas {
    fn put(&self, content: &[u8]) -> Result<Cid, polaris_sdk::content::ContentStoreError> {
        Cas::put(self, content).map_err(map_content_error)
    }

    fn get(
        &self,
        cid: &Cid,
        max_bytes: u64,
    ) -> Result<Bytes, polaris_sdk::content::ContentStoreError> {
        self.get_bounded(cid, max_bytes).map_err(map_content_error)
    }

    fn contains(&self, cid: &Cid) -> Result<bool, polaris_sdk::content::ContentStoreError> {
        Cas::contains(self, cid).map_err(map_content_error)
    }
}

fn map_content_error(error: CasError) -> polaris_sdk::content::ContentStoreError {
    use polaris_sdk::content::ContentStoreError;
    match error {
        CasError::Io(_) => ContentStoreError::Unavailable,
        CasError::NotFound(cid) => ContentStoreError::NotFound(cid),
        CasError::Corrupt { cid } => ContentStoreError::Corrupt(cid),
        CasError::TooLarge { limit, size } => ContentStoreError::TooLarge { limit, size },
        CasError::UnknownShortRef(_) | CasError::Ambiguous(_) | CasError::ShortRefCollision(_) => {
            ContentStoreError::InvalidEncoding
        }
        CasError::Decompress(_) | CasError::Format(_) => ContentStoreError::InvalidEncoding,
    }
}
