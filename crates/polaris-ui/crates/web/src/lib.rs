//! # polaris-ui-web
//!
//! The local web UI (`polaris web`): an embedded single-page app served on loopback, a JSON
//! command API and a Server-Sent Events stream, backed by the same session host, provider chain
//! and agent as the terminal and desktop frontends (file changes wait for review on the page). The welcome page is the design
//! preview's (`dist/demo-design-preview-slop/webui`), optimised (no image assets); the session
//! view, panels and motion follow the Slint desktop GUI. Plan: `@prime/WEB_UI_PLAN.md`.
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L6 — UI |
//! | Kind | lib |
//! | Phase | P9 |
//! | Parent facade | `polaris-ui` |
//!
//! Leptos (the original plan) remains the upgrade path; the HTTP API is the contract.

#![forbid(unsafe_code)]

mod assets;
mod backend;
mod server;

pub use backend::{SessionRow, WebEvent};
pub use server::{WebError, WebOptions, WebServer};
