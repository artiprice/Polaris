//! Version boundary for durable approval alerts; old versions retain their original rules.
use polaris_sdk::{
    contracts::{EVENT_SCHEMA_VERSION, Event, EventRecord, Timestamp},
    ids::{Cid, FrameId, Seq, SessionId},
};

#[test]
fn blast_alert_requires_schema_three() -> Result<(), Box<dyn std::error::Error>> {
    let future = EVENT_SCHEMA_VERSION
        .checked_add(1)
        .ok_or("schema overflow")?;
    for schema in 0_u16..=future {
        let record = EventRecord::from_versioned(
            schema,
            Seq::ZERO,
            Timestamp::UNIX_EPOCH,
            SessionId::from_u128(1),
            Event::BlastRadiusAlert {
                frame: FrameId::from_u128(2),
                report: Cid::from_bytes([3; 32]),
            },
        );
        assert_eq!(
            record.is_ok(),
            (3..=EVENT_SCHEMA_VERSION).contains(&schema),
            "exact alert minimum and maximum schema"
        );
    }
    Ok(())
}
