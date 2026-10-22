//! Kani harnesses for fixed-point entropy bounds.

use super::{FRAC_BITS, log2_q32, shannon_entropy_milli};

#[kani::proof]
fn fixed_point_log2_is_bounded_for_every_u64() {
    let value: u64 = kani::any();
    let result = log2_q32(value);
    assert!(result <= (64_u128 << FRAC_BITS));
}

#[kani::proof]
fn bounded_entropy_input_never_exceeds_eight_bits_per_byte() {
    let bytes: [u8; 32] = kani::any();
    let result = shannon_entropy_milli(&bytes);
    assert!(result <= 8000);
}
