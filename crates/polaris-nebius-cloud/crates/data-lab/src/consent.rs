//! Persisted session consent and synchronized revocation.
use crate::DataLabError;
use polaris_sdk::{
    contracts::Timestamp,
    ids::{Cid, SessionId},
};
use polaris_storage_cas::{Cas, content_id};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock, RwLockReadGuard};

/// Consent record persisted by an explicit user-directed configuration action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentRecord {
    /// Current record version; only version one is accepted.
    pub schema: u16,
    /// Session whose trajectories may be collected.
    pub session: SessionId,
    /// Whether local collection/export is enabled. Remote transmission is separate policy.
    pub enabled: bool,
    /// Monotonically increasing revision supplied by the consent owner.
    pub revision: u64,
    /// When the user made the decision.
    pub recorded_at: Timestamp,
}

#[derive(Debug)]
struct ConsentState {
    enabled: bool,
    revision: u64,
    record: Cid,
}

/// Loaded consent, shared by collectors/exporters for one session.
///
/// The composition root must resolve the latest persisted consent CID; a historical enabled
/// record is not evidence that consent is still enabled. `refresh` rejects rollback revisions.
#[derive(Debug, Clone)]
pub struct OptIn {
    session: SessionId,
    state: Arc<RwLock<ConsentState>>,
}

impl OptIn {
    /// Read a versioned consent record from actual CAS bytes.
    ///
    /// # Errors
    /// Missing/corrupt records and disabled or mismatched consent deny collection.
    pub fn load(cas: &Cas, record: Cid, session: SessionId) -> Result<Self, DataLabError> {
        let parsed = read(cas, record)?;
        if parsed.session != session || !parsed.enabled {
            return Err(DataLabError::NotOptedIn);
        }
        Ok(Self {
            session,
            state: Arc::new(RwLock::new(ConsentState {
                enabled: true,
                revision: parsed.revision,
                record,
            })),
        })
    }

    /// Session to which this consent applies.
    #[must_use]
    pub const fn session(&self) -> SessionId {
        self.session
    }

    /// Apply a newer persisted decision; older or conflicting same-revision records are denied.
    ///
    /// # Errors
    /// CAS/record errors, rollback attempts or poisoned synchronization state.
    pub fn refresh(&self, cas: &Cas, record: Cid) -> Result<(), DataLabError> {
        let parsed = read(cas, record)?;
        if parsed.session != self.session {
            return Err(DataLabError::NotOptedIn);
        }
        let mut state = self
            .state
            .write()
            .map_err(|_| DataLabError::ConsentUnavailable)?;
        if parsed.revision < state.revision
            || (parsed.revision == state.revision && record != state.record)
        {
            state.enabled = false;
            return Err(DataLabError::NotOptedIn);
        }
        // Re-reading a historical enabled record cannot undo an immediate local revocation.
        if parsed.enabled && !state.enabled && parsed.revision <= state.revision {
            return Err(DataLabError::NotOptedIn);
        }
        state.enabled = parsed.enabled;
        state.revision = parsed.revision;
        state.record = record;
        Ok(())
    }

    /// Immediately stop collection/export in this process. Persistence remains the consent
    /// owner's responsibility; this method waits for an already-started local commit to finish.
    ///
    /// # Errors
    /// Poisoned synchronization state.
    pub fn revoke(&self) -> Result<(), DataLabError> {
        self.state
            .write()
            .map_err(|_| DataLabError::ConsentUnavailable)?
            .enabled = false;
        Ok(())
    }

    pub(crate) fn permit(&self, session: SessionId) -> Result<ConsentPermit<'_>, DataLabError> {
        if session != self.session {
            return Err(DataLabError::NotOptedIn);
        }
        let state = self
            .state
            .read()
            .map_err(|_| DataLabError::ConsentUnavailable)?;
        if !state.enabled {
            return Err(DataLabError::NotOptedIn);
        }
        Ok(ConsentPermit { state })
    }
}

pub(crate) struct ConsentPermit<'a> {
    state: RwLockReadGuard<'a, ConsentState>,
}

impl ConsentPermit<'_> {
    pub(crate) fn record(&self) -> Cid {
        self.state.record
    }
}

fn read(cas: &Cas, cid: Cid) -> Result<ConsentRecord, DataLabError> {
    let bytes = cas.get(&cid)?;
    if bytes.len() > 16 * 1024 {
        return Err(DataLabError::Limit);
    }
    if content_id(&bytes) != cid {
        return Err(DataLabError::DigestMismatch);
    }
    let record: ConsentRecord =
        serde_json::from_slice(&bytes).map_err(|_| DataLabError::Schema("consent JSON"))?;
    if record.schema != 1 {
        return Err(DataLabError::Schema("consent version"));
    }
    Ok(record)
}
