//! Per-session aggregates folded from bus events.
//!
//! [`Aggregates`] is plain data with no clock and no I/O: `record` folds one event, `merge` adds
//! another aggregate (the `/stats` view across sessions), `snapshot` renders the immutable view.
//! Every counter saturates at `u64::MAX` and raises a sticky `saturated` flag instead of wrapping
//! or panicking.

use std::collections::BTreeMap;

use polaris_sdk::contracts::{Event, Timestamp};
use polaris_sdk::ids::{MicroUsd, ModelId, SessionId};
use serde::{Deserialize, Serialize};

use crate::cost::CostReport;
use crate::error::TelemetryError;
use crate::latency::{Latency, StageStats};
use crate::sat::{percent_floor, sat_add};
use crate::snapshot::TelemetrySnapshot;

/// Session-wide model usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UsageTotals {
    /// Completed model requests.
    pub requests: u64,
    /// Prompt tokens billed.
    pub prompt_tokens: u64,
    /// Completion tokens billed.
    pub completion_tokens: u64,
}

/// Provider prompt-cache statistics (the only cache tier the P1 events expose).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CacheStats {
    /// `true` once any request carried `cached_tokens > 0`. The event carries a plain count, so
    /// "provider does not report" and "reported zero" are indistinguishable until the first
    /// non-zero value (spike S0-5 will confirm which providers report at all).
    pub reported: bool,
    /// Prompt tokens of all requests.
    pub prompt_tokens: u64,
    /// Prompt tokens served from the provider cache.
    pub cached_tokens: u64,
}

impl CacheStats {
    /// `floor(100 * cached / prompt)` capped at 100, or `None` while nothing was reported.
    #[must_use]
    pub fn hit_pct(&self) -> Option<u8> {
        if !self.reported {
            return None;
        }
        let pct = percent_floor(self.cached_tokens, self.prompt_tokens).unwrap_or(100);
        Some(u8::try_from(pct.min(100)).unwrap_or(100))
    }
}

/// Tool-call latency and failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ToolStats {
    /// Latency of completed tool calls.
    pub stage: StageStats,
    /// Completed calls with `ok == false`.
    pub failed: u64,
}

/// Sandbox load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SandboxLoad {
    /// Latency of completed sandboxed commands.
    pub stage: StageStats,
    /// Commands with a non-zero exit code.
    pub failed: u64,
    /// Snapshot branches created.
    pub snapshot_branches: u64,
}

/// Counters of verifier and supervision events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Outcomes {
    /// Diffs the verifier approved.
    pub diffs_approved: u64,
    /// Diffs the verifier rejected.
    pub diffs_rejected: u64,
    /// Proof-of-Done checks that passed.
    pub proof_passed: u64,
    /// Proof-of-Done checks that blocked a completion claim.
    pub proof_blocked: u64,
    /// Worker restarts (one per `WorkerRestarted` event).
    pub worker_restarts: u64,
    /// Circuit breakers that opened.
    pub circuits_tripped: u64,
    /// Messages sent to the dead-letter queue.
    pub dead_letters: u64,
}

/// Usage of one model, for requests that were attributed to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUsage {
    /// The model.
    pub model: ModelId,
    /// Completed requests attributed to the model.
    pub requests: u64,
    /// Prompt tokens billed.
    pub prompt_tokens: u64,
    /// Completion tokens billed.
    pub completion_tokens: u64,
    /// Cached prompt tokens; `None` until a request of this model reported a non-zero count.
    pub cached_tokens: Option<u64>,
    /// Cost of the attributed requests.
    pub cost: MicroUsd,
}

/// Accumulator behind [`ModelUsage`].
#[derive(Debug, Clone, Default)]
struct ModelTotals {
    requests: u64,
    prompt_tokens: u64,
    completion_tokens: u64,
    cached_tokens: u64,
    cached_reported: bool,
    cost: u64,
}

