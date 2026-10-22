//! Independent arithmetic oracle: widened integer model and exhaustive byte-sized inputs.
use polaris_p2_acceptance::bounded_sum;

#[test]
fn exhaustive_against_widened_oracle() -> Result<(), String> {
    for left in 0_u32..=255 {
        for right in 0_u32..=255 {
            for limit in [0_u32, 127, 255, 510, u32::MAX] {
                let wide = u64::from(left).checked_add(u64::from(right)).ok_or("oracle overflow")?;
                let expected = if wide <= u64::from(limit) { Some(u32::try_from(wide).map_err(|_| "oracle conversion")?) } else { None };
                if bounded_sum(&[left, right], limit) != expected {
                    return Err(format!("oracle disagreement: {left}, {right}, limit {limit}"));
                }
            }
        }
    }
    Ok(())
}

#[test]
fn word_boundary_against_widened_oracle() -> Result<(), &'static str> {
    for left in [0_u32, 1, u32::MAX.saturating_sub(1), u32::MAX] {
        for right in [0_u32, 1, 2, u32::MAX] {
            let wide = u64::from(left).checked_add(u64::from(right)).ok_or("oracle overflow")?;
            let expected = u32::try_from(wide).ok();
            if bounded_sum(&[left, right], u32::MAX) != expected { return Err("word-boundary oracle disagreement"); }
        }
    }
    Ok(())
}
