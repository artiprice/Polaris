//! Small Kani proofs for the repository-map budget arithmetic.

use crate::repomap::{estimated_tokens, max_render_bytes};

/// For every u32 token budget and representable byte count that fits, the ceiling fits.
#[kani::proof]
#[kani::unwind(4)]
fn repo_map_budget_ceiling_respects_token_limit() {
    let token_budget: u32 = kani::any();
    let rendered_bytes: usize = kani::any();
    let byte_limit = max_render_bytes(token_budget);
    let rendered_bytes_u64 = match u64::try_from(rendered_bytes) {
        Ok(value) => value,
        Err(_) => return,
    };
    kani::assume(rendered_bytes_u64 <= byte_limit);
    let estimate = estimated_tokens(rendered_bytes);
    assert!(
        estimate.is_some(),
        "fitting byte counts must have a representable token estimate"
    );
    let estimate = match estimate {
        Some(value) => value,
        None => return,
    };
    assert!(estimate <= token_budget);
}
