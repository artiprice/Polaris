//! Pure Ratatui screen projection, layout and safe text rendering for the Polaris CLI.

/// Bounded, responsive screen geometry.
pub mod layout;
/// Pure Ratatui screen painting and deterministic dump APIs.
pub mod screen;
/// Safe terminal text shaping and Unicode-width wrapping.
pub mod text;

// Only the Win32 console reader decodes raw bracketed-paste bytes; elsewhere crossterm delivers
// paste events itself and the decoder is compiled for its tests alone.
#[cfg_attr(not(windows), allow(dead_code))]
mod bracketed_paste;
/// Crossterm event normalization and bounded paste handling.
pub mod input;
// Native key records come from the Win32 console reader only.
#[cfg_attr(not(windows), allow(dead_code))]
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
