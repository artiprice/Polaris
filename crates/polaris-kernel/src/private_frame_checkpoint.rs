//! Host-only encrypted associations between durable transcripts and reversible PII maps.
//!
//! The host provisions the private directory and sealing key. The directory must remain outside
//! the workspace and protected against other users and hostile replacement. This module does not
//! configure Windows ACLs, provision keys, or provide an external monotonic rollback anchor.
use crate::{KernelHandle, privacy_snapshot::KernelPiiSealer, storage::cas::content_id};
use polaris_sdk::{
    contracts::Event,
    ids::{Cid, CommandId, FrameId, Seq, SessionId},
};
use polaris_security::{
    automata::EntropyCfg,
    pii::{PiiLimits, PiiVault, SealedBlob, Sealer},
};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use zeroize::Zeroizing;

const MAX_ASSOCIATION_BYTES: u64 = 4096;
const MAX_SNAPSHOT_BYTES: u64 = 1_052_672;

/// Explicit private recovery owner for one actual kernel frame.
pub struct PrivateFrameCheckpointStore {
    kernel: KernelHandle,
    session: SessionId,
    frame: FrameId,
    directory: PathBuf,
    sealer: Arc<KernelPiiSealer>,
}
impl fmt::Debug for PrivateFrameCheckpointStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateFrameCheckpointStore(<host-owned encrypted recovery>)")
    }
}

/// Sanitized failures; no source text, marker map, key or private path is exposed.
#[derive(Debug, thiserror::Error)]
pub enum PrivateCheckpointError {
    /// The actual kernel/frame/vault identity differs from the requested owner.
    #[error("private checkpoint owner mismatch")]
    Owner,
    /// The host directory or supplied single-component file name is invalid.
    #[error("private checkpoint path rejected")]
    Path,
    /// Admission, durable storage, bounded readback or a blocking owner failed.
    #[error("private checkpoint storage failed")]
    Storage,
    /// Ciphertext authentication, snapshot bounds or association validation failed.
    #[error("private checkpoint authentication failed")]
    Authentication,
}

