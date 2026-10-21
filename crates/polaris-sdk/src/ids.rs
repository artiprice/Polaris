//! Identifier and quantity newtypes. Small ones are `Copy`; string ones validate on construction
//! and on deserialization.

use core::fmt;
use core::str::FromStr;
use std::borrow::Cow;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::error::{ParseError, echo};
use crate::hex;

macro_rules! uuid_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// A new time-ordered id (UUID v7).
            #[must_use]
            pub fn new_v7() -> Self {
                Self(Uuid::now_v7())
            }

            /// An id from a raw 128-bit value (fixtures, replay, tests).
            #[must_use]
            pub const fn from_u128(value: u128) -> Self {
                Self(Uuid::from_u128(value))
            }

            /// The underlying UUID.
            #[must_use]
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl FromStr for $name {
            type Err = ParseError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self).map_err(|_| ParseError::InvalidUuid(echo(s)))
            }
        }
    };
}

uuid_id!(
    /// A user session (persisted, resumable).
    SessionId
);
uuid_id!(
    /// An agent context frame inside a session.
    FrameId
);
uuid_id!(
    /// A command submitted to the kernel.
    CommandId
);

/// Event-log sequence number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seq(u64);

impl Seq {
    /// The first sequence number of an empty log.
    pub const ZERO: Self = Self(0);

    /// Wraps a raw sequence number.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The raw value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The following sequence number, or `None` on overflow.
    #[must_use]
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }
}

/// Serializes fixed-size bytes as lowercase hex in human-readable formats, raw bytes otherwise.
fn serialize_bytes<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    if serializer.is_human_readable() {
        serializer.serialize_str(&hex::encode(bytes))
    } else {
        serializer.serialize_bytes(bytes)
    }
}

/// BLAKE3 content id: the CAS key. Computed by `polaris-storage-cas`; sdk only carries it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cid([u8; 32]);

impl Cid {
    /// Wraps a 32-byte BLAKE3 digest.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// 64 lowercase hex characters.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(&self.0)
    }

    /// Parses 64 hex characters (either case).
    ///
    /// # Errors
    /// [`ParseError::InvalidHex`] for any other length or a non-hex character.
    pub fn from_hex(text: &str) -> Result<Self, ParseError> {
        hex::decode_array::<32>(text).map(Self)
    }
}

impl fmt::Display for Cid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Cid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cid({})", self.to_hex())
    }
}

impl FromStr for Cid {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

impl Serialize for Cid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_bytes(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for Cid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let text = Cow::<str>::deserialize(deserializer)?;
            Self::from_hex(&text).map_err(serde::de::Error::custom)
        } else {
            <[u8; 32]>::deserialize(deserializer).map(Self)
        }
    }
}

/// Prefix of a short reference marker inside model-visible text: `§ref:<12hex>`.
pub const SHORT_REF_PREFIX: &str = "§ref:";

/// First 48 bits of a [`Cid`]; rendered as 12 lowercase hex characters in `§ref:<12hex>` markers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShortRef([u8; 6]);

impl ShortRef {
    /// The short reference of a content id.
    #[must_use]
    pub fn of(cid: &Cid) -> Self {
        let mut out = [0u8; 6];
        for (slot, byte) in out.iter_mut().zip(cid.as_bytes()) {
            *slot = *byte;
        }
        Self(out)
    }

    /// The six raw bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }

    /// The marker form `§ref:<12hex>` used in folded context.
    #[must_use]
    pub fn to_marker(&self) -> String {
        let mut out = String::from(SHORT_REF_PREFIX);
        hex::encode_into(&self.0, &mut out);
        out
    }
}

impl fmt::Display for ShortRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(&self.0))
    }
}

impl fmt::Debug for ShortRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ShortRef({self})")
    }
}

impl FromStr for ShortRef {
    type Err = ParseError;

    /// Accepts `§ref:<12hex>` or bare `<12hex>` (either case).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex_part = s.strip_prefix(SHORT_REF_PREFIX).unwrap_or(s);
        hex::decode_array::<6>(hex_part).map(Self)
    }
}

