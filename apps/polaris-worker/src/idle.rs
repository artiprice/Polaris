//! Monotonic idle deadlines for resources owned by the native inference thread.

use std::time::{Duration, Instant};

/// Validated idle interval for the worker's single native S1 model.
///
/// Idle begins after completed native work. Cancellation before work starts does not extend it.
/// Destruction runs on the native owner thread and never preempts an in-progress inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct S1IdlePolicy {
    idle_ttl: Duration,
}

/// An idle interval is outside the supported one-millisecond to one-day range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("S1 idle interval must be between one millisecond and one day")]
pub struct IdlePolicyError;

impl S1IdlePolicy {
    /// Validates a finite idle interval.
    ///
    /// # Errors
    /// Returns [`IdlePolicyError`] below one millisecond or above one day.
    pub fn new(idle_ttl: Duration) -> Result<Self, IdlePolicyError> {
        if idle_ttl < Duration::from_millis(1) || idle_ttl > Duration::from_secs(86_400) {
            return Err(IdlePolicyError);
        }
        Ok(Self { idle_ttl })
    }

    /// Time without completed native work before the session is dropped.
    #[must_use]
    pub const fn idle_ttl(self) -> Duration {
        self.idle_ttl
    }
}

impl Default for S1IdlePolicy {
    fn default() -> Self {
        Self {
            idle_ttl: Duration::from_secs(45),
        }
    }
}

/// The resource and its deadline remain on one native owner thread.
pub(super) struct IdleSession<M> {
    model: Option<M>,
    deadline: Option<Instant>,
}

impl<M> IdleSession<M> {
    pub(super) const fn new() -> Self {
        Self {
            model: None,
            deadline: None,
        }
    }

    pub(super) const fn is_loaded(&self) -> bool {
        self.model.is_some()
    }

    pub(super) fn get_or_load(
        &mut self,
        load: impl FnOnce() -> Result<M, String>,
    ) -> Result<&M, String> {
        if self.model.is_none() {
            self.model = Some(load()?);
        }
        self.model
            .as_ref()
            .ok_or_else(|| "model initialization failed".to_owned())
    }

    pub(super) const fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub(super) fn completed_work(
        &mut self,
        now: Instant,
        policy: S1IdlePolicy,
    ) -> Result<(), std::io::Error> {
        self.deadline = if self.model.is_some() {
            Some(now.checked_add(policy.idle_ttl()).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "S1 idle deadline overflow",
                )
            })?)
        } else {
            None
        };
        Ok(())
    }

    pub(super) fn expire(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.evict()
        } else {
            false
        }
    }

    pub(super) fn evict(&mut self) -> bool {
        self.deadline = None;
        let model = self.model.take();
        let evicted = model.is_some();
        // Drop before returning an acknowledgement to the async RPC handler.
        drop(model);
        evicted
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use super::{IdleSession, S1IdlePolicy};

    struct ObservedDrop(Arc<AtomicBool>);

    impl Drop for ObservedDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[test]
    fn eviction_acknowledges_completed_destruction_and_is_idempotent()
    -> Result<(), Box<dyn std::error::Error>> {
        let dropped = Arc::new(AtomicBool::new(false));
        let mut session = IdleSession::new();
        let _model = session.get_or_load(|| Ok(ObservedDrop(Arc::clone(&dropped))))?;
        assert!(session.evict(), "a loaded model must be reported");
        assert!(
            dropped.load(Ordering::Acquire),
            "drop must precede acknowledgement"
        );
        assert!(!session.evict(), "a second eviction must report no model");
        assert!(
            session.deadline().is_none(),
            "no deadline for a cold session"
        );
        Ok(())
    }

    #[test]
    fn only_completed_work_moves_the_idle_deadline() -> Result<(), Box<dyn std::error::Error>> {
        let now = Instant::now();
        let policy = S1IdlePolicy::new(Duration::from_millis(10))?;
        let mut session = IdleSession::new();
        let _model = session.get_or_load(|| Ok(()))?;
        session.completed_work(now, policy)?;
        let deadline = session.deadline().ok_or("missing loaded deadline")?;
        let just_before = deadline
            .checked_sub(Duration::from_nanos(1))
            .ok_or("clock")?;
        assert!(
            !session.expire(just_before),
            "must not evict before deadline"
        );
        assert_eq!(
            session.deadline(),
            Some(deadline),
            "observation is not activity"
        );
        assert!(session.expire(deadline), "deadline is inclusive");
        assert!(!session.expire(deadline), "expired session is already cold");
        let _reloaded = session.get_or_load(|| Ok(()))?;
        session.completed_work(deadline, policy)?;
        assert!(session.deadline().is_some_and(|next| next > deadline));
        Ok(())
    }

    #[test]
    fn failed_loading_keeps_the_session_cold() -> Result<(), Box<dyn std::error::Error>> {
        let mut session = IdleSession::<()>::new();
        assert!(
            session
                .get_or_load(|| Err("fixture load failure".to_owned()))
                .is_err()
        );
        session.completed_work(Instant::now(), S1IdlePolicy::default())?;
        assert!(session.deadline().is_none());
        assert!(!session.evict());
        Ok(())
    }
}
