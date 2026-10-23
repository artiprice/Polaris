//! Typed, pure SDK tool dispatch for deterministic math operations.
use crate::{
    CalcLimits, ComputeCapability, Finite, GbPerSec, GpuSpecTable, Grid2, Interp, Knot,
    LaunchConfig, Limiter, MathError, Occupancy, Point2, Summary, bezier, bilinear, calc, calc_f64,
    effective_bandwidth, interpolate, occupancy, peak_bandwidth, summarize,
};
use polaris_sdk::registry::{Effects, Tool, ToolContext, ToolError, ToolSpec};
use rust_decimal::Decimal;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Tagged math operation input. Decimal values are JSON strings to avoid lossy JSON numbers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum MathInput {
    /// Fixed-scale exact decimal expression.
    Calc { expression: String },
    /// Floating-point expression including whitelisted transcendental functions.
    CalcF64 { expression: String },
    /// Knot interpolation.
    Interpolate {
        knots: Vec<Knot>,
        method: Interp,
        x: Finite,
    },
    /// Bilinear rectangle interpolation.
    Bilinear { grid: Grid2, x: Finite, y: Finite },
    /// De Casteljau Bezier evaluation.
    Bezier { control: Vec<Point2>, t: Finite },
    /// Descriptive statistics.
    Summarize { values: Vec<Finite> },
    /// CUDA occupancy estimate from caller-supplied sourced table TOML.
    Occupancy {
        capability: ComputeCapability,
        launch: LaunchConfig,
    },
    /// Effective bandwidth from byte count and elapsed time (decimal strings).
    EffectiveBandwidth { bytes: u64, elapsed_seconds: String },
    /// Theoretical peak bandwidth from GT/s and bus width (decimal strings).
    PeakBandwidth {
        data_rate_gtps: String,
        bus_bits: u32,
    },
}

/// Structured output; decimal values are strings to preserve exact textual representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "result", content = "value", rename_all = "snake_case")]
pub enum MathOutput {
    /// Exact decimal.
    Decimal(String),
    /// Finite floating-point scalar.
    Scalar(Finite),
    /// Finite point.
    Point(Point2),
    /// Statistical summary.
    Summary(Summary),
    /// Occupancy estimate.
    Occupancy(Occupancy),
    /// Decimal GB/s.
    Bandwidth(String),
}

/// SDK tool exposing the deterministic math operation set.
#[derive(Debug, Default, Clone, Copy)]
pub struct MathTool;
impl MathTool {
    /// Returns the model-facing SDK descriptor for the combined math dispatcher.
    #[must_use]
    pub fn tool_spec(&self) -> ToolSpec {
        <Self as Tool>::spec(self)
    }
}
impl Tool for MathTool {
    const NAME: &'static str = "math";
    const EFFECTS: Effects = Effects::PURE;
    type Input = MathInput;
    type Output = MathOutput;

