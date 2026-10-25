//! Pure Ratatui screen projection, layout and safe text rendering for the Polaris CLI.

/// Bounded, responsive screen geometry.
pub mod layout;
/// Pure Ratatui screen painting and deterministic dump APIs.
pub mod screen;
/// Safe terminal text shaping and Unicode-width wrapping.
pub mod text;

/// Crossterm event normalization and bounded paste handling.
pub mod input;
/// Bounded Ratatui frame staging and cursor output.
pub mod presenter;
/// Ordered semantic event scheduling and frame deadlines.
pub mod scheduler;
/// Crossterm terminal mode acquisition and restoration.
pub mod terminal;
