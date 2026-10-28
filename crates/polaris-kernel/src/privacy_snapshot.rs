//! Actual AEAD bridge for bounded PII snapshots; key custody stays with the explicit secret vault.
use crate::KernelHandle;
use polaris_sdk::ids::{Cid, FrameId, SessionId};
use polaris_security::{
    automata::EntropyCfg,
    pii::{PiiLimits, PiiVault},
};
use polaris_security::{
    pii::{PiiError, SealedBlob, Sealer},
    secrets::{SealedBlob as SecretBlob, SecretVault},
};
use std::{fmt, sync::Arc};
use zeroize::Zeroizing;

/// Host-only private CAS storage for actual encrypted PII marker snapshots.
///
/// Returned CIDs are host resume metadata. Saving does not publish events, expose a model
/// reference, write plaintext, or grant an agent CAS Read authority.
#[derive(Debug)]
pub struct KernelPiiSnapshotStore {
    kernel: KernelHandle,
    session: SessionId,
    frame: FrameId,
    sealer: Arc<KernelPiiSealer>,
}
/// Sanitized failure storing or reconstructing private encrypted snapshots.
#[derive(Debug, thiserror::Error)]
pub enum PiiSnapshotStoreError {
    /// Vault/frame does not belong to this actual kernel session tree.
    #[error("private PII snapshot owner mismatch")]
    Owner,
    /// Storage, digest, admission or owned blocking task failed.
    #[error("private PII snapshot storage failed")]
    Storage,
    /// Snapshot capacity, format or authentication failed.
    #[error(transparent)]
    Snapshot(#[from] PiiError),
}
impl KernelPiiSnapshotStore {
    /// Bind actual session identity and a root or registered descendant frame to an explicit key owner.
    /// # Errors
    /// Rejects foreign/unregistered frame identities. This constructor is trusted host composition.
    pub fn new(
        kernel: KernelHandle,
        frame: FrameId,
        sealer: Arc<KernelPiiSealer>,
    ) -> Result<Self, PiiSnapshotStoreError> {
        if frame != kernel.frame() && kernel.frame_cancellations().is_cancelled(frame).is_err() {
            return Err(PiiSnapshotStoreError::Owner);
        }
        Ok(Self {
            session: kernel.session(),
            kernel,
            frame,
            sealer,
        })
    }
    /// Seal and persist bytes under real kernel inspection/shutdown ownership.
    /// # Errors
    /// Foreign owner, encryption/capacity, CAS, closed admission or owner-task failure.
    pub async fn save(&self, vault: Arc<PiiVault>) -> Result<Cid, PiiSnapshotStoreError> {
        if !vault.belongs_to(self.session, self.frame) {
            return Err(PiiSnapshotStoreError::Owner);
        }
        let owner = self
            .kernel
            .inspection_owner()
            .await
            .map_err(|_| PiiSnapshotStoreError::Storage)?;
        let sealer = Arc::clone(&self.sealer);
        tokio::task::spawn_blocking(move || {
            owner
                .ensure_open()
                .map_err(|_| PiiSnapshotStoreError::Storage)?;
            let blob = vault.export_sealed(sealer.as_ref())?;
            let expected = crate::storage::cas::content_id(blob.ciphertext());
            let cid = owner
                .cas()
                .put(blob.ciphertext())
                .map_err(|_| PiiSnapshotStoreError::Storage)?;
            let read = owner
                .cas()
                .get_bounded(&cid, 1_052_672)
                .map_err(|_| PiiSnapshotStoreError::Storage)?;
            if cid != expected || read.as_ref() != blob.ciphertext() {
                return Err(PiiSnapshotStoreError::Storage);
            }
            Ok(cid)
        })
        .await
        .map_err(|_| PiiSnapshotStoreError::Storage)?
    }
    /// Read bounded actual ciphertext, verify its CID, and authenticate the exact owner and map.
    /// # Errors
    /// Missing/corrupt/oversized CAS object, admission, authentication, owner or configured capacity.
    pub async fn load(
        &self,
        cid: Cid,
        entropy: EntropyCfg,
        limits: PiiLimits,
    ) -> Result<PiiVault, PiiSnapshotStoreError> {
        let owner = self
            .kernel
            .inspection_owner()
            .await
            .map_err(|_| PiiSnapshotStoreError::Storage)?;
        let sealer = Arc::clone(&self.sealer);
        let session = self.session;
        let frame = self.frame;
        tokio::task::spawn_blocking(move || {
            owner
                .ensure_open()
                .map_err(|_| PiiSnapshotStoreError::Storage)?;
            let bytes = owner
                .cas()
                .get_bounded(&cid, 1_052_672)
                .map_err(|_| PiiSnapshotStoreError::Storage)?;
            if crate::storage::cas::content_id(&bytes) != cid {
                return Err(PiiSnapshotStoreError::Storage);
            }
            PiiVault::import_sealed(
                session,
                frame,
                entropy,
                limits,
                sealer.as_ref(),
                &SealedBlob::from_ciphertext(bytes.to_vec()),
            )
            .map_err(PiiSnapshotStoreError::Snapshot)
        })
        .await
        .map_err(|_| PiiSnapshotStoreError::Storage)?
    }
}

/// Authenticated snapshot sealer backed by the actual ChaCha20-Poly1305 secret vault.
///
/// `SecretVault::new` uses an ephemeral key: snapshots survive vault reconstruction only while
/// that key owner remains alive. `from_key_source` instead reads an explicitly provisioned stable
/// key. It enables reconstruction with the same credential source without provisioning or rotating it.
pub struct KernelPiiSealer {
    vault: Arc<SecretVault>,
}
impl fmt::Debug for KernelPiiSealer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KernelPiiSealer(<host-owned key>)")
    }
}
impl KernelPiiSealer {
    /// Recover a stable sealing key from the host's explicit source without mutating that source.
    /// # Errors
    /// Missing/malformed key or credential-source access failure. No replacement key is generated.
    pub fn from_key_source(
        location: &polaris_sdk::secrets::SecretLocation,
        source: &impl polaris_sdk::secrets::SecretSource,
    ) -> Result<Self, PiiError> {
        let vault = SecretVault::with_seal_key_from_source(
            location,
            source,
            polaris_security::secrets::VaultLimits::new(0, 0),
        )
        .map_err(|_| PiiError::Snapshot)?;
        Ok(Self::new(Arc::new(vault)))
    }
    /// Bind an explicitly retained key owner. Never obtains ambient credentials or writes keys.
    #[must_use]
    pub fn new(vault: Arc<SecretVault>) -> Self {
        Self { vault }
    }
}
impl Sealer for KernelPiiSealer {
    fn seal(&self, plaintext: &[u8]) -> Result<SealedBlob, PiiError> {
        if plaintext.len() > polaris_security::secrets::MAX_SECRET_BYTES {
            return Err(PiiError::Capacity);
        }
        let blob = self
            .vault
            .seal_bytes(plaintext)
            .map_err(|_| PiiError::Snapshot)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve(blob.ciphertext().len().saturating_add(12))
            .map_err(|_| PiiError::Capacity)?;
        bytes.extend_from_slice(blob.nonce());
        bytes.extend_from_slice(blob.ciphertext());
        Ok(SealedBlob::from_ciphertext(bytes))
    }
    fn open(&self, blob: &SealedBlob) -> Result<Zeroizing<Vec<u8>>, PiiError> {
        let bytes = blob.ciphertext();
        let nonce = bytes
            .get(..12)
            .ok_or(PiiError::Snapshot)?
            .try_into()
            .map_err(|_| PiiError::Snapshot)?;
        let ciphertext = bytes.get(12..).ok_or(PiiError::Snapshot)?;
        if ciphertext.len() > polaris_security::secrets::MAX_SECRET_BYTES.saturating_add(16) {
            return Err(PiiError::Capacity);
        }
        let blob =
            SecretBlob::from_parts(nonce, ciphertext.to_vec()).map_err(|_| PiiError::Snapshot)?;
        self.vault.open_bytes(&blob).map_err(|_| PiiError::Snapshot)
    }
}
