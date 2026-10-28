//! Dual RPM/TPM admission adapter that carries its reservation through model work.
use polaris_ratelimit::{
    Clock, Denied, DenyReason, DualLimiter, Estimate, Permit, Priority, RateLimitError,
    Reconciliation,
};
use polaris_sdk::{
    error::{ErrorKind, HasKind},
    registry::FunctionalOperator,
};
use std::{fmt, time::Duration};
use tokio_util::sync::CancellationToken;

/// Owned work and the trusted token estimate used for admission.
///
/// This value is intentionally not `Clone`: cloning an admission request could make a generic
/// retry wrapper acquire multiple reservations for one upstream operation.
///
/// A generic retry wrapper cannot execute this admission stage because its input is not cloneable:
///
/// ```compile_fail
/// use polaris_kernel_operators::{RateLimitInput, RateLimitOp, RetryOp};
/// use polaris_ratelimit::MonotonicClock;
/// use polaris_sdk::registry::FunctionalOperator;
/// use std::time::Duration;
///
/// async fn retry_admission(
///     operator: RateLimitOp<MonotonicClock, String>,
///     input: RateLimitInput<String>,
/// ) {
///     let retry = RetryOp::new(operator, 3, Duration::ZERO);
///     let _ = retry.apply(input).await;
/// }
/// ```
#[derive(Debug)]
pub struct RateLimitInput<T> {
    payload: T,
    estimate: Estimate,
    priority: Priority,
    cancellation: CancellationToken,
}

impl<T> RateLimitInput<T> {
    /// Construct an admission request using a caller's estimate, priority, and cancellation scope.
    #[must_use]
    pub const fn new(
        payload: T,
        estimate: Estimate,
        priority: Priority,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            payload,
            estimate,
            priority,
            cancellation,
        }
    }

    /// Borrow the owned work before admission completes.
    #[must_use]
    pub const fn payload(&self) -> &T {
        &self.payload
    }
}

/// Work paired with its actual limiter permit.
///
/// Keep this value alive through the upstream request. Call [`Self::mark_dispatched`] immediately
/// before transport handoff, then consume it with [`Self::reconcile`] after final usage arrives.
/// Dropping it never refunds a GCRA reservation because request delivery may be uncertain.
pub struct RateLimited<T, C: Clock> {
    payload: T,
    permit: Permit<C>,
}

impl<T: fmt::Debug, C: Clock> fmt::Debug for RateLimited<T, C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RateLimited")
            .field("payload", &self.payload)
            .field("reserved_tokens", &self.permit.reserved_tokens())
            .finish_non_exhaustive()
    }
}

impl<T, C: Clock> RateLimited<T, C> {
    /// Borrow the admitted payload while retaining its reservation.
    #[must_use]
    pub const fn payload(&self) -> &T {
        &self.payload
    }

    /// Mutably borrow the admitted payload while retaining its reservation.
    #[must_use]
    pub fn payload_mut(&mut self) -> &mut T {
        &mut self.payload
    }

    /// Reserved token count carried into upstream work.
    #[must_use]
    pub const fn reserved_tokens(&self) -> u64 {
        self.permit.reserved_tokens()
    }

    /// Mark the point immediately before handing the request to the transport.
    ///
    /// # Errors
    /// Returns an accounting error if dispatch was already marked or the limiter ledger failed.
    pub fn mark_dispatched(&mut self) -> Result<(), RateLimitError> {
        self.permit.mark_dispatched()
    }

    /// Consume the reservation after final provider usage is known.
    ///
    /// # Errors
    /// Returns [`RateLimitError::Undispatched`] unless [`Self::mark_dispatched`] was called first,
    /// or an accounting error if usage reconciliation fails.
    pub fn reconcile(self, actual_tokens: u64) -> Result<(T, Reconciliation), RateLimitError> {
        let Self { payload, permit } = self;
        permit
            .reconcile(actual_tokens)
            .map(|reconciliation| (payload, reconciliation))
    }
}

