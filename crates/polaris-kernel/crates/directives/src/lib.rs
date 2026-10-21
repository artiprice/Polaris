//! # polaris-kernel-directives
//!
//! Turns one user message into typed values: an optional [`slash::SlashCommand`], session-scoped
//! [`directive::SessionDirective`]s (`$name[(args)]`), [`mention::Mention`]s (`@path`,
//! `@docx(path)`, …) with byte spans, and the residual prose.
//!
//! The parser is a hand-written recursive descent over `&str` with checked slicing only: total
//! (never panics), linear in the input, with input-size, nesting-depth and argument-count limits
//! ([`ParseLimits`]); no regex. Grammar and catalog: PLAN.md §13, `polaris-kernel-directives`.
//!
//! Rules worth knowing:
//! - `/name` is a command only when it is the first non-whitespace text and is followed by
//!   whitespace or the end (`/usr/bin …` is prose). Unknown commands are errors.
//! - `$` and `@` start a token only after the start, whitespace, `(` or `[`; `$` also needs a
//!   lowercase letter next (`$5`, `$HOME`, `a$b` and `dev@company.com` stay prose).
//! - Text inside `` `code` `` spans and ```` ``` ```` fences is never parsed.
//! - Unknown `$names` become [`directive::SessionDirective::Custom`] metadata, never errors.
//! - D11: directives are session-scoped; [`state::DirectiveState`] folds them (last occurrence
//!   wins) and [`render::render`] gives the canonical text persisted for `/session resume`.

mod args;
mod cursor;
pub mod directive;
pub mod effort;
mod error;
#[cfg(feature = "kani")]
mod kani_proofs;
pub mod mention;
pub mod names;
mod parser;
pub mod render;
pub mod slash;
pub mod state;

pub use error::DirectiveError;
pub use parser::{ParseLimits, ParsedInput, Segment, Spanned, parse, parse_segments};
