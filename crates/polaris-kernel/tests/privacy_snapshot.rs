//! Actual ChaCha20-Poly1305 adapter contracts; synthetic canaries, no OS keyring credentials.
use polaris_kernel::privacy_snapshot::KernelPiiSealer;
use polaris_sdk::{
    ids::{FrameId, SessionId},
    registry::SandboxHandle,
};
use polaris_security::{
    automata::EntropyCfg,
    pii::{PiiLimits, PiiVault, SandboxUnmaskAuthorizer, SealedBlob},
    secrets::SecretVault,
};
use std::sync::Arc;
type TestResult = Result<(), Box<dyn std::error::Error>>;
struct ExactOwner(SessionId, FrameId);
impl SandboxUnmaskAuthorizer for ExactOwner {
    fn permits(&self, s: SessionId, f: FrameId, _: &SandboxHandle) -> bool {
        s == self.0 && f == self.1
    }
}

struct RecoverySource(secrecy::SecretString);
impl polaris_sdk::secrets::SecretSource for RecoverySource {
    fn load(
        &self,
        _: &polaris_sdk::secrets::SecretLocation,
    ) -> Result<Option<secrecy::SecretString>, polaris_sdk::secrets::SecretSourceError> {
        Ok(Some(self.0.clone()))
    }
    fn store(
        &self,
        _: &polaris_sdk::secrets::SecretLocation,
        _: &secrecy::SecretString,
    ) -> Result<(), polaris_sdk::secrets::SecretSourceError> {
        Err(polaris_sdk::secrets::SecretSourceError::Unsupported)
    }
    fn delete(
        &self,
        _: &polaris_sdk::secrets::SecretLocation,
    ) -> Result<bool, polaris_sdk::secrets::SecretSourceError> {
        Err(polaris_sdk::secrets::SecretSourceError::Unsupported)
    }
}

