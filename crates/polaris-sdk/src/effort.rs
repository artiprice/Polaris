//! Reasoning effort levels (`/model`, `$subagent-models`); `polaris-models` maps them to model tiers.

use core::fmt;
use core::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{ParseError, echo};

/// Effort level, ordered from cheapest to most capable. Parses from the names
/// `low|medium|high|xhigh|max` (any case) or the chatlog's numeric form `1`–`5`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Effort {
    /// 1 — cheapest tier.
    Low,
    /// 2.
    Medium,
    /// 3.
    High,
    /// 4 — high tier with a larger reasoning budget.
    XHigh,
    /// 5 — most capable tier.
    Max,
}

impl Effort {
    /// All levels in ascending order.
    pub const ALL: [Self; 5] = [Self::Low, Self::Medium, Self::High, Self::XHigh, Self::Max];

    /// The numeric form `1`–`5`.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
            Self::XHigh => 4,
            Self::Max => 5,
        }
    }

    /// The lowercase name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

impl TryFrom<u8> for Effort {
    type Error = ParseError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Low),
            2 => Ok(Self::Medium),
            3 => Ok(Self::High),
            4 => Ok(Self::XHigh),
            5 => Ok(Self::Max),
            other => Err(ParseError::InvalidEffortLevel(other)),
        }
    }
}

impl FromStr for Effort {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let text = s.trim();
        if let Some(level) = Self::ALL
            .into_iter()
            .find(|level| level.name().eq_ignore_ascii_case(text))
        {
            return Ok(level);
        }
        match text.parse::<u8>() {
            Ok(n) if text.len() == 1 => {
                Self::try_from(n).map_err(|_| ParseError::InvalidEffort(echo(s)))
            }
            _ => Err(ParseError::InvalidEffort(echo(s))),
        }
    }
}

impl fmt::Display for Effort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Human-readable formats use the name; compact binary formats (postcard) use the numeric form.
impl Serialize for Effort {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serializer.serialize_str(self.name())
        } else {
            serializer.serialize_u8(self.as_u8())
        }
    }
}

struct EffortVisitor;

impl Visitor<'_> for EffortVisitor {
    type Value = Effort;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an effort name (low|medium|high|xhigh|max) or an integer 1-5")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Effort, E> {
        v.parse().map_err(E::custom)
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Effort, E> {
        u8::try_from(v)
            .map_err(|_| E::custom(ParseError::InvalidEffort(echo(&v.to_string()))))
            .and_then(|n| Effort::try_from(n).map_err(E::custom))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Effort, E> {
        u64::try_from(v)
            .map_err(|_| E::custom(ParseError::InvalidEffort(echo(&v.to_string()))))
            .and_then(|n| self.visit_u64(n))
    }
}

impl<'de> Deserialize<'de> for Effort {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            deserializer.deserialize_any(EffortVisitor)
        } else {
            deserializer.deserialize_u8(EffortVisitor)
        }
    }
}
