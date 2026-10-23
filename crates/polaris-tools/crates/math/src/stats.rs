//! Bounded finite statistics; percentiles use Hyndman-Fan type 7 linear ranks.
use crate::{Finite, MathError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use statrs::statistics::Statistics;

/// Summary values for a non-empty sample (sample standard deviation, `n-1`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Summary {
    /// Number of observations.
    pub n: u64,
    /// Arithmetic mean.
    pub mean: Finite,
    /// Sample standard deviation; zero for a singleton.
    pub std_dev: Finite,
    /// Minimum.
    pub min: Finite,
    /// Maximum.
    pub max: Finite,
    /// Median, type-7 percentile.
    pub p50: Finite,
    /// 95th percentile, type-7.
    pub p95: Finite,
    /// 99th percentile, type-7.
    pub p99: Finite,
}

/// Computes bounded descriptive statistics for 1–1,000,000 finite observations.
pub fn summarize(xs: &[Finite]) -> Result<Summary, MathError> {
    if xs.is_empty() {
        return Err(MathError::EmptyInput);
    }
    if xs.len() > 1_000_000 {
        return Err(MathError::TooLong);
    }
    let values: Vec<f64> = xs.iter().map(|value| value.get()).collect();
    let mean = values.as_slice().mean();
    let std_dev = if values.len() == 1 {
        0.0
    } else {
        values.as_slice().std_dev()
    };
    let min = values.as_slice().min();
    let max = values.as_slice().max();
    let mut ordered = values;
    ordered.sort_by(f64::total_cmp);
    let p50 = percentile(&ordered, 50)?;
    let p95 = percentile(&ordered, 95)?;
    let p99 = percentile(&ordered, 99)?;
    Ok(Summary {
        n: u64::try_from(xs.len()).map_err(|_| MathError::TooLong)?,
        mean: Finite::new(mean)?,
        std_dev: Finite::new(std_dev)?,
        min: Finite::new(min)?,
        max: Finite::new(max)?,
        p50: Finite::new(p50)?,
        p95: Finite::new(p95)?,
        p99: Finite::new(p99)?,
    })
}

fn percentile(sorted: &[f64], percentile: u64) -> Result<f64, MathError> {
    let last = sorted.len().checked_sub(1).ok_or(MathError::EmptyInput)?;
    let scaled = u64::try_from(last)
        .map_err(|_| MathError::TooLong)?
        .checked_mul(percentile)
        .ok_or(MathError::Overflow)?;
    let lower_u64 = scaled / 100;
    let lower = usize::try_from(lower_u64).map_err(|_| MathError::TooLong)?;
    let upper = lower.saturating_add(1).min(last);
    let lower_value = sorted.get(lower).copied().ok_or(MathError::EmptyInput)?;
    let upper_value = sorted.get(upper).copied().ok_or(MathError::EmptyInput)?;
    let fraction = f64::from(u32::try_from(scaled % 100).map_err(|_| MathError::Overflow)?) / 100.0;
    let value = lower_value + (upper_value - lower_value) * fraction;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(MathError::NonFinite)
    }
}
