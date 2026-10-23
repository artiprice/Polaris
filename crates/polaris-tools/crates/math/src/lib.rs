//! Deterministic arithmetic, interpolation, statistics, and CUDA launch calculators.

mod bandwidth;
mod calc;
mod error;
mod finite;
mod gpu;
mod interp;
mod stats;
mod tool;

pub use bandwidth::{GbPerSec, effective_bandwidth, peak_bandwidth};
pub use calc::{CalcLimits, calc, calc_f64};
pub use error::MathError;
pub use finite::Finite;
pub use gpu::{ComputeCapability, GpuSpecTable, LaunchConfig, Limiter, Occupancy, occupancy};
pub use interp::{Grid2, Interp, Knot, Point2, bezier, bilinear, interpolate, lerp};
pub use stats::{Summary, summarize};
pub use tool::{MathInput, MathOperation, MathOutput, MathTool, limiter_name};
