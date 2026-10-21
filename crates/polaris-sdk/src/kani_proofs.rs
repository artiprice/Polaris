//! Kani proof harnesses, compiled only with the `kani` feature:
//! `cargo kani -p polaris-sdk --features kani` (WSL2; Kani is a per-PC install, `wsl-kani`).

use crate::effort::Effort;
use crate::system_one::Probability;

/// `Effort::try_from` is total over `u8` and agrees with `as_u8`.
#[kani::proof]
fn effort_try_from_is_total() {
    let n: u8 = kani::any();
    match Effort::try_from(n) {
        Ok(level) => assert!(level.as_u8() == n),
        Err(_) => assert!(n == 0 || n > 5),
    }
}

/// `Probability::new` never yields a value outside `0.0..=1.0` (and never NaN).
#[kani::proof]
fn probability_new_stays_in_range() {
    let value: f32 = kani::any();
    if let Ok(p) = Probability::new(value) {
        let v = p.get();
        assert!(v >= 0.0 && v <= 1.0);
        assert!(!v.is_nan());
    }
}

/// All u16 schemas for all three new lifecycle kinds and a legacy event representative.
/// Payload values cannot influence version admission. This does not prove parser/JSON behavior.
#[kani::proof]
fn lifecycle_schema_admission_is_exact() {
    use crate::contracts::{Event, EventRecord, Timestamp};
    use crate::ids::{Cid, Seq, SessionId};
    let schema: u16 = kani::any();
    let left: bool = kani::any();
    let right: bool = kani::any();
    let event = match (left, right) {
        (false, false) => Event::SessionClosed,
        (false, true) => Event::SessionConfigured {
            configuration: Cid::from_bytes([0; 32]),
        },
        (true, false) => Event::SessionPaused,
        (true, true) => Event::SessionResumed,
    };
    let result = EventRecord::from_versioned(
        schema,
        Seq::ZERO,
        Timestamp::UNIX_EPOCH,
        SessionId::from_u128(1),
        event,
    );
    assert_eq!(
        result.is_ok(),
        schema == 2 || (schema == 1 && !left && !right),
        "exact lifecycle schema admission"
    );
}
