//! GCRA diagnostics and shared Retry-After helpers.

use crate::Priority;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub(crate) const NANOS_PER_MINUTE: u64 = 60_000_000_000;

/// Why an admission attempt was denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// Request-per-minute cell is full.
    RpmExhausted,
    /// Token-weighted tokens-per-minute cell is full.
    TpmExhausted,
    /// Local pressure policy sheds this priority.
    Shed(Priority),
    /// A shared Retry-After penalty is active.
    PenaltyActive,
    /// Requested estimate exceeds the configured token burst.
    ExceedsBurst,
    /// Checked arithmetic could not represent valid state.
    Accounting,
    /// A fixed ledger slot is unavailable without waiting.
    Busy,
}

/// Admission denial with a conservative retry delay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Denied {
    /// Minimum wait suggested before another attempt.
    pub retry_after: Duration,
    /// Admission reason.
    pub reason: DenyReason,
}

impl Denied {
    pub(crate) const fn new(reason: DenyReason, retry_after: Duration) -> Self {
        Self {
            retry_after,
            reason,
        }
    }

    pub(crate) const fn accounting() -> Self {
        Self::new(DenyReason::Accounting, Duration::ZERO)
    }

    pub(crate) const fn busy() -> Self {
        Self::new(DenyReason::Busy, Duration::ZERO)
    }
}

/// Current pressure as percent of the burst horizon, independently for RPM and TPM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Utilization {
    /// Request-cell pressure from 0 through 100.
    pub rpm_pct: u8,
    /// Token-cell pressure from 0 through 100.
    pub tpm_pct: u8,
}

pub(crate) fn duration_ns(duration: Duration) -> Option<u64> {
    u64::try_from(duration.as_nanos()).ok()
}

pub(crate) fn atomic_max(target: &AtomicU64, value: u64) {
    let mut current = target.load(Ordering::Acquire);
    while current < value {
        match target.compare_exchange_weak(current, value, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return,
            Err(observed) => current = observed,
        }
    }
}
