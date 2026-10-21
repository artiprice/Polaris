//! Cost accounting in integer micro-USD.
//!
//! Telemetry does not price anything: every `ModelRequestCompleted` event already carries its
//! `cost` (computed where the provider response and the configured price table meet), and this
//! module only sums those amounts.

use polaris_sdk::ids::MicroUsd;
use serde::{Deserialize, Serialize};

/// Money spent in a session, exact to one micro-dollar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CostReport {
    /// Sum of the `cost` of all completed model requests (saturating, see the snapshot's
    /// `saturated` flag).
    pub total: MicroUsd,
    /// `total / requests`, rounded down; zero without requests.
    pub per_request_avg: MicroUsd,
}

impl CostReport {
    /// Builds the report from a total and the number of requests that produced it.
    #[must_use]
    pub fn from_totals(total: MicroUsd, requests: u64) -> Self {
        Self {
            total,
            per_request_avg: MicroUsd::new(total.get().checked_div(requests).unwrap_or(0)),
        }
    }
}
