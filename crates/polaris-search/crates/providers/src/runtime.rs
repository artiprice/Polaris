use polaris_ratelimit::{
    DualLimiter, Estimate, MonotonicClock, Priority, Quota, RateLimitError, ShedPolicy,
};
use polaris_sdk::ids::TokenCount;
use std::{fmt, sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::SearchProviderError;

/// Shared concurrency and caller-configured request-rate controls.
#[derive(Clone)]
pub struct ProviderRuntime {
    limiter: DualLimiter<MonotonicClock>,
    semaphore: Arc<Semaphore>,
}

impl fmt::Debug for ProviderRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderRuntime")
            .field("available_permits", &self.semaphore.available_permits())
            .finish_non_exhaustive()
    }
}

impl ProviderRuntime {
    /// Creates runtime controls from explicit local/provider quotas and maximum concurrency.
    ///
    /// # Errors
    /// Returns an error if the quota is invalid or concurrency is zero.
    pub fn new(
        quota: Quota,
        max_concurrency: usize,
        shed: ShedPolicy,
    ) -> Result<Self, SearchProviderError> {
        if max_concurrency == 0 || max_concurrency > Semaphore::MAX_PERMITS {
            return Err(SearchProviderError::ProviderConfig);
        }
        let semaphore = Semaphore::new(max_concurrency);
        let limiter = DualLimiter::new(quota, MonotonicClock::default(), shed)
            .map_err(|_| SearchProviderError::ProviderConfig)?;
        Ok(Self {
            limiter,
            semaphore: Arc::new(semaphore),
        })
    }

    /// Acquires a concurrency slot and one caller-configured request admission.
    ///
    /// # Errors
    /// Returns a sanitized admission error or timeout.
    pub(crate) async fn acquire(
        &self,
        timeout: Duration,
    ) -> Result<CallPermit, SearchProviderError> {
        let wait = async {
            let semaphore = Arc::clone(&self.semaphore)
                .acquire_owned()
                .await
                .map_err(|_| SearchProviderError::RateLimit)?;
            let permit = self
                .limiter
                .acquire(
                    Estimate {
                        prompt: TokenCount::new(1),
                        max_output: TokenCount::new(0),
                    },
                    Priority::InteractiveUser,
                )
                .await
                .map_err(map_rate_limit_error)?;
            Ok::<CallPermit, SearchProviderError>(CallPermit {
                limiter_permit: Some(permit),
                _semaphore_permit: Arc::new(semaphore),
                dispatched: false,
            })
        };
        tokio::time::timeout(timeout, wait)
            .await
            .map_err(|_| SearchProviderError::Unavailable)?
    }

    /// Applies a provider Retry-After penalty to future admissions.
    pub(crate) fn on_retry_after(
        &self,
        status: u16,
        retry_after: Option<Duration>,
    ) -> Result<(), SearchProviderError> {
        self.limiter
            .on_response(status, retry_after)
            .map_err(|_| SearchProviderError::RateLimit)
    }

    /// Returns request counters for local adapter contract tests.
    #[cfg(test)]
    pub(crate) fn window_snapshot(&self) -> polaris_ratelimit::WindowSnapshot {
        self.limiter.window()
    }
}

/// A single request admission held through response handling.
pub(crate) struct CallPermit {
    limiter_permit: Option<polaris_ratelimit::Permit<MonotonicClock>>,
    _semaphore_permit: Arc<OwnedSemaphorePermit>,
    dispatched: bool,
}

impl CallPermit {
    /// Keeps the shared concurrency slot alive while a blocking worker continues after timeout.
    pub(crate) fn worker_guard(&self) -> Arc<OwnedSemaphorePermit> {
        Arc::clone(&self._semaphore_permit)
    }

    /// Records the exact point where the request can become externally visible.
    pub(crate) async fn mark_dispatched(&mut self) -> Result<(), SearchProviderError> {
        let permit = self
            .limiter_permit
            .as_mut()
            .ok_or(SearchProviderError::RateLimit)?;
        for attempt in 0..8 {
            match permit.mark_dispatched() {
                Ok(()) => {
                    self.dispatched = true;
                    return Ok(());
                }
                Err(polaris_ratelimit::RateLimitError::Busy) if attempt < 7 => {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Err(_) => return Err(SearchProviderError::RateLimit),
            }
        }
        Err(SearchProviderError::RateLimit)
    }

    /// Reconciles a one-request, one-token estimate after response handling.
    pub(crate) fn finish(mut self) -> Result<(), SearchProviderError> {
        let permit = self
            .limiter_permit
            .take()
            .ok_or(SearchProviderError::RateLimit)?;
        if !self.dispatched {
            drop(permit);
            return Ok(());
        }
        permit
            .reconcile(1)
            .map(|_| ())
            .map_err(|_| SearchProviderError::RateLimit)
    }
}

fn map_rate_limit_error(error: RateLimitError) -> SearchProviderError {
    match error {
        RateLimitError::InvalidQuota
        | RateLimitError::ExceedsBurst
        | RateLimitError::Cancelled
        | RateLimitError::Accounting
        | RateLimitError::Undispatched
        | RateLimitError::Busy => SearchProviderError::RateLimit,
    }
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-search/crates/providers/src/runtime.rs"]
mod tests;
