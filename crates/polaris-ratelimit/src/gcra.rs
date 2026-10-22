//! Atomic GCRA cells used for request and token weighted admission.

use crate::{Priority, RateLimitError};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const NANOS_PER_MINUTE: u64 = 60_000_000_000;

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
        Self {
            retry_after: Duration::ZERO,
            reason: DenyReason::Accounting,
        }
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

pub(crate) struct GcraCell {
    theoretical_arrival_ns: AtomicU64,
    period_ns: u64,
    tolerance_ns: u64,
    burst: u32,
    reason: DenyReason,
}

pub(crate) struct GcraReservation {
    old_tat_ns: u64,
    new_tat_ns: u64,
}

impl GcraCell {
    pub(crate) fn new(
        rate_per_minute: u32,
        burst: u32,
        reason: DenyReason,
    ) -> Result<Self, RateLimitError> {
        if rate_per_minute == 0 || burst == 0 {
            return Err(RateLimitError::InvalidQuota);
        }
        let rate = u64::from(rate_per_minute);
        let period_ns = NANOS_PER_MINUTE
            .checked_add(rate - 1)
            .ok_or(RateLimitError::InvalidQuota)?
            / rate;
        let tolerance_ns = period_ns
            .checked_mul(u64::from(burst - 1))
            .ok_or(RateLimitError::InvalidQuota)?;
        Ok(Self {
            theoretical_arrival_ns: AtomicU64::new(0),
            period_ns,
            tolerance_ns,
            burst,
            reason,
        })
    }

    pub(crate) fn reserve(&self, now_ns: u64, cost: u64) -> Result<GcraReservation, Denied> {
        if cost == 0 || cost > u64::from(self.burst) {
            return Err(Denied::new(DenyReason::ExceedsBurst, Duration::ZERO));
        }
        loop {
            let old = self.theoretical_arrival_ns.load(Ordering::Acquire);
            let threshold = now_ns.saturating_add(self.tolerance_ns);
            if old > threshold {
                return Err(Denied::new(
                    self.reason,
                    Duration::from_nanos(old - threshold),
                ));
            }
            let baseline = old.max(now_ns);
            let delta = self
                .period_ns
                .checked_mul(cost)
                .ok_or_else(Denied::accounting)?;
            let next = baseline.checked_add(delta).ok_or_else(Denied::accounting)?;
            match self.theoretical_arrival_ns.compare_exchange_weak(
                old,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(GcraReservation {
                        old_tat_ns: old,
                        new_tat_ns: next,
                    });
                }
                Err(_) => continue,
            }
        }
    }

    pub(crate) fn rollback_if_unchanged(&self, reservation: GcraReservation) {
        let _ = self.theoretical_arrival_ns.compare_exchange(
            reservation.new_tat_ns,
            reservation.old_tat_ns,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    /// Charge actual post-response overage without rejecting already-consumed provider usage.
    pub(crate) fn charge_actual(&self, now_ns: u64, cost: u64) -> Result<(), RateLimitError> {
        if cost == 0 {
            return Ok(());
        }
        loop {
            let old = self.theoretical_arrival_ns.load(Ordering::Acquire);
            let baseline = old.max(now_ns);
            let delta = self.period_ns.checked_mul(cost);
            let next = delta.and_then(|value| baseline.checked_add(value));
            let Some(next) = next else {
                atomic_max(&self.theoretical_arrival_ns, u64::MAX);
                return Err(RateLimitError::Accounting);
            };
            match self.theoretical_arrival_ns.compare_exchange_weak(
                old,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(()),
                Err(_) => continue,
            }
        }
    }

    pub(crate) fn utilization_pct(&self, now_ns: u64) -> u8 {
        let tat = self.theoretical_arrival_ns.load(Ordering::Acquire);
        let debt = tat.saturating_sub(now_ns);
        let horizon = self.tolerance_ns.saturating_add(self.period_ns).max(1);
        let ratio = debt.saturating_mul(100) / horizon;
        match u8::try_from(ratio.min(100)) {
            Ok(value) => value,
            Err(_) => 100,
        }
    }

    pub(crate) fn retry_after_pct(&self, now_ns: u64, threshold_pct: u8) -> Duration {
        let tat = self.theoretical_arrival_ns.load(Ordering::Acquire);
        let debt = tat.saturating_sub(now_ns);
        let horizon = self.tolerance_ns.saturating_add(self.period_ns).max(1);
        let allowed_debt = horizon.saturating_mul(u64::from(threshold_pct)) / 100;
        Duration::from_nanos(debt.saturating_sub(allowed_debt))
    }
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
