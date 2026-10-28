//! Exact cost estimation and synchronized admission reservations.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use thiserror::Error;

const NANOS_PER_HOUR: u128 = 3_600_000_000_000;

/// Errors returned by budget admission and settlement.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BudgetError {
    /// The proposed reservation would exceed the configured aggregate budget.
    #[error("budget exceeded")]
    BudgetExceeded,
    /// A cost computation overflowed its integer representation.
    #[error("cost arithmetic overflow")]
    ArithmeticOverflow,
    /// Actual terminal cost exceeded the amount reserved before admission.
    #[error("settlement exceeds reservation")]
    SettlementExceedsReservation,
    /// Shared accounting was poisoned after a thread panic.
    #[error("budget accounting lock is poisoned")]
    LockPoisoned,
}

#[derive(Debug, Default)]
struct BudgetState {
    spent_micros: u64,
    outstanding_micros: u64,
    admission_halted: bool,
}

#[derive(Debug)]
struct SharedBudget {
    limit_micros: u64,
    state: Mutex<BudgetState>,
}

/// Thread-safe aggregate cloud spend guard, denominated in integer micro-USD.
#[derive(Clone, Debug)]
pub struct BudgetGuard {
    shared: Arc<SharedBudget>,
}

/// Read-only view of committed and reserved budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BudgetSnapshot {
    /// Cost committed by terminal settlements.
    pub spent_micros: u64,
    /// Cost reserved for admitted but unsettled lifetimes.
    pub outstanding_micros: u64,
    /// Configured aggregate spend limit.
    pub limit_micros: u64,
    /// True once budget accounting latches fail-closed.
    pub admission_halted: bool,
}

/// Budget reservation. Dropping without settlement leaves the reservation held,
/// which fails closed if the caller loses track of a provider resource.
#[derive(Debug)]
pub struct Reservation {
    shared: Arc<SharedBudget>,
    reserved_micros: u64,
}

impl BudgetGuard {
    /// Create a guard with a hard aggregate limit in integer micro-USD.
    #[must_use]
    pub fn new(limit_micros: u64) -> Self {
        Self {
            shared: Arc::new(SharedBudget {
                limit_micros,
                state: Mutex::new(BudgetState::default()),
            }),
        }
    }

    /// Reserve the rounded-up cost of a maximum VM lifetime before admission.
    pub fn reserve(
        &self,
        hourly_rate_micros: u64,
        maximum_lifetime: Duration,
    ) -> Result<Reservation, BudgetError> {
        if self.shared.limit_micros == 0 {
            return Err(BudgetError::BudgetExceeded);
        }
        let cost = estimate_cost(hourly_rate_micros, maximum_lifetime)?;
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| BudgetError::LockPoisoned)?;
        if state.admission_halted {
            return Err(BudgetError::BudgetExceeded);
        }
        let committed = state
            .spent_micros
            .checked_add(state.outstanding_micros)
            .ok_or(BudgetError::ArithmeticOverflow)?;
        let after_reservation = committed
            .checked_add(cost)
            .ok_or(BudgetError::ArithmeticOverflow)?;
        if after_reservation > self.shared.limit_micros {
            return Err(BudgetError::BudgetExceeded);
        }
        state.outstanding_micros = state
            .outstanding_micros
            .checked_add(cost)
            .ok_or(BudgetError::ArithmeticOverflow)?;
        Ok(Reservation {
            shared: Arc::clone(&self.shared),
            reserved_micros: cost,
        })
    }

    /// Snapshot synchronized accounting state.
    pub fn snapshot(&self) -> Result<BudgetSnapshot, BudgetError> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| BudgetError::LockPoisoned)?;
        Ok(BudgetSnapshot {
            spent_micros: state.spent_micros,
            outstanding_micros: state.outstanding_micros,
            limit_micros: self.shared.limit_micros,
            admission_halted: state.admission_halted,
        })
    }
}

impl Reservation {
    /// Return the amount held for this reservation.
    #[must_use]
    pub const fn reserved_micros(&self) -> u64 {
        self.reserved_micros
    }

    /// Commit the actual terminal cost and release only unused reserved headroom.
    ///
    /// The reservation is consumed, so it cannot be settled a second time. Committed
    /// spend is monotonic and is never refunded by this policy core.
    pub fn settle(self, actual_micros: u64) -> Result<(), BudgetError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| BudgetError::LockPoisoned)?;
        let is_overrun = actual_micros > self.reserved_micros;
        if is_overrun {
            state.admission_halted = true;
        }
        let Some(spent) = state.spent_micros.checked_add(actual_micros) else {
            state.admission_halted = true;
            return Err(BudgetError::ArithmeticOverflow);
        };
        let Some(outstanding) = state.outstanding_micros.checked_sub(self.reserved_micros) else {
            state.admission_halted = true;
            return Err(BudgetError::ArithmeticOverflow);
        };
        let Some(total) = spent.checked_add(outstanding) else {
            state.admission_halted = true;
            state.spent_micros = spent;
            state.outstanding_micros = outstanding;
            return Err(BudgetError::ArithmeticOverflow);
        };
        state.spent_micros = spent;
        state.outstanding_micros = outstanding;
        if is_overrun {
            return Err(BudgetError::SettlementExceedsReservation);
        }
        if total > self.shared.limit_micros {
            state.admission_halted = true;
            return Err(BudgetError::BudgetExceeded);
        }
        Ok(())
    }
}

fn estimate_cost(hourly_rate_micros: u64, duration: Duration) -> Result<u64, BudgetError> {
    let rate = u128::from(hourly_rate_micros);
    let product = rate
        .checked_mul(duration.as_nanos())
        .ok_or(BudgetError::ArithmeticOverflow)?;
    let estimate = product.div_ceil(NANOS_PER_HOUR);
    u64::try_from(estimate).map_err(|_| BudgetError::ArithmeticOverflow)
}
