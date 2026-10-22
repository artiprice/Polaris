//! # polaris-platform-wsl
//!
//! Native WSL2 discovery, safe argv execution, path conversion, config validation, and
//! keepalive process ownership. Provisioning returns an explicit plan; it never installs a
//! distro or changes host configuration implicitly.

mod adapter;

pub use adapter::*;