impl polaris_sdk::error::HasKind for PrivateCheckpointError {
    fn kind(&self) -> polaris_sdk::error::ErrorKind {
        match self {
            Self::Owner | Self::Path => polaris_sdk::error::ErrorKind::Policy,
            Self::Storage => polaris_sdk::error::ErrorKind::Transient,
            Self::Authentication => polaris_sdk::error::ErrorKind::Fatal,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Association {
    version: u16,
    session: SessionId,
    frame: FrameId,
    step: u32,
    transcript: Cid,
    snapshot: Cid,
}

impl PrivateFrameCheckpointStore {
    /// Admit an existing host-private directory outside the actual workspace.
    /// The host must retain the same sealing key for recovery and enforce directory access.
    /// # Errors
    /// Rejects foreign frames, relative/workspace directories and closed kernel admission.
    pub async fn open(
        kernel: KernelHandle,
        frame: FrameId,
        directory: impl AsRef<Path>,
        sealer: Arc<KernelPiiSealer>,
    ) -> Result<Self, PrivateCheckpointError> {
        if frame != kernel.frame() && kernel.frame_cancellations().is_cancelled(frame).is_err() {
            return Err(PrivateCheckpointError::Owner);
        }
        let directory = directory.as_ref().to_owned();
        if !directory.is_absolute() {
            return Err(PrivateCheckpointError::Path);
        }
        let owner = kernel
            .inspection_owner()
            .await
            .map_err(|_| PrivateCheckpointError::Storage)?;
        let directory = tokio::task::spawn_blocking(move || {
            owner
                .ensure_open()
                .map_err(|_| PrivateCheckpointError::Storage)?;
            let metadata =
                fs::symlink_metadata(&directory).map_err(|_| PrivateCheckpointError::Path)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(PrivateCheckpointError::Path);
            }
            let directory = directory
                .canonicalize()
                .map_err(|_| PrivateCheckpointError::Path)?;
            let workspace = owner
                .workspace_root()
                .canonicalize()
                .map_err(|_| PrivateCheckpointError::Path)?;
            if directory.starts_with(&workspace) || workspace.starts_with(&directory) {
                return Err(PrivateCheckpointError::Path);
            }
            Ok(directory)
        })
        .await
        .map_err(|_| PrivateCheckpointError::Storage)??;
        Ok(Self {
            session: kernel.session(),
            kernel,
            frame,
            directory,
            sealer,
        })
    }

    pub(crate) fn matches(&self, kernel: &KernelHandle, frame: FrameId, vault: &PiiVault) -> bool {
        self.session == kernel.session()
            && self.frame == frame
            && vault.belongs_to(self.session, self.frame)
    }

    // Called within the journal's already-admitted blocking owner, before event publication.
    // Returning success means the encrypted association and exact map readback both succeeded.
    pub(crate) fn persist(
        &self,
        cas: &crate::storage::cas::Cas,
        vault: &PiiVault,
        step: u32,
        transcript: Cid,
    ) -> Result<(), PrivateCheckpointError> {
        if step == 0 || !vault.belongs_to(self.session, self.frame) {
            return Err(PrivateCheckpointError::Owner);
        }
        let blob = vault
            .export_sealed(self.sealer.as_ref())
            .map_err(|_| PrivateCheckpointError::Authentication)?;
        let snapshot = cas
            .put(blob.ciphertext())
            .map_err(|_| PrivateCheckpointError::Storage)?;
        let read = cas
            .get_bounded(&snapshot, MAX_SNAPSHOT_BYTES)
            .map_err(|_| PrivateCheckpointError::Storage)?;
        if snapshot != content_id(blob.ciphertext()) || read.as_ref() != blob.ciphertext() {
            return Err(PrivateCheckpointError::Storage);
        }
        let association = Association {
            version: 1,
            session: self.session,
            frame: self.frame,
            step,
            transcript,
            snapshot,
        };
        let bytes = Zeroizing::new(
            serde_json::to_vec(&association).map_err(|_| PrivateCheckpointError::Storage)?,
        );
        let sealed = self
            .sealer
            .seal(&bytes)
            .map_err(|_| PrivateCheckpointError::Authentication)?;
        if u64::try_from(sealed.ciphertext().len()).map_err(|_| PrivateCheckpointError::Storage)?
            > MAX_ASSOCIATION_BYTES
        {
            return Err(PrivateCheckpointError::Storage);
        }
        let name = format!(
            "{}-{}-{step}-{}.sealed",
            self.session,
            self.frame,
            CommandId::new_v7()
        );
        let target = self.directory.join(name);
        let temporary = self
            .directory
            .join(format!("{}.pending", CommandId::new_v7()));
        let result = write_sealed(&temporary, &target, sealed.ciphertext());
        if result.is_err() {
            // Cleanup failures remain storage failures. A .pending orphan is never a recovery file.
            match fs::remove_file(&temporary) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(PrivateCheckpointError::Storage),
            }
        }
        result
    }

    /// Recover a map from an explicit host-selected file and actual journal identity.
    /// No "latest" inference is made from directory order or timestamps. The trusted caller
    /// must select an existing durable transcript event and supply its exact step and CID.
    /// # Errors
    /// Rejects traversal, oversized/corrupt ciphertext, foreign identity, mismatched transcript,
    /// unpublished/missing/corrupt transcript or snapshot data and incompatible map limits. An old valid checkpoint
    /// still requires host rollback policy; authenticated encryption alone cannot detect replay.
    pub async fn restore(
        &self,
        file_name: &str,
        step: u32,
        transcript: Cid,
        entropy: EntropyCfg,
        limits: PiiLimits,
    ) -> Result<PiiVault, PrivateCheckpointError> {
        if step == 0 || !valid_file_name(file_name) {
            return Err(PrivateCheckpointError::Path);
        }
        let owner = self
            .kernel
            .inspection_owner()
            .await
            .map_err(|_| PrivateCheckpointError::Storage)?;
        let path = self.directory.join(file_name);
        let sealer = Arc::clone(&self.sealer);
        let session = self.session;
        let frame = self.frame;
        tokio::task::spawn_blocking(move || {
            owner.ensure_open().map_err(|_| PrivateCheckpointError::Storage)?;
            let metadata = fs::symlink_metadata(&path).map_err(|_| PrivateCheckpointError::Storage)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_ASSOCIATION_BYTES {
                return Err(PrivateCheckpointError::Path);
            }
            let mut bytes = Vec::new();
            File::open(path).map_err(|_| PrivateCheckpointError::Storage)?
                .take(MAX_ASSOCIATION_BYTES.saturating_add(1)).read_to_end(&mut bytes)
                .map_err(|_| PrivateCheckpointError::Storage)?;
            if u64::try_from(bytes.len()).map_err(|_| PrivateCheckpointError::Storage)? > MAX_ASSOCIATION_BYTES {
                return Err(PrivateCheckpointError::Path);
            }
            let plaintext = sealer.open(&SealedBlob::from_ciphertext(bytes)).map_err(|_| PrivateCheckpointError::Authentication)?;
            let association: Association = serde_json::from_slice(&plaintext).map_err(|_| PrivateCheckpointError::Authentication)?;
            if association.version != 1 || association.session != session || association.frame != frame
                || association.step != step || association.transcript != transcript {
                return Err(PrivateCheckpointError::Authentication);
            }
            let mut published = false;
            let records = owner.context().bus.store().replay_iter(Seq::new(u64::MAX))
                .map_err(|_| PrivateCheckpointError::Storage)?;
            for (index, record) in records.enumerate() {
                if index >= 1_000_000 { return Err(PrivateCheckpointError::Storage); }
                let record = record.map_err(|_| PrivateCheckpointError::Storage)?;
                if record.session() == session && matches!(record.event(),
                    Event::FrameStepRecorded { frame: recorded_frame, step: recorded_step, transcript: recorded_transcript, .. }
                    if *recorded_frame == frame && *recorded_step == step && *recorded_transcript == transcript) {
                    published = true;
                    break;
                }
            }
            if !published {
                return Err(PrivateCheckpointError::Owner);
            }
            let transcript_bytes = owner.cas().get_bounded(&transcript, 67_108_864)
                .map_err(|_| PrivateCheckpointError::Storage)?;
            if content_id(&transcript_bytes) != transcript {
                return Err(PrivateCheckpointError::Authentication);
            }
            let snapshot = owner.cas().get_bounded(&association.snapshot, MAX_SNAPSHOT_BYTES).map_err(|_| PrivateCheckpointError::Storage)?;
            if content_id(&snapshot) != association.snapshot {
                return Err(PrivateCheckpointError::Authentication);
            }
            PiiVault::import_sealed(session, frame, entropy, limits, sealer.as_ref(), &SealedBlob::from_ciphertext(snapshot.to_vec()))
                .map_err(|_| PrivateCheckpointError::Authentication)
        }).await.map_err(|_| PrivateCheckpointError::Storage)?
    }
}

fn valid_file_name(value: &str) -> bool {
    let mut components = Path::new(value).components();
    !value.is_empty()
        && value.len() <= 200
        && value.ends_with(".sealed")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'))
        && matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none()
}

fn write_sealed(
    temporary: &Path,
    target: &Path,
    bytes: &[u8],
) -> Result<(), PrivateCheckpointError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(temporary)
        .map_err(|_| PrivateCheckpointError::Storage)?;
    file.write_all(bytes)
        .map_err(|_| PrivateCheckpointError::Storage)?;
    file.sync_all()
        .map_err(|_| PrivateCheckpointError::Storage)?;
    drop(file);
    fs::rename(temporary, target).map_err(|_| PrivateCheckpointError::Storage)?;
    #[cfg(unix)]
    File::open(target.parent().ok_or(PrivateCheckpointError::Path)?)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| PrivateCheckpointError::Storage)?;
    let mut read = Vec::new();
    File::open(target)
        .map_err(|_| PrivateCheckpointError::Storage)?
        .take(MAX_ASSOCIATION_BYTES.saturating_add(1))
        .read_to_end(&mut read)
        .map_err(|_| PrivateCheckpointError::Storage)?;
    if read != bytes {
        return Err(PrivateCheckpointError::Storage);
    }
    Ok(())
}
