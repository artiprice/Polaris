//! The L2 exact cache table, exposed through the sdk `ExactCacheStore` trait (consumed by
//! `polaris-storage-cache`). Stored value: `[scope tag][scope id][payload]`.

use polaris_sdk::registry::{CacheScope, CacheStoreError, ExactCacheStore};
use redb::{ReadableDatabase, ReadableTable};

use crate::error::EventLogError;
use crate::log::{EventLog, L2_CACHE};

fn scope_prefix(scope: &CacheScope) -> Vec<u8> {
    match scope {
        CacheScope::Global => vec![0],
        CacheScope::Workspace { workspace } => {
            let mut p = vec![1];
            p.extend_from_slice(workspace.as_bytes());
            p
        }
        CacheScope::Session { session } => {
            let mut p = vec![2];
            p.extend_from_slice(session.as_uuid().as_bytes());
            p
        }
    }
}

/// Splits a stored value into (scope prefix, payload).
fn split(value: &[u8]) -> Result<(&[u8], &[u8]), EventLogError> {
    let len = match value.first() {
        Some(0) => 1,
        Some(1) => 33,
        Some(2) => 17,
        _ => return Err(EventLogError::Codec("unknown cache scope tag".to_owned())),
    };
    if value.len() < len {
        return Err(EventLogError::Codec(
            "cache value shorter than its scope".to_owned(),
        ));
    }
    Ok(value.split_at(len))
}

impl ExactCacheStore for EventLog {
    fn get(&self, key: &[u8; 32]) -> Result<Option<Vec<u8>>, CacheStoreError> {
        let txn = self.db.begin_read().map_err(EventLogError::from)?;
        let table = txn.open_table(L2_CACHE).map_err(EventLogError::from)?;
        let Some(guard) = table.get(key.as_slice()).map_err(EventLogError::from)? else {
            return Ok(None);
        };
        let (_, payload) = split(guard.value())?;
        Ok(Some(payload.to_vec()))
    }

    fn get_bounded(
        &self,
        key: &[u8; 32],
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, CacheStoreError> {
        let txn = self.db.begin_read().map_err(EventLogError::from)?;
        let table = txn.open_table(L2_CACHE).map_err(EventLogError::from)?;
        let Some(guard) = table.get(key.as_slice()).map_err(EventLogError::from)? else {
            return Ok(None);
        };
        let (_, payload) = split(guard.value())?;
        if payload.len() > max_bytes {
            return Err(CacheStoreError::TooLarge {
                bytes: u64::try_from(payload.len()).unwrap_or(u64::MAX),
                limit: u64::try_from(max_bytes).unwrap_or(u64::MAX),
            });
        }
        Ok(Some(payload.to_vec()))
    }

    fn put(&self, key: [u8; 32], value: &[u8], scope: CacheScope) -> Result<(), CacheStoreError> {
        if value.len() > self.max_cache_value_bytes {
            let as_u64 = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
            return Err(CacheStoreError::TooLarge {
                bytes: as_u64(value.len()),
                limit: as_u64(self.max_cache_value_bytes),
            });
        }
        let mut stored = scope_prefix(&scope);
        stored.extend_from_slice(value);
        let txn = self.db.begin_write().map_err(EventLogError::from)?;
        {
            let mut table = txn.open_table(L2_CACHE).map_err(EventLogError::from)?;
            table
                .insert(key.as_slice(), stored.as_slice())
                .map_err(EventLogError::from)?;
        }
        txn.commit().map_err(EventLogError::from)?;
        Ok(())
    }

    fn clear_scope(&self, scope: &CacheScope) -> Result<u64, CacheStoreError> {
        let prefix = scope_prefix(scope);
        let txn = self.db.begin_write().map_err(EventLogError::from)?;
        let removed = {
            let mut table = txn.open_table(L2_CACHE).map_err(EventLogError::from)?;
            let mut doomed = Vec::new();
            for item in table.iter().map_err(EventLogError::from)? {
                let (key, value) = item.map_err(EventLogError::from)?;
                let (stored_scope, _) = split(value.value())?;
                if stored_scope == prefix.as_slice() {
                    doomed.push(key.value().to_vec());
                }
            }
            for key in &doomed {
                table.remove(key.as_slice()).map_err(EventLogError::from)?;
            }
            u64::try_from(doomed.len()).unwrap_or(u64::MAX)
        };
        txn.commit().map_err(EventLogError::from)?;
        Ok(removed)
    }
}
