//! Pure circuit-breaker state machine and a bus-backed handle.
use crate::ChildName;
use crate::task::{Inner, lock};
use polaris_sdk::contracts::Event;
use polaris_sdk::registry::EventStore;
use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

/// Circuit-breaker policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakerConfig {
    /// Failures needed to open.
    pub max_failures: NonZeroU32,
    /// Sliding failure window.
    pub window: Duration,
    /// Delay before one probe can run.
    pub cooldown: Duration,
}
/// Public admission state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerState {
    /// Normal admission.
    Closed,
    /// Calls refused until cooldown expires.
    Open {
        /// Earliest probe time.
        until: Instant,
    },
    /// One probe is outstanding.
    HalfOpen,
}
/// Failure history and one outstanding probe, with no wall-clock reads.
#[derive(Debug)]
pub struct CircuitBreaker {
    config: BreakerConfig,
    state: BreakerState,
    failures: VecDeque<Instant>,
    probe_since: Option<Instant>,
    deadline_overflow: bool,
}
impl CircuitBreaker {
    /// Construct a closed breaker.
    #[must_use]
    pub fn new(max_failures: NonZeroU32, window: Duration, cooldown: Duration) -> Self {
        Self {
            config: BreakerConfig {
                max_failures,
                window,
                cooldown,
            },
            state: BreakerState::Closed,
            failures: VecDeque::new(),
            probe_since: None,
            deadline_overflow: false,
        }
    }
    /// Current admission state.
    #[must_use]
    pub const fn state(&self) -> BreakerState {
        self.state
    }
    /// Admit normal calls, or one probe after cooldown. An unanswered probe can be replaced
    /// after another cooldown so missing results cannot wedge admission forever.
    pub fn allow(&mut self, now: Instant) -> bool {
        if self.deadline_overflow {
            return false;
        }
        let probe = match self.state {
            BreakerState::Closed => return true,
            BreakerState::Open { until } => now >= until,
            BreakerState::HalfOpen => self
                .probe_since
                .is_some_and(|at| now.saturating_duration_since(at) >= self.config.cooldown),
        };
        if probe {
            self.state = BreakerState::HalfOpen;
            self.probe_since = Some(now);
        }
        probe
    }
    /// A successful probe closes and clears failure history; late results while open are inert.
    pub fn on_success(&mut self, _now: Instant) {
        if self.state == BreakerState::HalfOpen {
            self.state = BreakerState::Closed;
            self.failures.clear();
            self.probe_since = None;
        }
    }
    /// Record failure and return the new state. Repeated late failures never extend an open state.
    pub fn on_failure(&mut self, now: Instant) -> BreakerState {
        let trip = match self.state {
            BreakerState::Open { .. } => return self.state,
            BreakerState::HalfOpen => true,
            BreakerState::Closed => {
                while self
                    .failures
                    .front()
                    .is_some_and(|at| now.saturating_duration_since(*at) >= self.config.window)
                {
                    let _old = self.failures.pop_front();
                }
                self.failures.push_back(now);
                match u32::try_from(self.failures.len()) {
                    Ok(count) => count >= self.config.max_failures.get(),
                    Err(_) => true,
                }
            }
        };
        if trip {
            // An unrepresentable cooldown fails closed rather than panicking or admitting early.
            let until = match now.checked_add(self.config.cooldown) {
                Some(until) => until,
                None => {
                    self.deadline_overflow = true;
                    now
                }
            };
            self.state = BreakerState::Open { until };
            self.failures.clear();
            self.probe_since = None;
        }
        self.state
    }
}

/// Shared breaker that announces transitions into Open through the durable bus.
pub struct BreakerHandle<S: EventStore + 'static> {
    pub(crate) name: ChildName,
    pub(crate) state: Arc<Mutex<CircuitBreaker>>,
    pub(crate) inner: Arc<Inner<S>>,
}
impl<S: EventStore + 'static> core::fmt::Debug for BreakerHandle<S> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BreakerHandle")
            .field("name", &self.name)
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}
impl<S: EventStore + 'static> BreakerHandle<S> {
    /// Admission using the Tokio clock.
    pub fn allow(&self) -> bool {
        lock(&self.state).allow(Instant::now())
    }
    /// Current state.
    #[must_use]
    pub fn state(&self) -> BreakerState {
        lock(&self.state).state()
    }
    /// Record a successful probe.
    pub fn on_success(&self) {
        lock(&self.state).on_success(Instant::now());
    }
    /// Record failure and announce only a new trip. Store failures are counted in supervisor status.
    pub async fn on_failure(&self) -> BreakerState {
        let (before, after) = {
            let mut state = lock(&self.state);
            let before = state.state();
            (before, state.on_failure(Instant::now()))
        };
        if !matches!(before, BreakerState::Open { .. })
            && matches!(after, BreakerState::Open { .. })
        {
            self.inner
                .emit(Event::CircuitTripped {
                    breaker: self.name.cid(),
                })
                .await;
        }
        after
    }
}
