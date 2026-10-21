//! Stored event format: `[schema: u16 little-endian][JSON of the sdk EventRecord]`.
//!
//! JSON, not postcard: the sdk `Event` is an internally tagged serde enum, which postcard can
//! encode but not decode (`WontImplement`, VERIFIED in P1). JSON is self-describing, so new event
//! variants never break old logs. The schema prefix is checked before parsing, so a record from a
//! newer Polaris is reported as such instead of being misread.

use polaris_sdk::contracts::{EVENT_SCHEMA_VERSION, EventRecord};

use crate::error::EventLogError;

/// Newest record schema this build reads (the sdk event schema).
pub const SUPPORTED_SCHEMA: u16 = EVENT_SCHEMA_VERSION;

/// Encodes a record with its validated envelope's schema prefix, preserving old records.
///
/// # Errors
/// [`EventLogError::Codec`] if serialization fails.
pub fn encode_record(record: &EventRecord) -> Result<Vec<u8>, EventLogError> {
    let mut out = record.schema().to_le_bytes().to_vec();
    serde_json::to_writer(&mut out, record).map_err(|e| EventLogError::Codec(e.to_string()))?;
    Ok(out)
}

/// Decodes a stored record.
///
/// # Errors
/// [`EventLogError::SchemaTooNew`] for a newer schema prefix, [`EventLogError::Codec`] for short,
/// zero-schema or malformed input.
pub fn decode_record(bytes: &[u8]) -> Result<EventRecord, EventLogError> {
    let [lo, hi, json @ ..] = bytes else {
        return Err(EventLogError::Codec(
            "record shorter than its schema prefix".to_owned(),
        ));
    };
    let schema = u16::from_le_bytes([*lo, *hi]);
    if schema > SUPPORTED_SCHEMA {
        return Err(EventLogError::SchemaTooNew {
            found: schema,
            supported: SUPPORTED_SCHEMA,
        });
    }
    if schema == 0 {
        return Err(EventLogError::Codec("schema 0 is invalid".to_owned()));
    }
    let record: EventRecord =
        serde_json::from_slice(json).map_err(|e| EventLogError::Codec(e.to_string()))?;
    if record.schema() != schema {
        return Err(EventLogError::Codec(
            "storage prefix and event envelope schema differ".to_owned(),
        ));
    }
    Ok(record)
}