impl ModelTotals {
    fn add(&mut self, other: &Self, saturated: &mut bool) {
        self.requests = sat_add(self.requests, other.requests, saturated);
        self.prompt_tokens = sat_add(self.prompt_tokens, other.prompt_tokens, saturated);
        self.completion_tokens =
            sat_add(self.completion_tokens, other.completion_tokens, saturated);
        self.cached_tokens = sat_add(self.cached_tokens, other.cached_tokens, saturated);
        self.cached_reported |= other.cached_reported;
        self.cost = sat_add(self.cost, other.cost, saturated);
    }
}

/// The folded state of one session (or of several merged sessions).
#[derive(Debug, Clone)]
pub struct Aggregates {
    events_seen: u64,
    usage: UsageTotals,
    cache: CacheStats,
    cost: u64,
    models: BTreeMap<ModelId, ModelTotals>,
    s1: Latency,
    s2: Latency,
    tools: Latency,
    tools_failed: u64,
    sandbox: Latency,
    sandbox_failed: u64,
    snapshot_branches: u64,
    outcomes: Outcomes,
    saturated: bool,
}

/// One billed request, as folded into the totals.
#[derive(Debug, Clone, Copy)]
struct RequestFacts {
    prompt: u64,
    completion: u64,
    cached: u64,
    cost: u64,
    latency_us: u64,
}

impl Aggregates {
    /// Empty aggregates.
    ///
    /// # Errors
    /// [`TelemetryError::Histogram`] if a latency histogram cannot be allocated.
    pub fn new() -> Result<Self, TelemetryError> {
        Ok(Self {
            events_seen: 0,
            usage: UsageTotals::default(),
            cache: CacheStats::default(),
            cost: 0,
            models: BTreeMap::new(),
            s1: Latency::new()?,
            s2: Latency::new()?,
            tools: Latency::new()?,
            tools_failed: 0,
            sandbox: Latency::new()?,
            sandbox_failed: 0,
            snapshot_branches: 0,
            outcomes: Outcomes::default(),
            saturated: false,
        })
    }

    /// Folds one event into the aggregates.
    ///
    /// The `match` is exhaustive on purpose: a new `Event` variant fails to compile here until
    /// telemetry decides what it means. Events carry no model id, so `model` is supplied by the
    /// caller (it knows the request record); it only matters for `ModelRequestCompleted`, where
    /// `Some` additionally fills the per-model table. Totals are always updated.
    pub fn record(&mut self, model: Option<&ModelId>, event: &Event) {
        self.events_seen = sat_add(self.events_seen, 1, &mut self.saturated);
        match *event {
            Event::ModelRequestCompleted {
                prompt_tokens,
                completion_tokens,
                cached_tokens,
                cost,
                latency_us,
                ..
            } => self.fold_request(
                model,
                RequestFacts {
                    prompt: u64::from(prompt_tokens.get()),
                    completion: u64::from(completion_tokens.get()),
                    cached: u64::from(cached_tokens.get()),
                    cost: cost.get(),
                    latency_us,
                },
            ),
            Event::DecisionMade { latency_us, .. } => {
                self.s1.record(latency_us, &mut self.saturated);
            }
            Event::ToolExecutionCompleted { ok, latency_us, .. } => {
                self.tools.record(latency_us, &mut self.saturated);
                if !ok {
                    self.tools_failed = sat_add(self.tools_failed, 1, &mut self.saturated);
                }
            }
            Event::SandboxExecutionCompleted {
                exit_code,
                latency_us,
                ..
            } => {
                self.sandbox.record(latency_us, &mut self.saturated);
                if exit_code != 0 {
                    self.sandbox_failed = sat_add(self.sandbox_failed, 1, &mut self.saturated);
                }
            }
            Event::SandboxSnapshotBranchCreated { .. } => {
                self.snapshot_branches = sat_add(self.snapshot_branches, 1, &mut self.saturated);
            }
            Event::DiffApproved { .. } => {
                self.outcomes.diffs_approved =
                    sat_add(self.outcomes.diffs_approved, 1, &mut self.saturated);
            }
            Event::DiffRejected { .. } => {
                self.outcomes.diffs_rejected =
                    sat_add(self.outcomes.diffs_rejected, 1, &mut self.saturated);
            }
            Event::ProofOfDonePassed { .. } => {
                self.outcomes.proof_passed =
                    sat_add(self.outcomes.proof_passed, 1, &mut self.saturated);
            }
            Event::ProofOfDoneBlocked { .. } => {
                self.outcomes.proof_blocked =
                    sat_add(self.outcomes.proof_blocked, 1, &mut self.saturated);
            }
            Event::WorkerRestarted { .. } => {
                self.outcomes.worker_restarts =
                    sat_add(self.outcomes.worker_restarts, 1, &mut self.saturated);
            }
            Event::CircuitTripped { .. } => {
                self.outcomes.circuits_tripped =
                    sat_add(self.outcomes.circuits_tripped, 1, &mut self.saturated);
            }
            Event::DeadLettered { .. } => {
                self.outcomes.dead_letters =
                    sat_add(self.outcomes.dead_letters, 1, &mut self.saturated);
            }
            // Counted in `events_seen` only: lifecycle, directive, lease and tool-start events
            // carry no usage, cost or latency.
            Event::SessionStarted { .. }
            | Event::DirectiveApplied { .. }
            | Event::ToolExecutionStarted { .. }
            | Event::FileLeaseGranted { .. }
            | Event::FileLeaseRevoked { .. }
            | Event::SessionClosed
            | Event::SessionConfigured { .. }
            | Event::SessionPaused
            | Event::SessionResumed
            | Event::BlastRadiusAlert { .. }
            | Event::DiffNeedsApproval { .. }
            | Event::FrameStepRecorded { .. } => {}
        }
    }

