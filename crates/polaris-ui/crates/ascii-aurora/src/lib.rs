//! Pure-Rust animated ASCII aurora.
//!
//! A procedural northern-lights field is sampled on a character grid, mapped to
//! a glyph ramp and blitted into an RGBA8 buffer with rounded tiles, inverted
//! dense glyphs, a faint grid and a soft underglow. The glyph atlas is baked
//! once from a font; the per-frame path does not allocate.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod atlas;
pub mod blit;
pub mod clock;
mod conv;
pub mod error;
pub mod field;
mod math;
pub mod params;
pub mod ramp;

pub use atlas::GlyphAtlas;
pub use blit::{Aurora, AuroraConfig, FrameSize};
pub use clock::{FrameClock, STATIC_T};
pub use error::AuroraError;
pub use ramp::RAMP;