impl Serialize for ShortRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serialize_bytes(&self.0, serializer)
    }
}

impl<'de> Deserialize<'de> for ShortRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let text = Cow::<str>::deserialize(deserializer)?;
            text.parse().map_err(serde::de::Error::custom)
        } else {
            <[u8; 6]>::deserialize(deserializer).map(Self)
        }
    }
}

/// xxh3-64 fingerprint for cheap dirty checks. **Not collision resistant**: never a security or
/// CAS key, and deliberately not convertible to or from [`Cid`].
///
/// ```compile_fail
/// use polaris_sdk::ids::{Cid, FastFingerprint};
/// let cid: Cid = FastFingerprint::new(42).into();
/// ```
///
/// ```
/// use polaris_sdk::ids::FastFingerprint;
/// assert_eq!(FastFingerprint::new(42).get(), 42);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FastFingerprint(u64);

impl FastFingerprint {
    /// Wraps a precomputed xxh3-64 value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The raw value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Validates an identifier-like string: non-empty, bounded, printable, no whitespace.
fn validate_name(
    what: &'static str,
    value: &str,
    max_len: usize,
    allowed: fn(char) -> bool,
) -> Result<(), ParseError> {
    if value.is_empty() {
        return Err(ParseError::InvalidValue {
            what,
            reason: "empty",
        });
    }
    if value.len() > max_len {
        return Err(ParseError::InvalidValue {
            what,
            reason: "too long",
        });
    }
    if !value.chars().all(allowed) {
        return Err(ParseError::InvalidValue {
            what,
            reason: "contains a disallowed character",
        });
    }
    Ok(())
}

fn model_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':')
}

/// A model identifier as accepted by an OpenAI-compatible endpoint, e.g.
/// `nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B` or `nemotron-mini:latest`. Validated, never hard-coded.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ModelId(Box<str>);

impl ModelId {
    /// Longest accepted model id in bytes.
    pub const MAX_LEN: usize = 128;

    /// Validates and wraps a model id.
    ///
    /// # Errors
    /// [`ParseError::InvalidValue`] when empty, longer than [`Self::MAX_LEN`], or containing
    /// characters other than ASCII alphanumerics and `-_./:`.
    pub fn new(value: &str) -> Result<Self, ParseError> {
        validate_name("model id", value, Self::MAX_LEN, model_char)?;
        Ok(Self(value.into()))
    }

    /// The id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ModelId {
    type Error = ParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<ModelId> for String {
    fn from(value: ModelId) -> Self {
        value.0.into_string()
    }
}

impl FromStr for ModelId {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Money in millionths of a US dollar; checked arithmetic only.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct MicroUsd(u64);

impl MicroUsd {
    /// Wraps an amount in micro-dollars.
    #[must_use]
    pub const fn new(micros: u64) -> Self {
        Self(micros)
    }

    /// The amount in micro-dollars.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Sum, or `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// Difference, or `None` if it would be negative.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// Product with a count, or `None` on overflow.
    #[must_use]
    pub const fn checked_mul(self, factor: u64) -> Option<Self> {
        match self.0.checked_mul(factor) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }
}

/// A number of model tokens.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TokenCount(u32);

impl TokenCount {
    /// Wraps a token count.
    #[must_use]
    pub const fn new(tokens: u32) -> Self {
        Self(tokens)
    }

    /// The count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Sum, or `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }
}

/// Role of an agent context frame (provisional set; semantics live in the runtime crates).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    /// Top-level frame that plans and delegates.
    Orchestrator,
    /// Delegated worker frame.
    SubAgent,
    /// Checks proposed changes and completions.
    Verifier,
    /// Long-running `/research` loop.
    Researcher,
    /// Zero-token adversary (`/hack`).
    Hacker,
    /// Goal-drift / loop / fatigue monitor.
    Watchdog,
}