    fn fold_request(&mut self, model: Option<&ModelId>, facts: RequestFacts) {
        let saturated = &mut self.saturated;
        self.usage.requests = sat_add(self.usage.requests, 1, saturated);
        self.usage.prompt_tokens = sat_add(self.usage.prompt_tokens, facts.prompt, saturated);
        self.usage.completion_tokens =
            sat_add(self.usage.completion_tokens, facts.completion, saturated);
        self.cache.prompt_tokens = sat_add(self.cache.prompt_tokens, facts.prompt, saturated);
        self.cache.cached_tokens = sat_add(self.cache.cached_tokens, facts.cached, saturated);
        if facts.cached > 0 {
            self.cache.reported = true;
        }
        self.cost = sat_add(self.cost, facts.cost, saturated);
        self.s2.record(facts.latency_us, saturated);
        if let Some(model) = model {
            let delta = ModelTotals {
                requests: 1,
                prompt_tokens: facts.prompt,
                completion_tokens: facts.completion,
                cached_tokens: facts.cached,
                cached_reported: facts.cached > 0,
                cost: facts.cost,
            };
            self.models
                .entry(model.clone())
                .or_default()
                .add(&delta, saturated);
        }
    }

    /// Adds everything `other` has seen: counters (saturating), the per-model table and the
    /// latency histograms. Folding two slices separately and merging gives the same snapshot
    /// as folding their concatenation.
    ///
    /// # Errors
    /// [`TelemetryError::Histogram`] if two histograms cannot be combined (cannot happen for
    /// aggregates built by [`Aggregates::new`]; reported rather than hidden). On error `self`
    /// may be partially updated.
    pub fn merge(&mut self, other: &Self) -> Result<(), TelemetryError> {
        let saturated = &mut self.saturated;
        *saturated |= other.saturated;
        self.events_seen = sat_add(self.events_seen, other.events_seen, saturated);
        self.usage.requests = sat_add(self.usage.requests, other.usage.requests, saturated);
        self.usage.prompt_tokens = sat_add(
            self.usage.prompt_tokens,
            other.usage.prompt_tokens,
            saturated,
        );
        self.usage.completion_tokens = sat_add(
            self.usage.completion_tokens,
            other.usage.completion_tokens,
            saturated,
        );
        self.cache.prompt_tokens = sat_add(
            self.cache.prompt_tokens,
            other.cache.prompt_tokens,
            saturated,
        );
        self.cache.cached_tokens = sat_add(
            self.cache.cached_tokens,
            other.cache.cached_tokens,
            saturated,
        );
        self.cache.reported |= other.cache.reported;
        self.cost = sat_add(self.cost, other.cost, saturated);
        for (model, totals) in &other.models {
            self.models
                .entry(model.clone())
                .or_default()
                .add(totals, saturated);
        }
        self.s1.merge(&other.s1, saturated)?;
        self.s2.merge(&other.s2, saturated)?;
        self.tools.merge(&other.tools, saturated)?;
        self.tools_failed = sat_add(self.tools_failed, other.tools_failed, saturated);
        self.sandbox.merge(&other.sandbox, saturated)?;
        self.sandbox_failed = sat_add(self.sandbox_failed, other.sandbox_failed, saturated);
        self.snapshot_branches =
            sat_add(self.snapshot_branches, other.snapshot_branches, saturated);
        let (mine, theirs) = (&mut self.outcomes, &other.outcomes);
        mine.diffs_approved = sat_add(mine.diffs_approved, theirs.diffs_approved, saturated);
        mine.diffs_rejected = sat_add(mine.diffs_rejected, theirs.diffs_rejected, saturated);
        mine.proof_passed = sat_add(mine.proof_passed, theirs.proof_passed, saturated);
        mine.proof_blocked = sat_add(mine.proof_blocked, theirs.proof_blocked, saturated);
        mine.worker_restarts = sat_add(mine.worker_restarts, theirs.worker_restarts, saturated);
        mine.circuits_tripped = sat_add(mine.circuits_tripped, theirs.circuits_tripped, saturated);
        mine.dead_letters = sat_add(mine.dead_letters, theirs.dead_letters, saturated);
        Ok(())
    }

