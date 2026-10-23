//! Bounded local RPM/TPM admission with conservative reservations and shared Retry-After.
//!
//! Limits are caller-configured. This crate does not invent provider quotas or treat rate-limit
//! response headers as authoritative limits. It reserves the estimated prompt plus maximum output
//! before dispatch and charges actual usage above that reservation after the response.

mod gcra;
mod permit;
mod retry;
mod window;

use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::ids::TokenCount;
use std::{
    num::NonZeroU32,
    sync::{Arc, atomic::AtomicU64},
    time::{Duration, Instant},
};

pub use gcra::{Denied, DenyReason, Utilization};
pub use permit::{Permit, Reconciliation};
pub use retry::jittered_backoff;
pub use window::WindowSnapshot;

/// Configured local rate capacity. Burst is explicit; zero is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    /// Requests per minute.
    pub rpm: NonZeroU32,
    /// Tokens per minute.
    pub tpm: NonZeroU32,
    /// Maximum request burst admitted by the RPM GCRA cell.
    pub burst_requests: u32,
    /// Maximum token burst admitted by the weighted TPM GCRA cell.
    pub burst_tokens: u32,
}

/// Upper reservation bound for one model request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Estimate {
    /// Prompt token upper bound from a trusted tokenizer/assembler.
    pub prompt: TokenCount,
    /// Requested maximum generated tokens.
    pub max_output: TokenCount,
}

/// Admission priority, ordered from lowest to highest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Background,
    SubAgent,
    Verifier,
    InteractiveUser,
}

/// Pressure thresholds used for best-effort shedding. InteractiveUser is never shed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShedPolicy {
    /// Shed Background at or above this observed percentage.
    pub background_above_pct: u8,
    /// Shed SubAgent at or above this observed percentage.
    pub subagent_above_pct: u8,
    /// Shed Verifier at or above this observed percentage.
    pub verifier_above_pct: u8,
}

impl Default for ShedPolicy {
    fn default() -> Self {
        Self {
            background_above_pct: 70,
            subagent_above_pct: 85,
            verifier_above_pct: 95,
        }
    }
}

/// Monotonic elapsed-time source. Implementations must never move backwards.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Duration;
}

/// Process-local monotonic clock, anchored when constructed.
#[derive(Debug, Clone)]
pub struct MonotonicClock {
    origin: Instant,
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}
impl Clock for MonotonicClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// Local configuration or accounting failure.
#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    /// Invalid local capacity or shed policy.
    #[error("invalid rate-limit configuration")]
    InvalidQuota,
    /// Async admission was canceled while waiting; no permit was created.
    #[error("rate-limit wait cancelled")]
    Cancelled,
    /// An internal checked arithmetic invariant failed.
    #[error("rate-limit accounting invariant failed")]
    Accounting,
    /// The request's prompt plus output reservation exceeds the configured token burst.
    #[error("request estimate exceeds configured token burst")]
    ExceedsBurst,
    /// Provider usage was reconciled before an HTTP request was marked dispatched.
    #[error("cannot reconcile a request before dispatch")]
    Undispatched,
}

impl HasKind for RateLimitError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::InvalidQuota | Self::ExceedsBurst => ErrorKind::Policy,
            Self::Cancelled => ErrorKind::Transient,
            Self::Accounting | Self::Undispatched => ErrorKind::Fatal,
        }
    }
}

pub(crate) struct Shared<C: Clock> {
    pub clock: C,
    pub quota: Quota,
    pub rpm: gcra::GcraCell,
    pub tpm: gcra::GcraCell,
    pub penalty_until_ns: AtomicU64,
    pub windows: window::WindowTracker,
}

/// Shared provider limiter. Clones share both admission state and Retry-After penalties.
pub struct DualLimiter<C: Clock> {
    pub(crate) shared: Arc<Shared<C>>,
    shed: ShedPolicy,
}

impl<C: Clock> Clone for DualLimiter<C> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            shed: self.shed,
        }
    }
}

impl<C: Clock> DualLimiter<C> {
    /// Create a limiter from explicit provider/local quotas and a monotonic clock.
    pub fn new(quota: Quota, clock: C, shed: ShedPolicy) -> Result<Self, RateLimitError> {
        if quota.burst_requests == 0
            || quota.burst_tokens == 0
            || shed.background_above_pct > 100
            || shed.subagent_above_pct > 100
            || shed.verifier_above_pct > 100
            || shed.background_above_pct > shed.subagent_above_pct
            || shed.subagent_above_pct > shed.verifier_above_pct
        {
            return Err(RateLimitError::InvalidQuota);
        }
        let rpm = gcra::GcraCell::new(
            quota.rpm.get(),
            quota.burst_requests,
            DenyReason::RpmExhausted,
        )?;
        let tpm = gcra::GcraCell::new(
            quota.tpm.get(),
            quota.burst_tokens,
            DenyReason::TpmExhausted,
        )?;
        Ok(Self {
            shared: Arc::new(Shared {
                clock,
                quota,
                rpm,
                tpm,
                penalty_until_ns: AtomicU64::new(0),
                windows: window::WindowTracker::default(),
            }),
            shed,
        })
    }

