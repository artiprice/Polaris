//! # xtask
//!
//! Repository automation for Polaris (P0-XT-1). Rust port of the checks in
//! `@prime/tools/workspace.py`, driven by the crate table `@prime/tools/workspace.toml`:
//!
//! - `cargo xtask check-members` — member globs vs filesystem vs table;
//! - `cargo xtask check-layers` — layer rule and manifest-vs-table edges;
//! - `cargo xtask check-witness` — `ApprovalWitness::new` only in sdk and the verifier.
//!
//! Test-first descriptors: `specs/descriptors/xtask_check_members.json`,
//! `specs/descriptors/xtask_check_layers.json`.

pub mod error;
pub mod layers;
pub mod members;
pub mod metadata;
pub mod policy;
pub mod table;
