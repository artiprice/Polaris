//! Sliding restart budget, evaluated using an injected clock.
use std::collections::VecDeque;
use std::time::Duration;
use tokio::time::Instant;

/// Maximum accepted restarts in one sliding window (the initial run is free).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartIntensity {
    /// Allowed restarts; zero prohibits restarting.
    pub max_restarts: u32,
    /// Window width, with the upper boundary exclusive.
    pub window: Duration,
}
impl Default for RestartIntensity {
    fn default() -> Self {
        Self {
            max_restarts: 3,
            window: Duration::from_secs(10),
        }
    }
}
/// Whether another restart consumes a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartVerdict {
    /// Restart accepted.
    Allowed {
        /// Accepted restarts still inside the window.
        restarts_in_window: u32,
    },
    /// Budget exhausted; a refused restart never extends the window.
    Exceeded,
}
/// Accepted restart timestamps only, bounded by the configured budget.
#[derive(Debug)]
pub struct RestartWindow {
    intensity: RestartIntensity,
    accepted: VecDeque<Instant>,
}
impl RestartWindow {
    /// Empty budget window.
    #[must_use]
    pub fn new(intensity: RestartIntensity) -> Self {
        Self {
            intensity,
            accepted: VecDeque::new(),
        }
    }
    /// Request a restart at `now`. Backward time retains existing budget consumption.
    pub fn record(&mut self, now: Instant) -> RestartVerdict {
        while self
            .accepted
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= self.intensity.window)
        {
            let _expired = self.accepted.pop_front();
        }
        let count = match u32::try_from(self.accepted.len()) {
            Ok(count) => count,
            Err(_) => return RestartVerdict::Exceeded,
        };
        if count >= self.intensity.max_restarts {
            return RestartVerdict::Exceeded;
        }
        self.accepted.push_back(now);
        RestartVerdict::Allowed {
            restarts_in_window: count.saturating_add(1),
        }
    }
}
