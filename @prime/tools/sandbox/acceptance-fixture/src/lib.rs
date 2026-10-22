//! Real dependency-free test subject for the P2 sandbox acceptance path.
//! This fixture validates execution plumbing; it is not the Polaris repository test suite.

/// Sum without overflow, refusing every intermediate value above the supplied limit.
pub fn bounded_sum(values: &[u32], limit: u32) -> Option<u32> {
    values.iter().try_fold(0_u32, |sum, value| {
        let next = sum.checked_add(*value)?;
        if next <= limit { Some(next) } else { None }
    })
}

#[cfg(test)]
mod tests {
    use super::bounded_sum;

    #[test]
    fn ordinary_sum() -> Result<(), &'static str> {
        if bounded_sum(&[2, 3, 5], 10) != Some(10) { return Err("ordinary sum differs"); }
        Ok(())
    }
    #[test]
    fn limit_is_enforced() -> Result<(), &'static str> {
        if bounded_sum(&[2, 3, 5], 9).is_some() { return Err("limit bypass"); }
        Ok(())
    }
    #[test]
    fn overflow_is_refused() -> Result<(), &'static str> {
        if bounded_sum(&[u32::MAX, 1], u32::MAX).is_some() { return Err("overflow accepted"); }
        Ok(())
    }
}
