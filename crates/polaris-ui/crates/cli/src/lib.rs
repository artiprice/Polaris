//! Pure Ratatui screen projection, layout and safe text rendering for the Polaris CLI.

/// Bounded, responsive screen geometry.
pub mod layout;
/// Pure Ratatui screen painting and deterministic dump APIs.
pub mod screen;
/// Safe terminal text shaping and Unicode-width wrapping.
pub mod text;

mod bracketed_paste;
/// Crossterm event normalization and bounded paste handling.
pub mod input;
mod input_decoder;
/// Bounded Ratatui frame staging and cursor output.
pub mod presenter;
/// Ordered semantic event scheduling and frame deadlines.
pub mod scheduler;
/// Crossterm terminal mode acquisition and restoration.
pub mod terminal;
/// Safe Win32 console input adapter.
#[cfg(windows)]
pub mod windows_input;

/// Shared Unicode editor geometry and source mapping.
pub mod editor;