    /// Billed tokens so far: prompt plus completion (cached tokens are a subset of the prompt
    /// and are not added again). Saturating.
    #[must_use]
    pub fn tokens_used(&self) -> u64 {
        self.usage
            .prompt_tokens
            .saturating_add(self.usage.completion_tokens)
    }

    /// Sum of the event costs so far (saturating).
    #[must_use]
    pub fn cost_total(&self) -> MicroUsd {
        MicroUsd::new(self.cost)
    }

    /// The immutable view of these aggregates. `recent_tokens_60s` is 0 here; the owning
    /// [`crate::Telemetry`] fills it from its rate window.
    #[must_use]
    pub fn snapshot(&self, session: SessionId, at: Timestamp) -> TelemetrySnapshot {
        let models = self
            .models
            .iter()
            .map(|(model, totals)| ModelUsage {
                model: model.clone(),
                requests: totals.requests,
                prompt_tokens: totals.prompt_tokens,
                completion_tokens: totals.completion_tokens,
                cached_tokens: totals.cached_reported.then_some(totals.cached_tokens),
                cost: MicroUsd::new(totals.cost),
            })
            .collect();
        TelemetrySnapshot {
            session,
            at,
            events_seen: self.events_seen,
            usage: self.usage,
            models,
            s1: self.s1.stats(),
            s2: self.s2.stats(),
            tools: ToolStats {
                stage: self.tools.stats(),
                failed: self.tools_failed,
            },
            sandbox: SandboxLoad {
                stage: self.sandbox.stats(),
                failed: self.sandbox_failed,
                snapshot_branches: self.snapshot_branches,
            },
            cache: self.cache,
            cost: CostReport::from_totals(self.cost_total(), self.usage.requests),
            outcomes: self.outcomes,
            recent_tokens_60s: 0,
            saturated: self.saturated,
        }
    }
}
