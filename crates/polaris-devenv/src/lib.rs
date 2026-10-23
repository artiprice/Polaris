//! Development environment adapters and sandbox-backed CI pipelines.
//!
//! The Nix child crate provides print-dev-env command construction, bounded JSON decoding,
//! environment filtering, exact-cache helpers, and an S2 prompt builder. Evaluation still needs a
//! composition-root sandbox policy and a runtime-verified Nix fixture. The Docker child crate
//! provides hardened one-shot execution, bounded build-context validation, and a single-use warm
//! pool with exact ownership labels and a durable journal seam. The L5 Kernel adapter connects
//! pool cleanup to Reaper without an upward Docker dependency. BuildKit cache mounts and live
//! Engine behavior remain unverified. CI is implemented in its child crate but still has separate
//! Dagger and validation work outstanding.

pub use polaris_devenv_cicd as cicd;
pub use polaris_devenv_docker as docker;
pub use polaris_devenv_nix as nix;
