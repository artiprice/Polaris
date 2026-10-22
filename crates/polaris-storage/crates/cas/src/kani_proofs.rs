//! Bounded verification of the allocation-free short reference formatter.
use crate::{FOLD_REF_LEN, fold_ref};
use polaris_sdk::ids::Cid;

/// For every 256-bit identifier, the formatter fills precisely its 18-byte destination.
#[kani::proof]
#[kani::unwind(40)]
fn fold_ref_fills_exact_destination() {
    let cid = Cid::from_bytes(kani::any());
    let mut out = [0xff; FOLD_REF_LEN];
    assert_eq!(fold_ref(&cid, &mut out), FOLD_REF_LEN);
    assert_eq!(out.get(..6), Some("§ref:".as_bytes()));
    for byte in out.iter().skip(6) {
        assert!(byte.is_ascii_hexdigit());
    }
}
