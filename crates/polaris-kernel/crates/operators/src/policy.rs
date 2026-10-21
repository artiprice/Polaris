//! Timeout, fallback, retry and race adapters around owner-provided operators.
use polaris_sdk::{error::{ErrorKind, HasKind}, registry::FunctionalOperator};
use std::{error::Error, fmt, future::Future, time::Duration};

/// Deterministic fallback policy for a fail-open stage.
pub trait Fallback<In, Out>: Send + Sync {
    /// Construct the declared fallback from the unchanged input.
    fn fallback(&self, input: &In) -> Out;
}

/// Why a fail-open path was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureOpenReason {
    /// Inner operator returned an error.
    Error,
    /// Inner operator exceeded its deadline.
    Timeout,
}

/// Required observer for fail-open telemetry. A durable event store can implement this contract.
pub trait FailureOpenObserver: Send + Sync {
    /// Failure from recording the degradation.
    type Err: HasKind + Send;
    /// Persist or otherwise account for the degradation before fallback is returned.
    fn record(&self, stage: &'static str, reason: FailureOpenReason) -> impl Future<Output = Result<(), Self::Err>> + Send;
}

/// Error from an open wrapper when degradation telemetry could not be recorded.
#[derive(Debug)]
pub struct OpenOpError<E>(pub E);
impl<E: fmt::Display> fmt::Display for OpenOpError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "failed-open event recording failed: {}", self.0) }
}
impl<E: Error + 'static> Error for OpenOpError<E> { fn source(&self) -> Option<&(dyn Error + 'static)> { Some(&self.0) } }
impl<E: HasKind> HasKind for OpenOpError<E> { fn kind(&self) -> ErrorKind { self.0.kind() } }

/// Fail-open adapter requiring an explicit fallback and degradation observer.
#[derive(Debug)]
pub struct OpenOp<Op, Fb, Obs> { inner: Op, fallback: Fb, observer: Obs, timeout: Duration, stage: &'static str }
impl<Op, Fb, Obs> OpenOp<Op, Fb, Obs> {
    /// Create an open adapter with a finite timeout and mandatory observer.
    #[must_use]
    pub const fn new(inner: Op, fallback: Fb, observer: Obs, timeout: Duration, stage: &'static str) -> Self { Self { inner, fallback, observer, timeout, stage } }
}
impl<Op, Fb, Obs> FunctionalOperator for OpenOp<Op, Fb, Obs>
where
    Op: FunctionalOperator,
    Op::In: Clone,
    Fb: Fallback<Op::In, Op::Out>,
    Obs: FailureOpenObserver,
{
    type In = Op::In;
    type Out = Op::Out;
    type Err = OpenOpError<Obs::Err>;
    async fn apply(&self, input: Self::In) -> Result<Self::Out, Self::Err> {
        let fallback_input = input.clone();
        let reason = match tokio::time::timeout(self.timeout, self.inner.apply(input)).await {
            Ok(Ok(output)) => return Ok(output),
            Ok(Err(_stage_error)) => FailureOpenReason::Error,
            Err(_elapsed) => FailureOpenReason::Timeout,
        };
        self.observer.record(self.stage, reason).await.map_err(OpenOpError)?;
        Ok(self.fallback.fallback(&fallback_input))
    }
}

/// Fail-closed timeout adapter. Timeout and stage errors remain visible to the caller.
#[derive(Debug)]
pub struct ClosedOp<Op> { inner: Op, timeout: Duration }
impl<Op> ClosedOp<Op> {
    /// Wrap an operator in a deadline; timed-out futures are dropped immediately.
    #[must_use]
    pub const fn new(inner: Op, timeout: Duration) -> Self { Self { inner, timeout } }
}
impl<Op> FunctionalOperator for ClosedOp<Op>
where Op: FunctionalOperator {
    type In = Op::In;
    type Out = Op::Out;
    type Err = ClosedOpError<Op::Err>;
    async fn apply(&self, input: Self::In) -> Result<Self::Out, Self::Err> {
        match tokio::time::timeout(self.timeout, self.inner.apply(input)).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(ClosedOpError::Stage(error)),
            Err(_) => Err(ClosedOpError::Timeout),
        }
    }
}

/// Fail-closed wrapper error.
#[derive(Debug)]
pub enum ClosedOpError<E> {
    /// Inner operator failed.
    Stage(E),
    /// Deadline elapsed and the inner future was dropped.
    Timeout,
}
impl<E: fmt::Display> fmt::Display for ClosedOpError<E> { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { match self { Self::Stage(err) => write!(f, "operator failed: {err}"), Self::Timeout => f.write_str("operator deadline elapsed") } } }
impl<E: Error + 'static> Error for ClosedOpError<E> { fn source(&self) -> Option<&(dyn Error + 'static)> { match self { Self::Stage(err) => Some(err), Self::Timeout => None } } }
impl<E: HasKind> HasKind for ClosedOpError<E> { fn kind(&self) -> ErrorKind { match self { Self::Stage(err) => err.kind(), Self::Timeout => ErrorKind::Transient } } }

