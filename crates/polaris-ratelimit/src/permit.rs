//! Request reservations and post-response usage reconciliation.

use crate::{Clock, RateLimitError, Shared, gcra};
use std::{sync::Arc, time::Duration};

/// Result of reconciling provider-reported usage against the pre-dispatch reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reconciliation {
    /// Tokens reserved before dispatch.
    pub reserved_tokens: u64,
    /// Actual prompt plus completion tokens reported by the provider.
    pub actual_tokens: u64,
    /// Whether actual usage exceeded the reservation; overage was charged into the TPM cell.
    pub reservation_overrun: bool,
}

/// Single-use reservation. Dropping it never refunds a GCRA charge: requests may already have
/// reached a provider unless the caller proves otherwise. Call `mark_dispatched` immediately
/// before sending HTTP, then `reconcile` after final usage arrives.
pub struct Permit<C: Clock> {
    shared: Arc<Shared<C>>,
    reserved_tokens: u64,
    dispatched_minute: Option<u64>,
}

impl<C: Clock> Permit<C> {
    pub(crate) fn new(shared: Arc<Shared<C>>, reserved_tokens: u64) -> Self {
        Self {
            shared,
            reserved_tokens,
            dispatched_minute: None,
        }
    }

    /// Mark the point at which the HTTP request can become externally visible. Must run before
    /// handing the request to the transport. A failed ledger update prevents caller dispatch.
    pub fn mark_dispatched(&mut self) -> Result<(), RateLimitError> {
        if self.dispatched_minute.is_some() {
            return Err(RateLimitError::Accounting);
        }
        let now = self.shared.clock.now();
        let minute = now.as_secs() / 60;
        self.shared
            .windows
            .record_dispatch(minute, self.reserved_tokens)?;
        self.dispatched_minute = Some(minute);
        Ok(())
    }

    /// Reconcile final provider usage. Lower actual usage is recorded but does not refund a
    /// reservation because out-of-order GCRA rollback cannot be proven safe. Any overage is
    /// charged to TPM before this method returns; caller should surface `reservation_overrun`.
    pub fn reconcile(self, actual_tokens: u64) -> Result<Reconciliation, RateLimitError> {
        let minute = self.dispatched_minute.ok_or(RateLimitError::Undispatched)?;
        let reservation_overrun = actual_tokens > self.reserved_tokens;
        if reservation_overrun {
            let extra = actual_tokens
                .checked_sub(self.reserved_tokens)
                .ok_or(RateLimitError::Accounting)?;
            let now =
                gcra::duration_ns(self.shared.clock.now()).ok_or(RateLimitError::Accounting)?;
            self.shared.tpm.charge_actual(now, extra)?;
        }
        self.shared.windows.record_actual(minute, actual_tokens)?;
        Ok(Reconciliation {
            reserved_tokens: self.reserved_tokens,
            actual_tokens,
            reservation_overrun,
        })
    }

    /// Reserved amount used by HTTP adapters for telemetry before dispatch.
    #[must_use]
    pub const fn reserved_tokens(&self) -> u64 {
        self.reserved_tokens
    }

    /// Conservative retry duration implied by the shared penalty at the current monotonic time.
    #[must_use]
    pub fn retry_after(&self) -> Duration {
        let now = match gcra::duration_ns(self.shared.clock.now()) {
            Some(value) => value,
            None => u64::MAX,
        };
        let until = self
            .shared
            .penalty_until_ns
            .load(std::sync::atomic::Ordering::Acquire);
        Duration::from_nanos(until.saturating_sub(now))
    }
}
