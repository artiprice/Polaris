//! # polaris-spike-web
//!
//! Spike tests for the web/daemon surface: client connection floods and SSE fan-out bursts.
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L8 — tests, spikes & tooling |
//! | Kind | spike |
//! | Phase | P9 |
//! | Parent facade | — |
//! | Status | executable fan-out and connection-flood scenarios (`tests/`) |
//!
//! ## Responsibilities
//!
//! - Assert bounded latency and memory for sudden client spikes.
//!
//! ## Scenarios
//!
//! - `sse_fanout`: 64 pages hold an event stream open; one command must reach every one of them,
//!   with the delivery latency recorded in an HDR histogram and bounded at p99.
//! - `connection_flood`: hundreds of concurrent page loads and short-lived event streams; the
//!   server keeps answering, a fresh stream still receives events, and resident memory stays
//!   bounded.
//!
//! The server runs with no model provider (`provider.order = []`), so nothing leaves the machine.
//!
//! ## Internal dependencies
//!
//! `polaris-ui-web`
