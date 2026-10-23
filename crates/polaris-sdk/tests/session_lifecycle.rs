//! Schema evolution for durable session lifecycle records (synthetic fixtures).
use polaris_sdk::contracts::{EVENT_SCHEMA_VERSION, Event, EventRecord, Timestamp};
use polaris_sdk::ids::{Cid, Seq, SessionId};
use proptest::prelude::*;
use std::error::Error;

#[test]
fn lifecycle_records_use_current_schema() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        EVENT_SCHEMA_VERSION, 4,
        "blast alert extension requires a schema revision"
    );
    for event in [
        Event::SessionConfigured {
            configuration: Cid::from_bytes([7; 32]),
        },
        Event::SessionPaused,
        Event::SessionResumed,
    ] {
        let record = EventRecord::new(
            Seq::new(1),
            Timestamp::UNIX_EPOCH,
            SessionId::from_u128(1),
            event,
        );
        assert_eq!(record.schema(), 4, "new records use the current schema");
        assert_eq!(
            serde_json::from_str::<EventRecord>(&serde_json::to_string(&record)?)?,
            record,
            "lifecycle roundtrip"
        );
    }
    Ok(())
}

#[test]
fn schema_one_records_remain_readable() -> Result<(), Box<dyn Error>> {
    let old = r#"{"seq":1,"at":"1970-01-01T00:00:00Z","schema":1,"session":"00000000-0000-0000-0000-000000000001","event":{"type":"session_closed"}}"#;
    let record: EventRecord = serde_json::from_str(old)?;
    assert_eq!(
        record.schema(),
        1,
        "do not silently upgrade decoded envelopes"
    );
    assert_eq!(
        *record.event(),
        Event::SessionClosed,
        "old payload preserved"
    );
    Ok(())
}

#[test]
fn new_lifecycle_events_cannot_claim_schema_one() -> Result<(), Box<dyn Error>> {
    for event in [
        Event::SessionConfigured {
            configuration: Cid::from_bytes([7; 32]),
        },
        Event::SessionPaused,
        Event::SessionResumed,
    ] {
        let mut value = serde_json::to_value(EventRecord::new(
            Seq::new(1),
            Timestamp::UNIX_EPOCH,
            SessionId::from_u128(1),
            event,
        ))?;
        let object = value.as_object_mut().ok_or("envelope must be an object")?;
        let _old = object.insert("schema".to_owned(), serde_json::json!(1));
        assert!(
            serde_json::from_value::<EventRecord>(value).is_err(),
            "new payload cannot be disguised as v1"
        );
    }
    Ok(())
}

#[test]
fn versioned_constructor_validates_schema() -> Result<(), Box<dyn Error>> {
    let make = |schema, event| {
        EventRecord::from_versioned(
            schema,
            Seq::new(9),
            Timestamp::UNIX_EPOCH,
            SessionId::from_u128(1),
            event,
        )
    };
    assert_eq!(
        make(1, Event::SessionClosed)?.schema(),
        1,
        "preserve explicitly valid old version"
    );
    assert!(
        make(0, Event::SessionClosed).is_err(),
        "zero schema refused"
    );
    assert!(
        make(4, Event::SessionClosed).is_err(),
        "future schema refused"
    );
    assert!(
        make(1, Event::SessionPaused).is_err(),
        "new variant requires v2"
    );
    Ok(())
}

proptest! {
    #[test]
    fn prop_schema_acceptance_matches_lifecycle_minimum(schema in any::<u16>(), configuration in any::<[u8; 32]>()) {
        for event in [Event::SessionConfigured { configuration: Cid::from_bytes(configuration) }, Event::SessionPaused, Event::SessionResumed] {
            let result = EventRecord::from_versioned(schema, Seq::ZERO, Timestamp::UNIX_EPOCH, SessionId::from_u128(1), event);
            prop_assert_eq!(result.is_ok(), schema == 2 || schema == 3);
        }
        let legacy = EventRecord::from_versioned(schema, Seq::ZERO, Timestamp::UNIX_EPOCH, SessionId::from_u128(1), Event::SessionClosed);
        prop_assert_eq!(legacy.is_ok(), schema == 1 || schema == 2 || schema == 3);
    }
}
