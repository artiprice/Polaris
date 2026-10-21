//! The immutable view the UI renders, its lock-free reader handle and the HUD projection.

use std::sync::Arc;

use arc_swap::ArcSwap;
use polaris_sdk::contracts::Timestamp;
use polaris_sdk::ids::SessionId;
use serde::{Deserialize, Serialize};

use crate::aggregate::{CacheStats, ModelUsage, Outcomes, SandboxLoad, ToolStats, UsageTotals};
use crate::budget::{Budget, BudgetReport, BudgetStatus};
use crate::cost::CostReport;
use crate::latency::StageStats;

/// Everything `/usage`, `/stats` and the HUD show, frozen at `at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetrySnapshot {
    /// The session the numbers belong to.
    pub session: SessionId,
    /// When the snapshot was published (supplied by the owner; the crate has no clock).
    pub at: Timestamp,
    /// Events folded so far, of every kind.
    pub events_seen: u64,
    /// Session-wide request and token totals.
    pub usage: UsageTotals,
    /// Per-model usage of attributed requests, sorted by model id.
    pub models: Vec<ModelUsage>,
    /// System One (decision) latency.
    pub s1: StageStats,
    /// System Two (model request) latency.
    pub s2: StageStats,
    /// Tool calls.
    pub tools: ToolStats,
    /// Sandbox load.
    pub sandbox: SandboxLoad,
    /// Provider prompt-cache statistics.
    pub cache: CacheStats,
    /// Money spent, from the event costs.
    pub cost: CostReport,
    /// Verifier and supervision counters.
    pub outcomes: Outcomes,
    /// Prompt plus completion tokens billed in the 60 seconds ending at `at` (events recorded
    /// without a timestamp are not included).
    pub recent_tokens_60s: u64,
    /// `true` when any counter hit `u64::MAX`: the displayed totals are then lower bounds.
    pub saturated: bool,
}

impl TelemetrySnapshot {
    /// The token-weather HUD projection.
    #[must_use]
    pub fn hud(&self) -> HudView {
        HudView {
            cache_hit_pct: self.cache.hit_pct(),
            burn_tokens_per_min: u32::try_from(self.recent_tokens_60s).unwrap_or(u32::MAX),
            s1_avg_us: self.s1.avg_us(),
            s2_avg_us: self.s2.avg_us(),
        }
    }

    /// Both budget limits evaluated against this snapshot.
    #[must_use]
    pub fn budget_report(&self, budget: &Budget) -> BudgetReport {
        budget.check(
            self.cost.total,
            self.usage
                .prompt_tokens
                .saturating_add(self.usage.completion_tokens),
        )
    }

    /// The more severe of the two budget statuses.
    #[must_use]
    pub fn budget(&self, budget: &Budget) -> BudgetStatus {
        self.budget_report(budget).worst()
    }
}

/// Fields of the token-weather HUD that P1 events can feed. Context load and time to first
/// token are deferred: no event carries them yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HudView {
    /// Provider cache hit percentage; `None` until the provider reports cached tokens.
    pub cache_hit_pct: Option<u8>,
    /// Tokens billed in the last 60 seconds, saturated at `u32::MAX`.
    pub burn_tokens_per_min: u32,
    /// Mean System One decision latency in microseconds (0 without samples).
    pub s1_avg_us: u64,
    /// Mean model request latency in microseconds (0 without samples).
    pub s2_avg_us: u64,
}

/// Cloneable, thread-safe handle that reads the latest published snapshot without locking.
#[derive(Debug, Clone)]
pub struct SnapshotReader {
    cell: Arc<ArcSwap<TelemetrySnapshot>>,
}

impl SnapshotReader {
    pub(crate) fn new(cell: Arc<ArcSwap<TelemetrySnapshot>>) -> Self {
        Self { cell }
    }

    /// The most recently published snapshot.
    #[must_use]
    pub fn snapshot(&self) -> Arc<TelemetrySnapshot> {
        self.cell.load_full()
    }
}
