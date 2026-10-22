//! Pure admission and sequence arithmetic; channels themselves are checked by Tokio/Loom tests.
use crate::BusConfig;
use std::num::NonZeroUsize;
#[kani::proof]
fn channel_capacity_admission_is_exact() {
    let commands: usize = kani::any();
    let events: usize = kani::any();
    if let (Some(command_capacity), Some(event_capacity)) =
        (NonZeroUsize::new(commands), NonZeroUsize::new(events))
    {
        let config = BusConfig {
            command_capacity,
            event_capacity,
        };
        assert_eq!(
            config.validate().is_ok(),
            commands <= (usize::MAX >> 4) && events <= (usize::MAX >> 4)
        );
    }
}
#[kani::proof]
fn sequence_increment_rejects_overflow() {
    let start: u64 = kani::any();
    assert_eq!(
        polaris_sdk::ids::Seq::new(start)
            .next()
            .map(|seq| seq.get()),
        start.checked_add(1)
    );
}
