//! Lease records (file / AST-scope locks). Durable across crashes: on reopen, leases owned by
//! another process are reported as stale for the Reaper instead of being silently dropped.

use polaris_sdk::ids::FrameId;
use redb::{ReadableDatabase, ReadableTable};
use serde::{Deserialize, Serialize};

use crate::error::EventLogError;
use crate::log::{EventLog, LOCKS, META};

/// One lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockRecord {
    /// Leased resource (workspace path or `path::symbol` scope).
    pub resource: String,
    /// Holding frame.
    pub holder: FrameId,
    /// Lease number (unique, increasing).
    pub lease: u64,
    /// Process that took the lease.
    pub owner_pid: u32,
}

fn decode(bytes: &[u8]) -> Result<LockRecord, EventLogError> {
    serde_json::from_slice(bytes).map_err(|e| EventLogError::Codec(e.to_string()))
}

impl EventLog {
    /// Leases `resource` for `holder` (re-entrant for the same holder).
    ///
    /// # Errors
    /// [`EventLogError::LockConflict`] when another holder has it; storage errors.
    pub fn acquire_lock(
        &self,
        resource: &str,
        holder: FrameId,
    ) -> Result<LockRecord, EventLogError> {
        let txn = self.db.begin_write()?;
        let record = {
            let mut locks = txn.open_table(LOCKS)?;
            let existing = match locks.get(resource)? {
                Some(guard) => Some(decode(guard.value())?),
                None => None,
            };
            if let Some(existing) = existing {
                if existing.holder == holder {
                    return Ok(existing);
                }
                return Err(EventLogError::LockConflict {
                    resource: resource.to_owned(),
                    holder: existing.holder,
                });
            }
            let mut meta = txn.open_table(META)?;
            let lease = meta.get("next_lease")?.map_or(1, |g| g.value());
            meta.insert("next_lease", lease.saturating_add(1))?;
            let record = LockRecord {
                resource: resource.to_owned(),
                holder,
                lease,
                owner_pid: self.owner_pid,
            };
            let bytes =
                serde_json::to_vec(&record).map_err(|e| EventLogError::Codec(e.to_string()))?;
            locks.insert(resource, bytes.as_slice())?;
            record
        };
        txn.commit()?;
        Ok(record)
    }

    /// Releases lease `lease` on `resource`.
    ///
    /// # Errors
    /// [`EventLogError::UnknownLease`] when that lease is not held; storage errors.
    pub fn release_lock(&self, resource: &str, lease: u64) -> Result<(), EventLogError> {
        let txn = self.db.begin_write()?;
        {
            let mut locks = txn.open_table(LOCKS)?;
            let held = match locks.get(resource)? {
                Some(guard) => Some(decode(guard.value())?),
                None => None,
            };
            match held {
                Some(record) if record.lease == lease => {
                    locks.remove(resource)?;
                }
                _ => {
                    return Err(EventLogError::UnknownLease {
                        resource: resource.to_owned(),
                        lease,
                    });
                }
            }
        }
        txn.commit()?;
        Ok(())
    }

    /// Every lease, ordered by resource.
    ///
    /// # Errors
    /// Storage or codec errors.
    pub fn locks(&self) -> Result<Vec<LockRecord>, EventLogError> {
        let txn = self.db.begin_read()?;
        let locks = txn.open_table(LOCKS)?;
        let mut out = Vec::new();
        for item in locks.iter()? {
            let (_, value) = item?;
            out.push(decode(value.value())?);
        }
        Ok(out)
    }

    /// Leases taken by another process (e.g. one that crashed): candidates for the Reaper.
    ///
    /// # Errors
    /// Storage or codec errors.
    pub fn stale_locks(&self) -> Result<Vec<LockRecord>, EventLogError> {
        Ok(self
            .locks()?
            .into_iter()
            .filter(|l| l.owner_pid != self.owner_pid)
            .collect())
    }
}
