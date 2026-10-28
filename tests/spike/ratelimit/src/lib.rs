//! Deterministic rate-limit pressure workloads over the actual `DualLimiter` implementation.
//!
//! `tests/burst.rs` records observed admission latency and verifies staged priority pressure,
//! shared Retry-After penalties, conservative token reservations and owned waiter cancellation.
//! Reports are written independently under `target/spike/ratelimit-*.json`.
//!
//! This scoped matrix does not claim live-header correctness, concurrent priority scheduling,
//! unused-budget refunds, 10,000-cancellation stress, or the complete PLAN load acceptance.