/// Admission failure. Timeout and cancellation do not produce a permit or a successful value.
#[derive(Debug)]
pub enum RateLimitOpError {
    /// Local limiter rejected the estimate or encountered an accounting error.
    Admission(RateLimitError),
    /// Priority was explicitly shed by the configured local pressure policy.
    Shed(Denied),
    /// Waiting for quota exceeded the configured deadline.
    Timeout,
    /// The caller canceled while admission was pending.
    Cancelled,
}

impl fmt::Display for RateLimitOpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Admission(error) => write!(formatter, "rate-limit admission failed: {error}"),
            Self::Shed(_) => formatter.write_str("rate-limit priority was shed"),
            Self::Timeout => formatter.write_str("rate-limit admission timed out"),
            Self::Cancelled => formatter.write_str("rate-limit admission was cancelled"),
        }
    }
}

impl std::error::Error for RateLimitOpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Admission(error) => Some(error),
            Self::Shed(_) | Self::Timeout | Self::Cancelled => None,
        }
    }
}

impl HasKind for RateLimitOpError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Admission(error) => error.kind(),
            Self::Shed(_) | Self::Timeout | Self::Cancelled => ErrorKind::Policy,
        }
    }
}

/// Configured fail-closed admission operator over the real shared limiter.
pub struct RateLimitOp<C: Clock, T> {
    limiter: DualLimiter<C>,
    timeout: Duration,
    payload: std::marker::PhantomData<fn() -> T>,
}

impl<C: Clock, T> fmt::Debug for RateLimitOp<C, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RateLimitOp")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl<C: Clock, T> RateLimitOp<C, T> {
    /// Construct with an actual limiter and finite positive admission deadline.
    ///
    /// # Errors
    /// Returns [`RateLimitOpConfigError`] for a zero admission deadline.
    pub fn new(limiter: DualLimiter<C>, timeout: Duration) -> Result<Self, RateLimitOpConfigError> {
        if timeout.is_zero() {
            return Err(RateLimitOpConfigError);
        }
        Ok(Self {
            limiter,
            timeout,
            payload: std::marker::PhantomData,
        })
    }
}

impl<C, T> FunctionalOperator for RateLimitOp<C, T>
where
    C: Clock,
    T: Send,
{
    type In = RateLimitInput<T>;
    type Out = RateLimited<T, C>;
    type Err = RateLimitOpError;

    async fn apply(&self, input: Self::In) -> Result<Self::Out, Self::Err> {
        let RateLimitInput {
            payload,
            estimate,
            priority,
            cancellation,
        } = input;
        let admission = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(RateLimitOpError::Cancelled),
            result = tokio::time::timeout(self.timeout, async {
                loop {
                    match self.limiter.try_acquire(estimate, priority) {
                        Ok(permit) => return Ok(permit),
                        Err(denied) if denied.reason == DenyReason::Accounting => {
                            return Err(RateLimitOpError::Admission(RateLimitError::Accounting));
                        }
                        Err(denied) if denied.reason == DenyReason::ExceedsBurst => {
                            return Err(RateLimitOpError::Admission(RateLimitError::ExceedsBurst));
                        }
                        Err(denied @ Denied { reason: DenyReason::Shed(_), .. }) => {
                            return Err(RateLimitOpError::Shed(denied));
                        }
                        Err(denied) => {
                            tokio::time::sleep(denied.retry_after.max(Duration::from_millis(1))).await;
                        }
                    }
                }
            }) => match result {
                Ok(Ok(permit)) => permit,
                Ok(Err(error)) => return Err(error),
                Err(_) => return Err(RateLimitOpError::Timeout),
            },
        };
        Ok(RateLimited {
            payload,
            permit: admission,
        })
    }
}

/// Invalid rate-limit operator construction policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("rate-limit admission timeout must be positive")]
pub struct RateLimitOpConfigError;