    /// Nonblocking admission. No allocation or async timer is used by the GCRA checks.
    pub fn try_acquire(&self, estimate: Estimate, priority: Priority) -> Result<Permit<C>, Denied> {
        let now = self.shared.clock.now();
        let now_ns = gcra::duration_ns(now).ok_or_else(Denied::accounting)?;
        let penalty = self
            .shared
            .penalty_until_ns
            .load(std::sync::atomic::Ordering::Acquire);
        if penalty > now_ns {
            return Err(Denied::new(
                DenyReason::PenaltyActive,
                Duration::from_nanos(penalty - now_ns),
            ));
        }
        let cost = u64::from(estimate.prompt.get())
            .checked_add(u64::from(estimate.max_output.get()))
            .ok_or_else(Denied::accounting)?;
        if cost == 0 || cost > u64::from(self.shared.quota.burst_tokens) {
            return Err(Denied::new(DenyReason::ExceedsBurst, Duration::ZERO));
        }
        let utilization = self.utilization();
        if let Some(threshold) = self.shed_threshold(priority) {
            let pressure = utilization.rpm_pct.max(utilization.tpm_pct);
            if pressure >= threshold {
                let retry = self
                    .shared
                    .rpm
                    .retry_after_pct(now_ns, threshold)
                    .max(self.shared.tpm.retry_after_pct(now_ns, threshold));
                return Err(Denied::new(DenyReason::Shed(priority), retry));
            }
        }
        let rpm_reservation = self.shared.rpm.reserve(now_ns, 1)?;
        if let Err(denied) = self.shared.tpm.reserve(now_ns, cost) {
            // Refund only when no later concurrent reservation has advanced the cell. In the
            // contended case the compare-exchange fails and the conservative debit is retained.
            self.shared.rpm.rollback_if_unchanged(rpm_reservation);
            return Err(denied);
        }
        Ok(Permit::new(Arc::clone(&self.shared), cost))
    }

    /// Wait for local capacity. Canceling this future before it returns drops no permit and charges
    /// no additional state beyond conservative partial reservations from a raced dual-cell check.
    pub async fn acquire(
        &self,
        estimate: Estimate,
        priority: Priority,
    ) -> Result<Permit<C>, RateLimitError> {
        loop {
            match self.try_acquire(estimate, priority) {
                Ok(permit) => return Ok(permit),
                Err(denied) if denied.reason == DenyReason::Accounting => {
                    return Err(RateLimitError::Accounting);
                }
                Err(denied) if denied.reason == DenyReason::ExceedsBurst => {
                    return Err(RateLimitError::ExceedsBurst);
                }
                Err(denied) => {
                    tokio::time::sleep(denied.retry_after.max(Duration::from_millis(1))).await
                }
            }
        }
    }

    /// Extend one shared penalty for all clones after the HTTP adapter parses Retry-After.
    pub fn on_retry_after(&self, delay: Duration) -> Result<(), RateLimitError> {
        let now = gcra::duration_ns(self.shared.clock.now()).ok_or(RateLimitError::Accounting)?;
        let delay = gcra::duration_ns(delay).ok_or(RateLimitError::Accounting)?;
        let until = now.checked_add(delay).ok_or(RateLimitError::Accounting)?;
        gcra::atomic_max(&self.shared.penalty_until_ns, until);
        Ok(())
    }

    /// Apply a parsed Retry-After only for HTTP 429 responses. Header parsing and provider header
    /// names belong to the HTTP adapter; no rate-limit headers are interpreted as local quotas.
    pub fn on_response(
        &self,
        status: u16,
        retry_after: Option<Duration>,
    ) -> Result<(), RateLimitError> {
        if status == 429
            && let Some(delay) = retry_after
        {
            self.on_retry_after(delay)?;
        }
        Ok(())
    }

    /// Current local RPM/TPM GCRA pressure; this is not a provider quota hint.
    pub fn utilization(&self) -> Utilization {
        let Some(now) = gcra::duration_ns(self.shared.clock.now()) else {
            // An unrepresentably distant clock is beyond every stored arrival.
            return Utilization {
                rpm_pct: 0,
                tpm_pct: 0,
            };
        };
        Utilization {
            rpm_pct: self.shared.rpm.utilization_pct(now),
            tpm_pct: self.shared.tpm.utilization_pct(now),
        }
    }

    /// Bounded, conservative 15-minute request/reservation/actual usage snapshot.
    pub fn window(&self) -> WindowSnapshot {
        let minute = self.shared.clock.now().as_secs() / 60;
        self.shared.windows.snapshot(minute)
    }

    fn shed_threshold(&self, priority: Priority) -> Option<u8> {
        match priority {
            Priority::Background => Some(self.shed.background_above_pct),
            Priority::SubAgent => Some(self.shed.subagent_above_pct),
            Priority::Verifier => Some(self.shed.verifier_above_pct),
            Priority::InteractiveUser => None,
        }
    }
}
