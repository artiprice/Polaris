//! Hostile protobuf bytes must fail safely; accepted SDK conversions preserve wire round trips.
#![no_main]
use libfuzzer_sys::fuzz_target;
use polaris_proto::pb;
use polaris_sdk::contracts::{Command, EventRecord, Unverified};
use prost::Message;
fuzz_target!(|data: &[u8]| {
    if data.len() > polaris_proto::limits::MAX_MESSAGE_BYTES {
        return;
    }
    if let Ok(wire) = pb::Command::decode(data) {
        if let Ok(command) = Command::<Unverified>::try_from(wire) {
            let encoded = pb::Command::from(&command).encode_to_vec();
            let roundtrip = pb::Command::decode(encoded.as_slice())
                .ok()
                .and_then(|wire| Command::<Unverified>::try_from(wire).ok());
            assert_eq!(roundtrip, Some(command));
        }
    }
    if let Ok(wire) = pb::EventRecord::decode(data) {
        if let Ok(record) = EventRecord::try_from(wire) {
            let encoded = pb::EventRecord::from(&record).encode_to_vec();
            let roundtrip = pb::EventRecord::decode(encoded.as_slice())
                .ok()
                .and_then(|wire| EventRecord::try_from(wire).ok());
            assert_eq!(roundtrip, Some(record));
        }
    }
});
