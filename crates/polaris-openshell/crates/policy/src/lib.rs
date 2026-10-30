//! # polaris-openshell-policy
//!
//! Generate and lint OpenShell policy YAML (filesystem_policy, landlock, process,
//! network_policies) and provider profiles (layer L2, phase P2, parent `polaris-openshell`).
//!
//! - [`policy_for`] derives a least-privilege policy per agent role and execution tier.
//! - [`provider_profile_token_factory`] and [`provider_profile_yaml`] emit provider profiles.
//! - Policies are validated against the pinned upstream schema and its limits
//!   ([`MAX_POLICY_BYTES`], [`MAX_FILESYSTEM_PATHS`], no bare-root read-write).

mod implementation;
pub use implementation::*;
