//! Decimal bandwidth calculations with explicit GB/s units.
use crate::MathError;
use rust_decimal::Decimal;

/// Decimal gigabytes per second (1 GB = 1,000,000,000 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GbPerSec(Decimal);
impl GbPerSec {
    /// Returns the decimal GB/s value.
    #[must_use]
    pub const fn value(self) -> Decimal {
        self.0
    }
}

/// Computes decimal GB/s from bytes moved and positive elapsed seconds.
pub fn effective_bandwidth(bytes: u64, elapsed_s: Decimal) -> Result<GbPerSec, MathError> {
    if elapsed_s <= Decimal::ZERO {
        return Err(MathError::InvalidDuration);
    }
    let bytes = Decimal::from(bytes);
    let seconds = bytes.checked_div(elapsed_s).ok_or(MathError::Overflow)?;
    Ok(GbPerSec(
        seconds
            .checked_div(Decimal::from(1_000_000_000_u64))
            .ok_or(MathError::Overflow)?,
    ))
}

/// Computes theoretical decimal GB/s from GT/s and a bus width in bits.
pub fn peak_bandwidth(data_rate_gtps: Decimal, bus_bits: u32) -> Result<GbPerSec, MathError> {
    if data_rate_gtps <= Decimal::ZERO || bus_bits == 0 || !bus_bits.is_multiple_of(8) {
        return Err(MathError::InvalidLaunch(
            "data rate must be positive and bus width a positive multiple of eight",
        ));
    }
    let bytes_per_second = data_rate_gtps
        .checked_mul(Decimal::from(bus_bits))
        .ok_or(MathError::Overflow)?;
    Ok(GbPerSec(
        bytes_per_second
            .checked_div(Decimal::from(8_u32))
            .ok_or(MathError::Overflow)?,
    ))
}
