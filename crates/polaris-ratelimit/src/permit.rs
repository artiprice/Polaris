//! Request reservations and post-response usage reconciliation.

use crate::{Clock, RateLimitError, Shared, gcra, ledger::ReservationId};
use std::{sync::Arc, time::Duration};

/// Result of reconciling provider-reported usage against the pre-dispatch reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reconciliation {
    /// Tokens reserved before dispatch.
    pub reserved_tokens: u64,
    /// Actual prompt plus completion tokens reported by the provider.
    pub actual_tokens: u64,
    /// Whether actual usage exceeded the reservation; overage was charged at reconciliation time.
    pub reservation_overrun: bool,
    /// Unused reserved tokens removed from the replayed GCRA history before return.
    pub refunded_tokens: u64,
    /// Unused tokens still charged because a later admission-time floor retained some debt.
    pub unused_tokens_retained: u64,
    /// True when the settlement intent is safely owned by the limiter but has not yet been folded
    /// into a published snapshot. No refund is reported until a helper publishes and acknowledges it.
    pub settlement_deferred: bool,
}

/// Single-use reservation. Dropping before dispatch publishes a cancellation intent; dropping
/// after dispatch commits the full reservation conservatively. Call `mark_dispatched` immediately
/// before handing the request to the transport, then `reconcile` after final usage arrives.
pub struct Permit<C: Clock> {
    shared: Arc<Shared<C>>,
    reserved_tokens: u64,
    dispatched_minute: Option<u64>,
    settlement_published: bool,
    reservation: ReservationId,
}

impl<C: Clock> Permit<C> {
    pub(crate) fn new(
        shared: Arc<Shared<C>>,
        reserved_tokens: u64,
        reservation: ReservationId,
    ) -> Self {
        Self {
            shared,
            reserved_tokens,
            dispatched_minute: None,
            settlement_published: false,
            reservation,
        }
    }

    /// Mark the point at which the HTTP request can become externally visible. A `Busy` error
    /// leaves this permit intact; the caller must retry before dispatching the request.
    pub fn mark_dispatched(&mut self) -> Result<(), RateLimitError> {
        if self.dispatched_minute.is_some() {
            return Err(RateLimitError::Accounting);
        }
        let now = self.shared.clock.now();
        let minute = now.as_secs().saturating_div(60);
        self.shared.ledger.dispatch(self.reservation)?;
        self.dispatched_minute = Some(minute);
        self.shared
            .windows
            .record_dispatch(minute, self.reserved_tokens);
        Ok(())
    }

    /// Reconcile final provider usage. If the bounded snapshot pool is temporarily saturated, the
    /// preallocated per-reservation intent remains owned by the limiter and future operations help
    /// publish it. In that case this result reports no refund and sets `settlement_deferred`.
    pub fn reconcile(mut self, actual_tokens: u64) -> Result<Reconciliation, RateLimitError> {
        let minute = self.dispatched_minute.ok_or(RateLimitError::Undispatched)?;
        let now_ns = match gcra::duration_ns(self.shared.clock.now()) {
            Some(value) => value,
            None => u64::MAX,
        };
        let (refunded_tokens, settlement_deferred) = self
            .shared
            .ledger
            .settle(self.reservation, actual_tokens, now_ns)?;
        self.settlement_published = true;
        self.shared.windows.record_actual(minute, actual_tokens);
        let unused_tokens = self.reserved_tokens.saturating_sub(actual_tokens);
        let reservation_overrun = actual_tokens > self.reserved_tokens;
        let (refunded_tokens, unused_tokens_retained) = if settlement_deferred {
            (0, unused_tokens)
        } else if reservation_overrun {
            (0, 0)
        } else {
            (refunded_tokens, unused_tokens.saturating_sub(refunded_tokens))
        };
        Ok(Reconciliation {
            reserved_tokens: self.reserved_tokens,
            actual_tokens,
            reservation_overrun,
            refunded_tokens,
            unused_tokens_retained,
            settlement_deferred,
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
        let Some(now) = gcra::duration_ns(self.shared.clock.now()) else {
            return Duration::ZERO;
        };
        let until = self
            .shared
            .penalty_until_ns
            .load(std::sync::atomic::Ordering::Acquire);
        Duration::from_nanos(until.saturating_sub(now))
    }
}

impl<C: Clock> Drop for Permit<C> {
    fn drop(&mut self) {
        if self.settlement_published {
            return;
        }
        let now_ns = match gcra::duration_ns(self.shared.clock.now()) {
            Some(value) => value,
            None => u64::MAX,
        };
        if self.dispatched_minute.is_some() {
            self.shared.ledger.commit_drop(self.reservation, now_ns);
        } else {
            self.shared.ledger.cancel(self.reservation, now_ns);
        }
        self.settlement_published = true;
    }
}
