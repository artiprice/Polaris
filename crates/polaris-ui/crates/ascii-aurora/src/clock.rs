//! Frame pacing: fixed simulation step, no catch-up, pause when hidden.

use core::time::Duration;

/// Simulation time returned once in reduced-motion mode (a pleasing frame).
pub const STATIC_T: f32 = 12.0;

/// Smallest allowed step (1 ms).
pub const MIN_STEP: Duration = Duration::from_millis(1);
/// Largest allowed step (1 s).
pub const MAX_STEP: Duration = Duration::from_secs(1);

/// Decides when a new aurora frame is due and at which simulation time.
#[derive(Debug, Clone)]
pub struct FrameClock {
    step: Duration,
    reduced_motion: bool,
    visible: bool,
    accumulated: Duration,
    sim: Duration,
    invalidated: bool,
}

impl FrameClock {
    /// New clock with the given step (clamped to 1 ms..=1 s).
    ///
    /// The first call to [`FrameClock::advance`] always yields a frame.
    #[must_use]
    pub fn new(step: Duration, reduced_motion: bool) -> Self {
        Self {
            step: step.clamp(MIN_STEP, MAX_STEP),
            reduced_motion,
            visible: true,
            accumulated: Duration::ZERO,
            sim: Duration::ZERO,
            invalidated: true,
        }
    }

    /// The effective (clamped) step.
    #[must_use]
    pub fn step(&self) -> Duration {
        self.step
    }

    /// Current simulation time in seconds.
    #[must_use]
    pub fn sim_seconds(&self) -> f32 {
        self.sim.as_secs_f32()
    }

    /// Show or hide the surface. Hidden clocks neither advance nor wake up;
    /// becoming visible again requests an immediate repaint.
    pub fn set_visible(&mut self, visible: bool) {
        if visible && !self.visible {
            self.invalidated = true;
            self.accumulated = Duration::ZERO;
        }
        self.visible = visible;
    }

    /// Force the next [`FrameClock::advance`] to return a frame (after a resize).
    pub fn invalidate(&mut self) {
        self.invalidated = true;
    }

    /// Feed the wall time since the previous call.
    ///
    /// Returns `Some(simulation_seconds)` when a repaint is due. At most one
    /// step is taken per call; any longer stall is dropped, not replayed.
    pub fn advance(&mut self, elapsed: Duration) -> Option<f32> {
        if !self.visible {
            return None;
        }
        if self.reduced_motion {
            if self.invalidated {
                self.invalidated = false;
                return Some(STATIC_T);
            }
            return None;
        }
        self.accumulated = self.accumulated.saturating_add(elapsed);
        if self.accumulated >= self.step {
            self.accumulated = self.accumulated.saturating_sub(self.step);
            if self.accumulated >= self.step {
                self.accumulated = Duration::ZERO;
            }
            self.sim = self.sim.saturating_add(self.step);
            self.invalidated = false;
            return Some(self.sim.as_secs_f32());
        }
        if self.invalidated {
            self.invalidated = false;
            return Some(self.sim.as_secs_f32());
        }
        None
    }
}
