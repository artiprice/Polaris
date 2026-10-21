//! Cached values, the L2 envelope and the clock used for expiry.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::CacheError;

/// Cached bytes; cloning is a reference-count increment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedValue(Arc<[u8]>);

impl CachedValue {
    /// The bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// `true` for an empty value.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<u8>> for CachedValue {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes.into())
    }
}

impl From<&[u8]> for CachedValue {
    fn from(bytes: &[u8]) -> Self {
        Self(bytes.into())
    }
}

/// Wall-clock source for TTLs (persisted in L2, so it must be wall time, not `Instant`).
pub trait Clock: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn now_unix_ms(&self) -> u64;
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }
}

/// L2 envelope version.
const VERSION: u8 = 1;
/// `[version][has expiry][expires_at u64 LE]`.
const HEADER: usize = 10;

/// Encodes an L2 value: header then payload.
pub(crate) fn encode(expires_at: Option<u64>, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER.saturating_add(payload.len()));
    out.push(VERSION);
    out.push(u8::from(expires_at.is_some()));
    out.extend_from_slice(&expires_at.unwrap_or(0).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Decodes an L2 value into its expiry and payload.
pub(crate) fn decode(bytes: &[u8]) -> Result<(Option<u64>, &[u8]), CacheError> {
    let (header, payload) = bytes.split_at_checked(HEADER).ok_or_else(|| {
        CacheError::Codec(format!(
            "entry of {} bytes is shorter than its header",
            bytes.len()
        ))
    })?;
    let (version, rest) = header
        .split_first()
        .ok_or_else(|| CacheError::Codec("empty header".into()))?;
    if *version != VERSION {
        return Err(CacheError::Codec(format!(
            "unknown envelope version {version}"
        )));
    }
    let (flag, time) = rest
        .split_first()
        .ok_or_else(|| CacheError::Codec("truncated header".into()))?;
    let time: [u8; 8] = time
        .try_into()
        .map_err(|_| CacheError::Codec("truncated expiry".into()))?;
    let expires_at = match flag {
        0 => None,
        1 => Some(u64::from_le_bytes(time)),
        other => return Err(CacheError::Codec(format!("bad expiry flag {other}"))),
    };
    Ok((expires_at, payload))
}
