//! Bounded latency statistics for one stage (S1, S2, tools, sandbox).

use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};

use crate::error::TelemetryError;
use crate::sat::sat_add;

/// Significant decimal figures kept by every latency histogram: relative error at most 0.1%.
/// Values below 2048 us are exact.
pub const SIGNIFICANT_FIGURES: u8 = 3;

/// Largest latency the histograms resolve: one hour in microseconds. Larger samples are clamped
/// to this value in the percentiles (never an error); `max_us` and `total_us` stay exact.
pub const MAX_TRACKABLE_US: u64 = 3_600_000_000;

/// Smallest trackable value (a zero sample is recorded in the lowest bucket).
const MIN_TRACKABLE_US: u64 = 1;

/// Latency summary of one stage, in microseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StageStats {
    /// Number of samples.
    pub count: u64,
    /// Sum of all samples (saturating).
    pub total_us: u64,
    /// Median (histogram-resolved, see [`SIGNIFICANT_FIGURES`]); 0 without samples.
    pub p50_us: u64,
    /// 99th percentile (histogram-resolved, clamped to [`MAX_TRACKABLE_US`]); 0 without samples.
    pub p99_us: u64,
    /// Largest sample, exact and unclamped.
    pub max_us: u64,
}

impl StageStats {
    /// `total_us / count` rounded down; 0 without samples.
    #[must_use]
    pub fn avg_us(&self) -> u64 {
        self.total_us.checked_div(self.count).unwrap_or(0)
    }
}

/// Histogram plus exact count, sum and maximum.
#[derive(Debug, Clone)]
pub(crate) struct Latency {
    hist: Histogram<u64>,
    count: u64,
    total_us: u64,
    max_us: u64,
}

fn histogram_error(error: impl std::fmt::Display) -> TelemetryError {
    TelemetryError::Histogram(error.to_string().into_boxed_str())
}

impl Latency {
    pub(crate) fn new() -> Result<Self, TelemetryError> {
        let hist =
            Histogram::new_with_bounds(MIN_TRACKABLE_US, MAX_TRACKABLE_US, SIGNIFICANT_FIGURES)
                .map_err(histogram_error)?;
        Ok(Self {
            hist,
            count: 0,
            total_us: 0,
            max_us: 0,
        })
    }

    /// Records one sample; out-of-range values are clamped in the histogram only.
    pub(crate) fn record(&mut self, latency_us: u64, saturated: &mut bool) {
        self.hist.saturating_record(latency_us);
        self.count = sat_add(self.count, 1, saturated);
        self.total_us = sat_add(self.total_us, latency_us, saturated);
        self.max_us = self.max_us.max(latency_us);
    }

    /// Adds every sample of `other`.
    pub(crate) fn merge(
        &mut self,
        other: &Self,
        saturated: &mut bool,
    ) -> Result<(), TelemetryError> {
        self.hist.add(&other.hist).map_err(histogram_error)?;
        self.count = sat_add(self.count, other.count, saturated);
        self.total_us = sat_add(self.total_us, other.total_us, saturated);
        self.max_us = self.max_us.max(other.max_us);
        Ok(())
    }

    pub(crate) fn stats(&self) -> StageStats {
        if self.count == 0 {
            return StageStats::default();
        }
        StageStats {
            count: self.count,
            total_us: self.total_us,
            p50_us: self.hist.value_at_quantile(0.5),
            p99_us: self.hist.value_at_quantile(0.99),
            max_us: self.max_us,
        }
    }
}
