//! Kani proof harnesses, compiled only with the `kani` feature:
//! `cargo kani -p polaris-kernel-directives --features kani` (WSL2; per-PC install `wsl-kani`).
//!
//! Scope: a whole-`parse` harness exhausts CBMC memory on this hardware even for 3-byte inputs
//! (> 3.4 GB after 7 minutes; PLAN W24), so the proofs target the component the span invariant
//! rests on — the cursor. `parse` itself is covered by proptest and the `directives_grammar`
//! fuzz target.

use crate::cursor::Cursor;

/// For any valid UTF-8 input of up to 4 bytes, stepping the cursor always lands on a char boundary,
/// never passes the end, and reaches the end.
#[kani::proof]
#[kani::unwind(6)]
fn cursor_bump_stays_on_char_boundaries() {
    let bytes: [u8; 4] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= bytes.len());
    let Some(prefix) = bytes.get(..len) else {
        return;
    };
    if let Ok(text) = core::str::from_utf8(prefix) {
        let mut cur = Cursor::new(text, 0);
        while cur.bump().is_some() {
            assert!(cur.pos() <= text.len());
            assert!(text.is_char_boundary(cur.pos()));
        }
        assert!(cur.at_end());
        assert!(cur.peek().is_none());
    }
}

/// `take_while`, `skip_ws` and `reset` keep the cursor on char boundaries for any predicate outcome.
#[kani::proof]
#[kani::unwind(6)]
fn cursor_take_while_stays_on_char_boundaries() {
    let bytes: [u8; 4] = kani::any();
    let stop_at: u32 = kani::any();
    if let Ok(text) = core::str::from_utf8(&bytes) {
        let mut cur = Cursor::new(text, 0);
        let taken = cur.take_while(|c| u32::from(c) != stop_at);
        assert!(taken.len() == cur.pos());
        assert!(text.is_char_boundary(cur.pos()));
        let mid = cur.pos();
        cur.skip_ws();
        assert!(text.is_char_boundary(cur.pos()));
        cur.reset(mid);
        assert!(cur.pos() == mid);
    }
}