#[tokio::test]
async fn snapshot_reopens_kernel_cas_using_explicit_recovered_key() -> TestResult {
    use polaris_kernel::{Kernel, LocalKernelConfig, privacy_snapshot::KernelPiiSnapshotStore};
    use polaris_sdk::secrets::{KeyringLocation, SecretLocation};
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let session = SessionId::new_v7();
    let frame = FrameId::new_v7();
    let config = LocalKernelConfig::new(state.path(), workspace.path(), session, frame);
    let source = RecoverySource(secrecy::SecretString::from("47".repeat(32)));
    let location = SecretLocation::Keyring(KeyringLocation::new(
        "polaris.synthetic",
        "snapshot.recovery",
    )?);
    let kernel = Kernel::start(config.clone()).await?;
    let sealer = Arc::new(KernelPiiSealer::from_key_source(&location, &source)?);
    let store = KernelPiiSnapshotStore::new(kernel.clone(), frame, sealer)?;
    let vault = Arc::new(PiiVault::new(
        session,
        frame,
        EntropyCfg::default(),
        PiiLimits::default(),
    )?);
    let masked = vault.mask("recovery.canary@example.com")?;
    let cid = store.save(vault).await?;
    kernel.shutdown().await?;
    drop(store);
    drop(kernel);
    let recovered = Kernel::start(config).await?;
    let sealer = Arc::new(KernelPiiSealer::from_key_source(&location, &source)?);
    let store = KernelPiiSnapshotStore::new(recovered.clone(), frame, sealer)?;
    let vault = store
        .load(cid, EntropyCfg::default(), PiiLimits::default())
        .await?;
    assert_eq!(vault.mask("recovery.canary@example.com")?.text, masked.text);
    recovered.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn kernel_snapshot_cas_roundtrip_is_private_and_owner_bound() -> TestResult {
    use polaris_kernel::{Kernel, LocalKernelConfig, privacy_snapshot::KernelPiiSnapshotStore};
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let session = SessionId::new_v7();
    let frame = FrameId::new_v7();
    let kernel = Kernel::start(LocalKernelConfig::new(
        state.path(),
        workspace.path(),
        session,
        frame,
    ))
    .await?;
    let sealer = Arc::new(KernelPiiSealer::new(Arc::new(SecretVault::new()?)));
    let store = KernelPiiSnapshotStore::new(kernel.clone(), frame, Arc::clone(&sealer))?;
    let vault = Arc::new(PiiVault::new(
        session,
        frame,
        EntropyCfg::default(),
        PiiLimits::default(),
    )?);
    let masked = vault.mask("durable.canary@example.com")?;
    let cid = store.save(Arc::clone(&vault)).await?;
    let restored = store
        .load(cid, EntropyCfg::default(), PiiLimits::default())
        .await?;
    assert_eq!(
        restored.mask("durable.canary@example.com")?.text,
        masked.text
    );
    let foreign = Arc::new(PiiVault::new(
        session,
        FrameId::new_v7(),
        EntropyCfg::default(),
        PiiLimits::default(),
    )?);
    assert!(store.save(foreign).await.is_err());
    assert!(
        store
            .load(
                polaris_sdk::ids::Cid::from_bytes([9; 32]),
                EntropyCfg::default(),
                PiiLimits::default()
            )
            .await
            .is_err()
    );
    let events = kernel.context().records(
        polaris_sdk::ids::Seq::new(1),
        polaris_sdk::ids::Seq::new(u64::MAX),
    )?;
    assert!(!serde_json::to_string(&events)?.contains("durable.canary@example.com"));
    kernel.shutdown().await?;
    Ok(())
}
#[test]
fn sealed_snapshot_restores_exact_markers_with_real_aead() -> TestResult {
    let session = SessionId::new_v7();
    let frame = FrameId::new_v7();
    let vault = PiiVault::new(session, frame, EntropyCfg::default(), PiiLimits::default())?;
    let canary = "Reach synthetic.canary@example.com from 192.168.42.17";
    let masked = vault.mask(canary)?;
    assert!(!masked.text.contains("synthetic.canary@example.com"));
    let sealer = KernelPiiSealer::new(Arc::new(SecretVault::new()?));
    let blob = vault.export_sealed(&sealer)?;
    assert!(
        !blob
            .ciphertext()
            .windows(canary.len())
            .any(|b| b == canary.as_bytes())
    );
    let restored = PiiVault::import_sealed(
        session,
        frame,
        EntropyCfg::default(),
        PiiLimits::default(),
        &sealer,
        &blob,
    )?;
    assert_eq!(restored.mask(canary)?.text, masked.text);
    let sandbox = SandboxHandle::new("snapshot-contract")?;
    assert_eq!(
        restored
            .unmask_for_sandbox(&masked.text, &sandbox, &ExactOwner(session, frame))?
            .as_str(),
        canary
    );
    assert!(
        restored
            .unmask_for_sandbox(
                &masked.text,
                &sandbox,
                &ExactOwner(session, FrameId::new_v7())
            )
            .is_err()
    );
    Ok(())
}
#[test]
fn snapshot_rejects_wrong_owner_key_tamper_and_capacity() -> TestResult {
    let session = SessionId::new_v7();
    let frame = FrameId::new_v7();
    let vault = PiiVault::new(session, frame, EntropyCfg::default(), PiiLimits::default())?;
    vault.mask("a@example.com and b@example.com")?;
    let sealer = KernelPiiSealer::new(Arc::new(SecretVault::new()?));
    let blob = vault.export_sealed(&sealer)?;
    let restore = |s, f, limits, sealer: &KernelPiiSealer, blob: &SealedBlob| {
        PiiVault::import_sealed(s, f, EntropyCfg::default(), limits, sealer, blob)
    };
    assert!(
        restore(
            SessionId::new_v7(),
            frame,
            PiiLimits::default(),
            &sealer,
            &blob
        )
        .is_err()
    );
    assert!(
        restore(
            session,
            FrameId::new_v7(),
            PiiLimits::default(),
            &sealer,
            &blob
        )
        .is_err()
    );
    let other = KernelPiiSealer::new(Arc::new(SecretVault::new()?));
    assert!(restore(session, frame, PiiLimits::default(), &other, &blob).is_err());
    let mut bytes = blob.ciphertext().to_vec();
    let last = bytes.last_mut().ok_or("empty ciphertext")?;
    *last ^= 1;
    assert!(
        restore(
            session,
            frame,
            PiiLimits::default(),
            &sealer,
            &SealedBlob::from_ciphertext(bytes)
        )
        .is_err()
    );
    assert!(
        restore(
            session,
            frame,
            PiiLimits {
                max_entries: 1,
                ..PiiLimits::default()
            },
            &sealer,
            &blob
        )
        .is_err()
    );
    Ok(())
}
