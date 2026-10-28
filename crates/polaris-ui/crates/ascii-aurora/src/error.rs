//! Error type for the aurora engine.

use thiserror::Error;

/// Everything that can go wrong while building or rendering the aurora.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AuroraError {
    /// The font bytes could not be parsed.
    #[error("font bytes could not be parsed")]
    Font,
    /// A ramp glyph is not present in the font.
    #[error("font has no glyph for {0:?}")]
    MissingGlyph(char),
    /// The output buffer length does not match `width * height * 4`.
    #[error("output buffer holds {actual} bytes but the frame needs {expected}")]
    BufferSize {
        /// Bytes required for the current frame size.
        expected: usize,
        /// Bytes that were supplied.
        actual: usize,
    },
    /// A size computation overflowed or a cell dimension was zero.
    #[error("invalid or overflowing size")]
    Size,
}
