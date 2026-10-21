//! Message-size guard.
//!
//! The OpenShell gateway accepts at most 1 MiB per request (PLAN pitfall W22), so every message of
//! this protocol is kept under [`MAX_MESSAGE_BYTES`] and larger payloads are either CAS references
//! or streams of chunks of at most [`MAX_CHUNK_BYTES`]. tonic itself only limits *decoding* (4 MiB
//! by default, VERIFIED in tonic 0.14.6 `codec/mod.rs`) and does not limit encoding, so the
//! transports (`polaris-rpc`) must apply these limits explicitly:
//! `max_decoding_message_size(MAX_MESSAGE_BYTES)` and `max_encoding_message_size(MAX_MESSAGE_BYTES)`
//! on every generated client and server.

use prost::Message;

use crate::error::ProtoError;

/// Largest message the protocol allows: 1 MiB, the OpenShell gateway request limit.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// Largest payload of one stream chunk (terminal, stdin / stdout / stderr): 896 KiB, which leaves
/// 128 KiB of the message limit for framing.
pub const MAX_CHUNK_BYTES: usize = 896 * 1024;

/// Accepts a message of `len` bytes when it is at most `limit` bytes long.
///
/// # Errors
/// [`ProtoError::Oversize`] when `len > limit`.
pub const fn check_message_len(len: usize, limit: usize) -> Result<(), ProtoError> {
    if len > limit {
        Err(ProtoError::Oversize { len, limit })
    } else {
        Ok(())
    }
}

/// Encodes `message` after checking that its encoded length is at most `limit`; nothing is
/// allocated for an oversize message.
///
/// # Errors
/// [`ProtoError::Oversize`] when the encoded message is longer than `limit`.
pub fn encode_checked<M: Message>(message: &M, limit: usize) -> Result<Vec<u8>, ProtoError> {
    check_message_len(message.encoded_len(), limit)?;
    Ok(message.encode_to_vec())
}

/// Decodes `bytes` as `M` after checking that they are at most `limit` bytes long; an oversize
/// input is refused before any decoding work.
///
/// # Errors
/// [`ProtoError::Oversize`] when `bytes` is longer than `limit`, [`ProtoError::Decode`] when it is
/// not a valid encoding of `M`.
pub fn decode_checked<M: Message + Default>(bytes: &[u8], limit: usize) -> Result<M, ProtoError> {
    check_message_len(bytes.len(), limit)?;
    Ok(M::decode(bytes)?)
}
