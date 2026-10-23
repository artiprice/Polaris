//! Finite floating-point values with checked construction and deserialization.

use core::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::MathError;

/// An `f64` known to be finite.
#[derive(Debug, Clone, Copy, PartialEq, JsonSchema)]
#[schemars(transparent)]
pub struct Finite(f64);

impl Finite {
    /// The finite numeric zero.
    pub const ZERO: Self = Self(0.0);
    /// Constructs a finite value.
    pub fn new(value: f64) -> Result<Self, MathError> {
        if value.is_finite() {
            Ok(Self(value))
        } else {
            Err(MathError::NonFinite)
        }
    }

    /// Returns the inner value.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl Serialize for Finite {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f64(self.0)
    }
}

impl<'de> Deserialize<'de> for Finite {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = f64::deserialize(deserializer)?;
        if value.is_finite() {
            Ok(Self(value))
        } else {
            Err(de::Error::custom("value must be finite"))
        }
    }
}

impl fmt::Display for Finite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}
