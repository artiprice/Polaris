//! Exact binary calibration measurements over explicitly labelled, completely joined observations.
//!
//! Scores are positive-class probabilities quantized by the measurement producer to integer parts
//! per million. This module neither runs inference nor certifies fixture labels or provenance hashes.
//! Original engine responses and quantization policy must accompany a production calibration record.
use polaris_sdk::ids::Cid;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Maximum joined observations in one bounded calibration.
pub const MAX_CALIBRATION_SAMPLES: usize = 65_536;
const SCALE: u32 = 1_000_000;
const BIN_WIDTH: u32 = 100_000;

/// One independently labelled fixture, keyed by stable identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationLabel {
    /// Unique printable fixture identity, at most 256 bytes.
    pub id: String,
    /// Whether the positive class is correct for this fixture.
    pub positive: bool,
}
/// Actual measured score for one fixture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationPrediction {
    /// Identity present exactly once in the labelled fixture set.
    pub id: String,
    /// Positive-class probability in `0..=1_000_000`.
    pub positive_ppm: u32,
}
/// Exact, unreduced rational value; undefined metrics use `None`, never fabricated zeroes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ExactRatio {
    /// Nonnegative numerator.
    pub numerator: u64,
    /// Strictly positive denominator.
    pub denominator: u64,
}
/// Measured evidence for a threshold; this does not promote a schema to calibrated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CalibrationReport {
    /// Schema whose positive-class outputs were measured.
    pub schema: String,
    /// Exact schema version.
    pub version: u32,
    /// Caller-supplied hash of the immutable labelled fixture bytes.
    pub fixture: Cid,
    /// Caller-supplied hash of the actual inference checkpoint.
    pub checkpoint: Cid,
    /// Predicted positive exactly when score is at least this threshold.
    pub threshold_ppm: u32,
    /// Number of completely joined cases.
    pub samples: u64,
    /// Correctly predicted positives.
    pub true_positive: u64,
    /// Incorrectly predicted positives.
    pub false_positive: u64,
    /// Correctly predicted negatives.
    pub true_negative: u64,
    /// Incorrectly predicted negatives.
    pub false_negative: u64,
    /// TP / (TP + FP), undefined without predicted positives.
    pub precision: Option<ExactRatio>,
    /// TP / (TP + FN), undefined without actual positives.
    pub recall: Option<ExactRatio>,
    /// Mean squared positive-class probability error.
    pub brier: ExactRatio,
    /// Ten equal-width probability-bin expected calibration error; 1.0 is in the final bin.
    pub ece: ExactRatio,
}
/// Minimum measured precision and positive support used for threshold selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrecisionRequirement {
    /// Required precision in `0..=1_000_000`.
    pub minimum_ppm: u32,
    /// At least this many predicted positives; must be nonzero.
    pub minimum_predicted_positive: u32,
}
/// Immutable bounded join of truth labels and measured outputs.
#[derive(Debug, Clone)]
pub struct CalibrationDataset {
    schema: String,
    version: u32,
    fixture: Cid,
    checkpoint: Cid,
    observations: Vec<(u32, bool)>,
    brier: ExactRatio,
    ece: ExactRatio,
}
/// Fail-closed errors from malformed calibration input or arithmetic bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CalibrationError {
    /// Empty, oversized or malformed input or policy.
    #[error("invalid calibration input or bound")]
    Input,
    /// IDs were duplicated, missing or foreign to the labelled set.
    #[error("calibration fixture/output identity sets differ")]
    Identity,
    /// Checked metric arithmetic could not represent the result.
    #[error("calibration arithmetic overflow")]
    Arithmetic,
}
impl CalibrationDataset {
    /// Join exact identity sets before computing any measurement. Sorting makes input order irrelevant.
    /// # Errors
    /// Rejects invalid bounds, identity mismatches, out-of-range probabilities and arithmetic failure.
    pub fn join(
        schema: &str,
        version: u32,
        fixture: Cid,
        checkpoint: Cid,
        labels: &[CalibrationLabel],
        outputs: &[CalibrationPrediction],
    ) -> Result<Self, CalibrationError> {
        if !valid_id(schema, 128)
            || version == 0
            || labels.is_empty()
            || labels.len() > MAX_CALIBRATION_SAMPLES
            || outputs.len() != labels.len()
        {
            return Err(CalibrationError::Input);
        }
        let mut truth = BTreeMap::new();
        for label in labels {
            if !valid_id(&label.id, 256) {
                return Err(CalibrationError::Input);
            }
            if truth.insert(label.id.as_str(), label.positive).is_some() {
                return Err(CalibrationError::Identity);
            }
        }
        let mut observations = Vec::with_capacity(labels.len());
        let mut bins: BTreeMap<u32, (u64, u64, u64)> = BTreeMap::new();
        let mut squared = 0_u64;
        for output in outputs {
            if output.positive_ppm > SCALE {
                return Err(CalibrationError::Input);
            }
            let positive = truth
                .remove(output.id.as_str())
                .ok_or(CalibrationError::Identity)?;
            let score = u64::from(output.positive_ppm);
            let target = if positive { u64::from(SCALE) } else { 0 };
            let error = score.abs_diff(target);
            squared = add(
                squared,
                error
                    .checked_mul(error)
                    .ok_or(CalibrationError::Arithmetic)?,
            )?;
            let bin = output
                .positive_ppm
                .checked_div(BIN_WIDTH)
                .ok_or(CalibrationError::Arithmetic)?
                .min(9);
            let totals = bins.entry(bin).or_insert((0, 0, 0));
            totals.0 = add(totals.0, 1)?;
            totals.1 = add(totals.1, score)?;
            totals.2 = add(totals.2, target)?;
            observations.push((output.positive_ppm, positive));
        }
        if !truth.is_empty() {
            return Err(CalibrationError::Identity);
        }
        observations.sort_unstable();
        let samples =
            u64::try_from(observations.len()).map_err(|_| CalibrationError::Arithmetic)?;
        let denominator = samples
            .checked_mul(u64::from(SCALE))
            .ok_or(CalibrationError::Arithmetic)?;
        let brier_denominator = denominator
            .checked_mul(u64::from(SCALE))
            .ok_or(CalibrationError::Arithmetic)?;
        let mut error = 0_u64;
        for (_, (_, scores, targets)) in bins {
            error = add(error, scores.abs_diff(targets))?;
        }
        Ok(Self {
            schema: schema.to_owned(),
            version,
            fixture,
            checkpoint,
            observations,
            brier: ExactRatio {
                numerator: squared,
                denominator: brier_denominator,
            },
            ece: ExactRatio {
                numerator: error,
                denominator,
            },
        })
    }
    /// Compute exact confusion metrics for one inclusive positive threshold.
    /// # Errors
    /// Invalid threshold or checked arithmetic failure.
    pub fn evaluate(&self, threshold_ppm: u32) -> Result<CalibrationReport, CalibrationError> {
        if threshold_ppm > SCALE {
            return Err(CalibrationError::Input);
        }
        let mut tp = 0;
        let mut fp = 0;
        let mut tn = 0;
        let mut fn_count = 0;
        for (score, positive) in &self.observations {
            match (*score >= threshold_ppm, *positive) {
                (true, true) => tp = add(tp, 1)?,
                (true, false) => fp = add(fp, 1)?,
                (false, false) => tn = add(tn, 1)?,
                (false, true) => fn_count = add(fn_count, 1)?,
            }
        }
        self.report(threshold_ppm, tp, fp, tn, fn_count)
    }
    /// Select the lowest observed threshold meeting measured precision/support (maximum recall).
    /// Equal scores are processed atomically; `None` means the evidence cannot meet the policy.
    /// Runs in O(n) over the immutable sorted join, without a threshold-by-sample quadratic scan.
    /// # Errors
    /// Invalid precision/support policy or checked arithmetic failure.
    pub fn select_threshold(
        &self,
        requirement: PrecisionRequirement,
    ) -> Result<Option<CalibrationReport>, CalibrationError> {
        if requirement.minimum_ppm > SCALE || requirement.minimum_predicted_positive == 0 {
            return Err(CalibrationError::Input);
        }
        let mut tp = 0;
        let mut fp = 0;
        let mut best = None;
        let positives = u64::try_from(self.observations.iter().filter(|(_, yes)| *yes).count())
            .map_err(|_| CalibrationError::Arithmetic)?;
        let samples =
            u64::try_from(self.observations.len()).map_err(|_| CalibrationError::Arithmetic)?;
        let mut iter = self.observations.iter().rev().peekable();
        while let Some((score, positive)) = iter.next() {
            if *positive {
                tp = add(tp, 1)?;
            } else {
                fp = add(fp, 1)?;
            }
            if iter.peek().is_some_and(|(next, _)| next == score) {
                continue;
            }
            let predicted = add(tp, fp)?;
            let achieved = tp
                .checked_mul(u64::from(SCALE))
                .ok_or(CalibrationError::Arithmetic)?;
            let required = predicted
                .checked_mul(u64::from(requirement.minimum_ppm))
                .ok_or(CalibrationError::Arithmetic)?;
            if predicted >= u64::from(requirement.minimum_predicted_positive)
                && achieved >= required
            {
                best = Some((*score, tp, fp));
            }
        }
        match best {
            None => Ok(None),
            Some((threshold, tp, fp)) => {
                let fn_count = positives
                    .checked_sub(tp)
                    .ok_or(CalibrationError::Arithmetic)?;
                let tn = samples
                    .checked_sub(add(add(tp, fp)?, fn_count)?)
                    .ok_or(CalibrationError::Arithmetic)?;
                self.report(threshold, tp, fp, tn, fn_count).map(Some)
            }
        }
    }
    fn report(
        &self,
        threshold_ppm: u32,
        tp: u64,
        fp: u64,
        tn: u64,
        fn_count: u64,
    ) -> Result<CalibrationReport, CalibrationError> {
        Ok(CalibrationReport {
            schema: self.schema.clone(),
            version: self.version,
            fixture: self.fixture,
            checkpoint: self.checkpoint,
            threshold_ppm,
            samples: u64::try_from(self.observations.len())
                .map_err(|_| CalibrationError::Arithmetic)?,
            true_positive: tp,
            false_positive: fp,
            true_negative: tn,
            false_negative: fn_count,
            precision: ratio(tp, add(tp, fp)?),
            recall: ratio(tp, add(tp, fn_count)?),
            brier: self.brier,
            ece: self.ece,
        })
    }
}
fn valid_id(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.trim() == value
        && !value.chars().any(char::is_control)
}
fn add(a: u64, b: u64) -> Result<u64, CalibrationError> {
    a.checked_add(b).ok_or(CalibrationError::Arithmetic)
}
fn ratio(numerator: u64, denominator: u64) -> Option<ExactRatio> {
    if denominator == 0 {
        None
    } else {
        Some(ExactRatio {
            numerator,
            denominator,
        })
    }
}
