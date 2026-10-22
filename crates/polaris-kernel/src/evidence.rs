//! Real local CAS adapters for approval evidence and masked CI logs.
use crate::{
    KernelCtx,
    storage::cas::CasError,
    verifier::{EvidenceError, EvidenceStore},
};
use polaris_sdk::ids::Cid;
use std::sync::Arc;

/// Composition-root adapter sharing the active kernel's durable CAS.
///
/// These calls perform blocking I/O. An asynchronous daemon must run the surrounding approval
/// workflow on an owner/blocking worker. The hybrid verifier checks returned content identities
/// and exact read-back bytes before minting an approval witness.
#[derive(Debug, Clone)]
pub struct KernelEvidence {
    context: Arc<KernelCtx>,
}

impl KernelEvidence {
    /// Bind evidence to the active kernel's existing storage owner.
    #[must_use]
    pub fn new(context: Arc<KernelCtx>) -> Self {
        Self { context }
    }
}

impl EvidenceStore for KernelEvidence {
    fn put(&self, bytes: &[u8]) -> Result<Cid, EvidenceError> {
        self.context
            .ensure_open()
            .map_err(|_| EvidenceError::Backend)?;
        self.context
            .cas
            .put(bytes)
            .map_err(|_| EvidenceError::Backend)
    }
    fn get(&self, cid: &Cid) -> Result<Option<Vec<u8>>, EvidenceError> {
        self.context
            .ensure_open()
            .map_err(|_| EvidenceError::Backend)?;
        match self.context.cas.get(cid) {
            Ok(bytes) => Ok(Some(bytes.to_vec())),
            Err(CasError::NotFound(_)) => Ok(None),
            Err(CasError::Corrupt { .. } | CasError::Decompress(_) | CasError::Format(_)) => {
                Err(EvidenceError::DigestMismatch)
            }
            Err(
                CasError::Io(_)
                | CasError::UnknownShortRef(_)
                | CasError::Ambiguous(_)
                | CasError::ShortRefCollision(_)
                | CasError::TooLarge { .. },
            ) => Err(EvidenceError::Backend),
        }
    }
}

impl polaris_devenv::cicd::CiLogStore for KernelEvidence {
    fn store(&self, bytes: &[u8]) -> Result<Cid, String> {
        self.context
            .ensure_open()
            .map_err(|_| "kernel storage is closed".to_owned())?;
        self.context
            .cas
            .put(bytes)
            .map_err(|_| "kernel CAS log write failed".to_owned())
    }
}
