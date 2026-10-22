//! # polaris-config
//!
//! Layered configuration (L1, P1): built-in defaults \< `~/.polaris/config.toml` \<
//! `<repo>/.polaris/config.toml` \< `POLARIS_*` environment \< session overrides (`$directives`, D11).
//!
//! - **Merge:** tables merge, scalars replace; precedence comes from [`Layer`], not input order.
//!   [`Provenance`] records which layer (and file) supplied every effective key (`/status`).
//! - **Strict schema:** unknown keys ([`KNOWN_KEYS`]) and ill-typed values are errors, never ignored.
//! - **Repository trust:** a cloned repository is untrusted. Its layer may only set [`REPO_ALLOWED`]
//!   keys unless the user grants more ([`TrustGrant`]), so it cannot redirect endpoints, swap
//!   credentials or downgrade the sandbox tier.
//! - **Secrets:** credential keys hold [`SecretRef`]s (`keyring:service/user`, `env:NAME`) only; a
//!   literal is rejected with an error that names the key, never the value.
//!
//! ```
//! use polaris_config::{Layer, LayerInput, TrustGrant, load};
//!
//! let global = LayerInput::parse(Layer::Global, "[ratelimit]\nrpm = 30", None)?;
//! let loaded = load(&[global], &TrustGrant::none())?;
//! assert_eq!(loaded.config.ratelimit.rpm, 30);
//! assert_eq!(loaded.provenance.layer_of("ratelimit.rpm"), Some(Layer::Global));
//! # Ok::<(), polaris_config::ConfigError>(())
//! ```
//!
//! Test-first descriptors: `specs/descriptors/config_*.json`.

mod error;
mod layer;
mod merge;
mod schema;
mod secret_ref;
mod sources;
mod trust;

pub use error::ConfigError;
pub use layer::{Layer, LayerInput};
pub use merge::{Loaded, Provenance, load};
pub use schema::{
    BudgetConfig, ConcurrencyConfig, DEFAULTS_TOML, EffortModels, KNOWN_KEYS, LogConfig, LogLevel,
    NebiusConfig, PolarisConfig, RateLimitConfig, SandboxConfig, SandboxTier, SearchConfig,
    StorageConfig, StorageLevel, Theme, UiConfig,
};
pub use secret_ref::SecretRef;
pub use sources::{
    CONFIG_FILE, Dirs, ENV_PREFIX, EnvSnapshot, MAX_CONFIG_FILE_BYTES, SessionOverrides, discover,
    discover_from, read_layers,
};
pub use trust::{REPO_ALLOWED, TrustGrant, filter_repo_layer};
