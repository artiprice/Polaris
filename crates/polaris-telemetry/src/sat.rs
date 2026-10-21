//! Saturating counter helpers shared by the aggregates.

/// `a + b`, pinned at `u64::MAX`; sets `flag` when the true sum did not fit.
pub(crate) fn sat_add(a: u64, b: u64, flag: &mut bool) -> u64 {
    match a.checked_add(b) {
        Some(sum) => sum,
        None => {
            *flag = true;
            u64::MAX
        }
    }
}

/// `floor(100 * part / whole)`, or `None` when `whole` is zero. Exact (128-bit), never overflows.
pub(crate) fn percent_floor(part: u64, whole: u64) -> Option<u128> {
    u128::from(part)
        .saturating_mul(100)
        .checked_div(u128::from(whole))
}
