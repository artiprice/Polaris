//! Numeric conversions: the single module in this crate allowed to use `as`.
//!
//! Every helper clamps (or range-checks) its input first, so the casts below
//! can neither truncate nor wrap in a way that matters.

#![allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "this module is the one audited home of float/int conversions; inputs are clamped first"
)]

/// Replace NaN by `fallback`, otherwise return `x`.
pub(crate) fn finite_or(x: f32, fallback: f32) -> f32 {
    if x.is_nan() { fallback } else { x }
}

/// Map a unit-range float to a byte with rounding (NaN becomes 0).
///
/// Adding 2^23 pushes the rounded integer into the low mantissa bits, so the
/// conversion is a handful of vectorisable float ops with no float-to-int
/// instruction and no cast.
pub(crate) fn unit_to_u8(x: f32) -> u8 {
    const MAGIC: f32 = 8_388_608.0;
    #[allow(
        clippy::manual_clamp,
        reason = "max/min drop NaN (mapping it to 0); clamp would keep NaN"
    )]
    let clamped = x.max(0.0).min(1.0);
    let [low, ..] = (clamped * 255.0 + MAGIC).to_bits().to_le_bytes();
    low
}

/// Exact for the magnitudes used here (cell and pixel counts below 2^24).
pub(crate) fn u32_to_f32(v: u32) -> f32 {
    v as f32
}

/// Exact for the magnitudes used here (cell and pixel counts below 2^24).
pub(crate) fn usize_to_f32(v: usize) -> f32 {
    v as f32
}

/// `floor(x)` clamped into `0..=max` (NaN becomes 0).
pub(crate) fn floor_to_u32(x: f32, max: u32) -> u32 {
    let clamped = finite_or(x, 0.0).floor().clamp(0.0, u32_to_f32(max));
    (clamped as u32).min(max)
}

/// `floor(x)` clamped into `0..=max` (NaN becomes 0).
pub(crate) fn floor_to_usize(x: f32, max: usize) -> usize {
    let clamped = finite_or(x, 0.0).floor().clamp(0.0, usize_to_f32(max));
    (clamped as usize).min(max)
}

/// `floor(x)` clamped into a generous symmetric range (NaN becomes 0).
pub(crate) fn floor_to_i32(x: f32) -> i32 {
    const LIMIT: f32 = 1_000_000.0;
    finite_or(x, 0.0).floor().clamp(-LIMIT, LIMIT) as i32
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-ui/crates/ascii-aurora/src/conv.rs"]
mod tests;
