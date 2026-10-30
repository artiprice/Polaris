//! # polaris-cli
//!
//! Terminal client: ratatui TUI (default) and headless `polaris run` mode; binary name `polaris`.
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L7 — application |
//! | Kind | app |
//! | Phase | P1 |
//! | Parent facade | — |
//! | Status | interactive CLI, isolated demo, local controls and deterministic dumps implemented; UI release gates remain open |
//!
//! ## Responsibilities
//!
//! - Parse CLI arguments (clap) and resolve config layers.
//! - Embed the kernel in-process by default, or attach to `polarisd` with `--connect`.
//! - Run the TUI event loop (composer, completion, $ / / / @ grammar, sessions, HUD).
//! - Provide `--dump WxH` deterministic snapshot mode and `polaris run` headless mode.
//! - Install the rustls aws-lc-rs crypto provider and the global allocator at startup.
//!
//! ## Planned API
//!
//! - bin `polaris`
//! - `polaris run <task>`
//! - `polaris doctor`
//! - `polaris --dump WxH`
//!
//! ## Internal dependencies
//!
//! `polaris-kernel`, `polaris-ui-cli`, `polaris-rpc`, `polaris-config`, `polaris-log`
//!
//! ## Planned external dependencies
//!
//! `tokio`, `clap`, `crossterm`, `ratatui`, `miette`, `mimalloc`, `rustls`

mod app;
/// CLI preferences editor using the configuration owner's atomic persistence boundary.
pub mod cli_settings;
pub mod demo;
// Some gallery helpers (effect consumption, page navigation) are not wired to an input path yet.
#[allow(dead_code)]
mod demo_gallery;
mod doctor;
pub mod key_trace;
mod rtk;
mod sandbox;
pub mod session;
mod task;
pub mod tui;
/// Bounded clipboard and workspace-index workers owned by the CLI controller.
pub mod ui_services;
pub use app::{Action, Args, CliError, install_crypto_provider, run};
pub use rtk::{RtkArgs, RtkCliError, RtkModeArg};
pub use sandbox::{SandboxAction, SandboxArgs};
