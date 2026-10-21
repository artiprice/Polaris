//! # polaris-telemetry
//!
//! Layer L1. Folds sdk bus events into per-session usage, cost, latency and cache statistics and
//! publishes immutable snapshots for `/usage`, `/stats` and the token-weather HUD. It consumes
//! `polaris_sdk::contracts::Event` values handed to it; it does not depend on the bus crate and
//! has no clock and no I/O.
//!
//! | Module | Contents |
//! |---|---|
//! | [`Telemetry`] | single-writer owner: `record`, `record_record`, `record_for_model`, `publish`, `budget` |
//! | [`Aggregates`] | pure, mergeable fold state (`record`, `merge`, `snapshot`) |
//! | [`TelemetrySnapshot`], [`HudView`], [`SnapshotReader`] | what the UI renders; lock-free reads via `arc-swap` |
//! | [`CostReport`] | exact integer micro-USD totals |
//! | [`Budget`], [`BudgetStatus`] | warn-only `$budget` evaluation |
//!
//! ## Decisions
//!
//! - **Money is integer micro-USD, summed from the event `cost`.** There is no price table here:
//!   the producer of `ModelRequestCompleted` already priced the request, and a second table would
//!   be a second source of truth. No floats touch money; percentages use exact 128-bit integers.
//! - **Overflow saturates, it is not an error.** Every counter, sum and histogram count pins at
//!   `u64::MAX` and sets `TelemetrySnapshot::saturated` (totals are then lower bounds). A
//!   display-only metric must never fail the fold of a bus event.
//! - **Latency histograms** (`hdrhistogram`) resolve 1 us to 1 hour with 3 significant figures
//!   (relative error at most 0.1%, exact below 2048 us). Larger samples are clamped in the
//!   percentiles only; `max_us` and `total_us` stay exact.
//! - **Events carry no model id.** Totals work from events alone; the per-model table is filled
//!   only through `Telemetry::record_for_model`, where the caller supplies the model.
//! - **Cache.** `cached_tokens` is a plain count, so "not reported" is inferred as "no request
//!   has had `cached_tokens > 0` yet" (provider behaviour is spike S0-5, unverified).
//! - **Tokens for budgets and burn rate** are prompt plus completion; cached tokens are a subset
//!   of the prompt and are never added again.
//!
//! ## Deferred
//!
//! Not in P1 because no event feeds them yet: price-table loading and `priced=false` reporting,
//! time to first token, context-load percentage, per-tier (L1/L2/L3) cache rates, the periodic
//! publish timer (the owner task calls `publish`), and `polaris --dump` wiring.

mod aggregate;
mod budget;
mod cost;
mod error;
mod latency;
mod rate;
mod sat;
mod snapshot;
mod telemetry;

pub use aggregate::{
    Aggregates, CacheStats, ModelUsage, Outcomes, SandboxLoad, ToolStats, UsageTotals,
};
pub use budget::{Budget, BudgetReport, BudgetStatus};
pub use cost::CostReport;
pub use error::TelemetryError;
pub use latency::{MAX_TRACKABLE_US, SIGNIFICANT_FIGURES, StageStats};
pub use snapshot::{HudView, SnapshotReader, TelemetrySnapshot};
pub use telemetry::Telemetry;
