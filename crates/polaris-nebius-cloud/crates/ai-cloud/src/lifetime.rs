//! Hard-TTL and idle-timeout evaluation using monotonic observations.

use std::time::{Duration, Instant};

use thiserror::Error;

/// Errors returned while building or evaluating a lifetime guard.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LifetimeError {
    /// TTL and idle timeout must be positive and idle cannot exceed TTL.
    #[error("invalid lifetime configuration")]
    InvalidConfiguration,
    /// A duration cannot be represented from the supplied monotonic start.
    #[error("monotonic deadline overflow")]
    DeadlineOverflow,
    /// The supplied observation precedes the most recent observation.
    #[error("non-monotonic time observation")]
    NonMonotonicTime,
}

/// Current lifetime decision for a managed VM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifetimeStatus {
    /// Neither idle timeout nor hard TTL has expired.
    Running,
    /// The absolute hard lifetime has expired.
    ExpiredByHardTtl,
    /// The idle timeout has expired before the hard TTL.
    ExpiredByIdleTimeout,
}

/// Pure local lifetime state; callers must invoke it from a scheduler.
#[derive(Clone, Debug)]
pub struct LifetimeGuard {
    hard_deadline: Instant,
    idle_timeout: Duration,
    last_activity: Instant,
    last_observed: Instant,
    expired: Option<LifetimeStatus>,
}

impl LifetimeGuard {
    /// Start a guard at the observed monotonic start instant.
    pub fn new(
        started_at: Instant,
        hard_ttl: Duration,
        idle_timeout: Duration,
    ) -> Result<Self, LifetimeError> {
        if hard_ttl.is_zero() || idle_timeout.is_zero() || idle_timeout > hard_ttl {
            return Err(LifetimeError::InvalidConfiguration);
        }
        let hard_deadline = started_at
            .checked_add(hard_ttl)
            .ok_or(LifetimeError::DeadlineOverflow)?;
        started_at
            .checked_add(idle_timeout)
            .ok_or(LifetimeError::DeadlineOverflow)?;
        Ok(Self {
            hard_deadline,
            idle_timeout,
            last_activity: started_at,
            last_observed: started_at,
            expired: None,
        })
    }

    /// Record activity, moving the idle deadline without moving the hard deadline.
    pub fn record_activity(&mut self, now: Instant) -> Result<(), LifetimeError> {
        let status = self.evaluate(now)?;
        if status == LifetimeStatus::Running {
            self.last_activity = now;
        }
        Ok(())
    }

    /// Evaluate hard TTL before idle timeout and retain monotonic observation order.
    pub fn evaluate(&mut self, now: Instant) -> Result<LifetimeStatus, LifetimeError> {
        if now < self.last_observed {
            return Err(LifetimeError::NonMonotonicTime);
        }
        if let Some(expired) = self.expired {
            self.last_observed = now;
            return Ok(expired);
        }
        let status = if now >= self.hard_deadline {
            LifetimeStatus::ExpiredByHardTtl
        } else {
            let idle_deadline = self
                .last_activity
                .checked_add(self.idle_timeout)
                .ok_or(LifetimeError::DeadlineOverflow)?;
            if now >= idle_deadline {
                LifetimeStatus::ExpiredByIdleTimeout
            } else {
                LifetimeStatus::Running
            }
        };
        self.last_observed = now;
        if status != LifetimeStatus::Running {
            self.expired = Some(status);
        }
        Ok(status)
    }
}