    fn spec(&self) -> ToolSpec {
        let schema = schema_for!(MathInput);
        let schema_json =
            serde_json::to_string(&schema).unwrap_or_else(|_| r#"{"not":{}}"#.to_owned());
        ToolSpec { name: Self::NAME.into(), description: "Deterministic math: fixed-scale decimal arithmetic (28–29 significant digits, not arbitrary precision; rational fractions such as 1/3 are rounded to this scale), finite interpolation/statistics, and first-order GPU occupancy from a bundled curated NVIDIA table with per-row citations. Operations have bounded inputs and return typed validation errors; occupancy excludes compiler allocation granularity and spills.".into(), input_schema_json: schema_json.into(), effects: Effects::PURE }
    }

    async fn call(
        &self,
        input: Self::Input,
        _ctx: &ToolContext<'_>,
    ) -> Result<Self::Output, ToolError> {
        dispatch(input).map_err(|error| ToolError::InvalidInput(error.to_string()))
    }
}

fn dispatch(input: MathInput) -> Result<MathOutput, MathError> {
    let limits = CalcLimits::default();
    match input {
        MathInput::Calc { expression } => {
            Ok(MathOutput::Decimal(calc(&expression, &limits)?.to_string()))
        }
        MathInput::CalcF64 { expression } => Ok(MathOutput::Scalar(Finite::new(calc_f64(
            &expression,
            &limits,
        )?)?)),
        MathInput::Interpolate { knots, method, x } => {
            Ok(MathOutput::Scalar(interpolate(&knots, method, x)?))
        }
        MathInput::Bilinear { grid, x, y } => Ok(MathOutput::Scalar(bilinear(&grid, x, y)?)),
        MathInput::Bezier { control, t } => Ok(MathOutput::Point(bezier(&control, t)?)),
        MathInput::Summarize { values } => Ok(MathOutput::Summary(summarize(&values)?)),
        MathInput::Occupancy { capability, launch } => {
            let table = GpuSpecTable::nvidia_reference()?;
            Ok(MathOutput::Occupancy(occupancy(
                &table, capability, &launch,
            )?))
        }
        MathInput::EffectiveBandwidth {
            bytes,
            elapsed_seconds,
        } => {
            let elapsed =
                Decimal::from_str(&elapsed_seconds).map_err(|_| MathError::Parse { at: 0 })?;
            let rate: GbPerSec = effective_bandwidth(bytes, elapsed)?;
            Ok(MathOutput::Bandwidth(rate.value().to_string()))
        }
        MathInput::PeakBandwidth {
            data_rate_gtps,
            bus_bits,
        } => {
            let rate =
                Decimal::from_str(&data_rate_gtps).map_err(|_| MathError::Parse { at: 0 })?;
            Ok(MathOutput::Bandwidth(
                peak_bandwidth(rate, bus_bits)?.value().to_string(),
            ))
        }
    }
}

/// Public calculation operation names advertised by descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathOperation {
    Calc,
    Interpolate,
    Bilinear,
    Summarize,
    Occupancy,
    EffectiveBandwidth,
    PeakBandwidth,
}
impl MathOperation {
    /// Stable descriptor name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Calc => "calc",
            Self::Interpolate => "interpolate",
            Self::Bilinear => "bilinear",
            Self::Summarize => "summarize",
            Self::Occupancy => "occupancy",
            Self::EffectiveBandwidth => "effective_bandwidth",
            Self::PeakBandwidth => "peak_bandwidth",
        }
    }
}

/// Typed set of possible limiting factors for occupancy outputs.
#[must_use]
pub const fn limiter_name(limiter: Limiter) -> &'static str {
    match limiter {
        Limiter::Threads => "threads",
        Limiter::Registers => "registers",
        Limiter::SharedMemory => "shared_memory",
        Limiter::BlockSlots => "block_slots",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use polaris_sdk::registry::Tool;

    #[test]
    fn tool_spec_is_pure_and_has_a_real_operation_schema() {
        let spec = MathTool.spec();
        let schema: Result<serde_json::Value, _> = serde_json::from_str(&spec.input_schema_json);
        assert_eq!(spec.name.as_ref(), "math");
        assert_eq!(spec.effects, Effects::PURE);
        assert!(
            schema
                .as_ref()
                .is_ok_and(|value| value.get("oneOf").is_some() || value.get("$defs").is_some())
        );
        let snapshot_value = serde_json::json!({
            "name": spec.name,
            "description": spec.description,
            "effects": spec.effects,
            "input_schema": schema.unwrap_or(serde_json::Value::Null),
        });
        insta::assert_json_snapshot!("math_tool_spec", snapshot_value);
    }

    #[test]
    fn typed_dispatch_preserves_decimal_output() {
        let result = dispatch(MathInput::Calc {
            expression: "0.1+0.2".to_owned(),
        });
        assert_eq!(result, Ok(MathOutput::Decimal("0.3".to_owned())));
    }

    #[test]
    fn invalid_decimal_string_returns_typed_error() {
        let result = dispatch(MathInput::EffectiveBandwidth {
            bytes: 10,
            elapsed_seconds: "zero".to_owned(),
        });
        assert!(result.is_err());
    }
}
