//! # polaris-web
//!
//! Axum + Leptos SSR web UI with streaming, per-route lazy loading and a PWA service worker.
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L7 — apps |
//! | Kind | app |
//! | Phase | P9 |
//! | Parent facade | — |
//! | Status | SSR + hydration over the shared session host; PWA manifest and service worker; per-route lazy loading and WebSocket terminals not yet |
//!
//! ## Layout
//!
//! One crate, two builds (cargo-leptos dual-feature layout):
//!
//! - `ssr` (default, native): the `polaris-web` binary. It starts the session owner, JSON API and
//!   event stream of [`polaris_ui_web`] and serves this app's pages in place of the embedded
//!   JavaScript page: out-of-order streamed HTML with a per-response CSP nonce, `/pkg` for the
//!   browser bundle, `/manifest.webmanifest`, `/icon.svg` and `/sw.js`.
//! - `hydrate` (wasm32, built by `cargo leptos build`): the same components hydrated in the
//!   browser. The page's behaviour is Rust: [`state::UiState`] reduces server events and clicks
//!   into network [`state::Effect`]s, which the browser client performs with the access token
//!   from the URL fragment.
//!
//! The service worker (`assets/sw.js`) is the only hand-written JavaScript: it caches the shell
//! and bundle for offline start and never caches the API.
//!
//! ## Internal dependencies
//!
//! `polaris-ui-web`, `polaris-config` (server build only)

// Leptos view types nest deeply; the default limit is too small for a full page.
#![recursion_limit = "512"]

pub mod app;
pub mod events;
pub mod markdown;
pub mod state;

#[cfg(feature = "hydrate")]
mod client;

#[cfg(feature = "ssr")]
pub mod server;

/// Browser entry point called by the generated loader after the wasm module is fetched.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(app::App);
}