/// Retry a transiently failing operator a bounded number of times.
///
/// Only wrap idempotent or deduplicated operations: a transient error can occur after a remote
/// side effect committed, and retrying cannot roll that effect back.
#[derive(Debug)]
pub struct RetryOp<Op> { inner: Op, max_attempts: u32, delay: Duration }
impl<Op> RetryOp<Op> {
    /// Construct with attempts including the initial call; zero is rejected at execution as a
    /// permanent configuration error rather than silently issuing no operation.
    #[must_use]
    pub const fn new(inner: Op, max_attempts: u32, delay: Duration) -> Self { Self { inner, max_attempts, delay } }
}
impl<Op> FunctionalOperator for RetryOp<Op>
where Op: FunctionalOperator, Op::In: Clone {
    type In = Op::In;
    type Out = Op::Out;
    type Err = RetryOpError<Op::Err>;
    async fn apply(&self, input: Self::In) -> Result<Self::Out, Self::Err> {
        if self.max_attempts == 0 { return Err(RetryOpError::NoAttempts); }
        let mut attempt = 0_u32;
        loop {
            attempt = attempt.saturating_add(1);
            match self.inner.apply(input.clone()).await {
                Ok(output) => return Ok(output),
                Err(error) if error.kind() == ErrorKind::Transient && attempt < self.max_attempts => tokio::time::sleep(self.delay).await,
                Err(error) => return Err(RetryOpError::Stage { attempts: attempt, source: error }),
            }
        }
    }
}

/// Retry wrapper error.
#[derive(Debug)]
pub enum RetryOpError<E> {
    /// Invalid zero-attempt policy.
    NoAttempts,
    /// Final stage failure.
    Stage { attempts: u32, source: E },
}
impl<E: fmt::Display> fmt::Display for RetryOpError<E> { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { match self { Self::NoAttempts => f.write_str("retry policy has zero attempts"), Self::Stage { attempts, source } => write!(f, "operator failed after {attempts} attempts: {source}") } } }
impl<E: Error + 'static> Error for RetryOpError<E> { fn source(&self) -> Option<&(dyn Error + 'static)> { match self { Self::NoAttempts => None, Self::Stage { source, .. } => Some(source) } } }
impl<E: HasKind> HasKind for RetryOpError<E> { fn kind(&self) -> ErrorKind { match self { Self::NoAttempts => ErrorKind::Permanent, Self::Stage { source, .. } => source.kind() } } }

/// Race two implementations on cloned inputs; first success wins and the loser future is dropped.
///
/// Use only for read-only or duplicate-safe work. Dropping the losing future does not undo remote
/// side effects already issued by that operator.
#[derive(Debug)]
pub struct RaceOp<Fast, Deep> { fast: Fast, deep: Deep }
impl<Fast, Deep> RaceOp<Fast, Deep> {
    /// Build a race between two equivalent operator contracts.
    #[must_use]
    pub const fn new(fast: Fast, deep: Deep) -> Self { Self { fast, deep } }
}
impl<Fast, Deep> FunctionalOperator for RaceOp<Fast, Deep>
where Fast: FunctionalOperator, Deep: FunctionalOperator<In = Fast::In, Out = Fast::Out, Err = Fast::Err>, Fast::In: Clone {
    type In = Fast::In;
    type Out = Fast::Out;
    type Err = Fast::Err;
    async fn apply(&self, input: Self::In) -> Result<Self::Out, Self::Err> {
        let fast = self.fast.apply(input.clone());
        let deep = self.deep.apply(input);
        tokio::pin!(fast, deep);
        tokio::select! {
            fast_result = &mut fast => match fast_result { Ok(value) => Ok(value), Err(first_error) => match deep.await { Ok(value) => Ok(value), Err(_second_error) => Err(first_error) } },
            deep_result = &mut deep => match deep_result { Ok(value) => Ok(value), Err(first_error) => match fast.await { Ok(value) => Ok(value), Err(_second_error) => Err(first_error) } },
        }
    }
}


