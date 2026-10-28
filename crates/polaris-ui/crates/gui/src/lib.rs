//! # polaris-ui-gui
//!
//! Slint views of the Polaris desktop app. Today this is the welcome screen: a native port of
//! the design demo's welcome page (`dist/demo-design-preview-slop/webui/index.html`) whose
//! background is the animated ASCII aurora of [`polaris_ui_ascii_aurora`].
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L6 — UI |
//! | Kind | lib |
//! | Phase | P9 (welcome shell in progress; kernel bridge deferred) |
//! | Parent facade | `polaris-ui` |
//!
//! - **Tokens.** `build.rs` writes the Slint `Tokens` global from
//!   [`polaris_ui_model::tokens::ROLES`] and [`palette::WELCOME_ROLES`]; `.slint` files carry no
//!   colour literals.
//! - **Screens.** Welcome (ASCII aurora) and session (sidebar of real sessions, conversation,
//!   docked composer). [`session_view::SessionController`] holds the flow; the desktop app's
//!   bridge implements [`backend`] over an offline kernel session.
//! - **Honest controls.** Messages are recorded locally and nothing replies (model execution is
//!   P3). Attach and voice input are disabled with a tooltip; sidebar pages the desktop shell
//!   does not have yet say so in their tooltip.
//! - **Licence.** Slint is used under its royalty-free licence (PLAN ADR-014); the About panel
//!   shows the `AboutSlint` widget the licence requires.
//!
//! Test-first descriptors: `specs/descriptors/ui_gui_*.json`.

// `deny`, not `forbid`: the Slint-generated module carries `#[allow(unsafe_code)]` items, which
// `forbid` rejects. Hand-written code in this crate contains no `unsafe`.
#![deny(unsafe_code)]

mod app;
mod aurora_driver;
pub mod backend;
mod error;
pub mod palette;
pub mod session_view;

pub use app::{
    AppOptions, AppReport, ModelOption, TRAY_DIRECTIVES, Wiring, attach, build_window, run_app,
};
pub use aurora_driver::{FrameHistogram, HISTOGRAM_BUCKETS_MS};
pub use error::GuiError;

#[allow(
    missing_docs,
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
    clippy::restriction,
    reason = "Slint-generated code"
)]
mod generated {
    slint::include_modules!();
}

pub use generated::AppWindow;
