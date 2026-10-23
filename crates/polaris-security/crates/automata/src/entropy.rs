//! Shannon entropy in integer fixed point (no floats, no `as` casts) and high-entropy run
//! detection for secret candidates.

use crate::error::AutomataError;
use crate::scan::{Match, RuleId, ScanBuf, check_len};

#[cfg(feature = "kani")]
#[path = "kani_proofs.rs"]
mod kani_proofs;

/// Fractional bits of the fixed-point log2.
const FRAC_BITS: u32 = 32;

/// `floor(log2(x) * 2^32)` for `x >= 1` (0 for `x = 0`), by repeated squaring of the mantissa.
fn log2_q32(x: u64) -> u128 {
    if x == 0 {
        return 0;
    }
    let int = 63 - x.leading_zeros();
    // Mantissa in Q62: [2^62, 2^63).
    let mut y: u128 = if int <= 62 {
        u128::from(x) << (62 - int)
    } else {
        u128::from(x) >> (int - 62)
    };
    let mut frac: u128 = 0;
    for bit in (0..FRAC_BITS).rev() {
        y = (y * y) >> 62;
        if y >= 1 << 63 {
            y >>= 1;
            frac |= 1 << bit;
        }
    }
    (u128::from(int) << FRAC_BITS) | frac
}

/// Shannon entropy of `bytes` in millibits per byte (0..=8000), rounded; 0 for empty input.
/// Inputs of 4 GiB or more are measured over their first `u32::MAX` bytes.
#[must_use]
pub fn shannon_entropy_milli(bytes: &[u8]) -> u32 {
    let bytes = match usize::try_from(u32::MAX) {
        Ok(limit) => match bytes.get(..limit) {
            Some(prefix) => prefix,
            None => bytes,
        },
        Err(_) => bytes,
    };
    let mut counts = [0u64; 256];
    for b in bytes {
        if let Some(c) = counts.get_mut(usize::from(*b)) {
            *c += 1;
        }
    }
    let n = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if n == 0 {
        return 0;
    }
    // H = log2(n) - (1/n) * sum(c * log2(c)).
    let mut sum = 0_u128;
    for count in &counts {
        if *count > 0 {
            sum += u128::from(*count) * log2_q32(*count);
        }
    }
    let h_q32 = log2_q32(n).saturating_sub(sum / u128::from(n));
    let milli = (h_q32 * 1000 + (1 << (FRAC_BITS - 1))) >> FRAC_BITS;
    match u32::try_from(milli) {
        Ok(value) => value.min(8000),
        Err(_) => 8000,
    }
}

/// Parameters for [`high_entropy_spans`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntropyCfg {
    /// Minimum run length. Below 32 a run cannot reach 4.5 bits/byte with distinct symbols
    /// (`log2(len)` caps it), so shorter minimums only make sense with lower thresholds.
    pub min_len: usize,
    /// Minimum entropy in millibits per byte (hex digests and UUIDs stay at or below 4000).
    pub threshold_milli: u32,
}

impl Default for EntropyCfg {
    /// 32 bytes, 4.5 bits/byte: a GUESSING heuristic until calibrated against fixtures.
    fn default() -> Self {
        Self {
            min_len: 32,
            threshold_milli: 4500,
        }
    }
}

const fn is_run_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'_' | b'-')
}

/// Lazy iterator over candidate spans (allocation-free; used by the redactor).
pub(crate) struct EntropySpans<'h> {
    haystack: &'h [u8],
    pos: usize,
    cfg: EntropyCfg,
}

impl<'h> EntropySpans<'h> {
    pub(crate) const fn new(haystack: &'h [u8], cfg: EntropyCfg) -> Self {
        Self {
            haystack,
            pos: 0,
            cfg,
        }
    }
}

impl Iterator for EntropySpans<'_> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        loop {
            let rest = self.haystack.get(self.pos..)?;
            let start = self.pos + rest.iter().position(|b| is_run_byte(*b))?;
            let run = self.haystack.get(start..)?;
            let run_end = match run.iter().position(|b| !is_run_byte(*b)) {
                Some(offset) => offset,
                None => run.len(),
            };
            let mut end = start + run_end;
            // Base64 padding belongs to the run only at its end (at most two `=`).
            for _ in 0..2 {
                if self.haystack.get(end) == Some(&b'=') {
                    end += 1;
                }
            }
            self.pos = end;
            let span = self.haystack.get(start..end)?;
            if span.len() >= self.cfg.min_len
                && shannon_entropy_milli(span) >= self.cfg.threshold_milli
            {
                return Some((start, end));
            }
        }
    }
}

/// Writes runs of base64/hex-like bytes that are at least `cfg.min_len` long and at least
/// `cfg.threshold_milli` entropy into `out` (rule [`RuleId::ENTROPY`]); returns the count.
///
/// # Errors
/// [`AutomataError::BufferFull`] or [`AutomataError::TooLarge`] (haystack of 4 GiB or more).
pub fn high_entropy_spans(
    haystack: &[u8],
    cfg: &EntropyCfg,
    out: &mut ScanBuf,
) -> Result<usize, AutomataError> {
    check_len(haystack)?;
    out.clear();
    for (start, end) in EntropySpans::new(haystack, *cfg) {
        let (Ok(start), Ok(end)) = (u32::try_from(start), u32::try_from(end)) else {
            break;
        };
        out.push(Match {
            rule: RuleId::ENTROPY,
            start,
            end,
        })?;
    }
    Ok(out.len())
}
