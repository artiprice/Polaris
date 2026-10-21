//! Layer merging with provenance and schema checks.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::error::ConfigError;
use crate::layer::{Layer, LayerInput, insert_path, leaves};
use crate::schema::{DEFAULTS_TOML, KNOWN_KEYS, PolarisConfig, build};
use crate::secret_ref::check_secret_value;
use crate::trust::{TrustGrant, filter_repo_layer};

/// Which layer (and file) supplied each effective key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Provenance {
    entries: BTreeMap<String, (Layer, Option<PathBuf>)>,
}

impl Provenance {
    /// The layer that supplied `key`'s effective value.
    #[must_use]
    pub fn layer_of(&self, key: &str) -> Option<Layer> {
        self.entries.get(key).map(|(layer, _)| *layer)
    }

    /// The file that supplied `key`'s effective value (`None` for defaults, env and session).
    #[must_use]
    pub fn origin_of(&self, key: &str) -> Option<&Path> {
        self.entries
            .get(key)
            .and_then(|(_, origin)| origin.as_deref())
    }

    /// Every effective key with its layer and origin, sorted by key.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Layer, Option<&Path>)> {
        self.entries
            .iter()
            .map(|(key, (layer, origin))| (key.as_str(), *layer, origin.as_deref()))
    }
}

/// A resolved configuration and where each value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    /// The typed configuration.
    pub config: PolarisConfig,
    /// Per-key provenance.
    pub provenance: Provenance,
}

fn defaults() -> Result<LayerInput, ConfigError> {
    LayerInput::parse(Layer::Defaults, DEFAULTS_TOML, None)
}

/// Merges `layers` over the built-in defaults by ascending [`Layer`] (stable for equal layers):
/// tables merge, scalars replace.
///
/// Every layer is checked before it is merged: unknown keys, then literal secrets, then (for the
/// repository layer) the trust filter.
///
/// # Errors
/// [`ConfigError::UnknownKey`], [`ConfigError::SecretLiteral`], [`ConfigError::UntrustedRepoKey`],
/// or [`ConfigError::Invalid`] for a value of the wrong type or range.
pub fn load(layers: &[LayerInput], grant: &TrustGrant) -> Result<Loaded, ConfigError> {
    let base = defaults()?;
    let mut ordered: Vec<&LayerInput> = layers.iter().collect();
    ordered.sort_by_key(|input| input.layer);

    let mut merged = toml::Table::new();
    let mut provenance = Provenance::default();
    for input in std::iter::once(&base).chain(ordered) {
        let entries = leaves(&input.table);
        for (key, value) in &entries {
            if !KNOWN_KEYS.contains(&key.as_str()) {
                return Err(ConfigError::UnknownKey(key.clone()));
            }
            check_secret_value(key, value)?;
        }
        if input.layer == Layer::Repo {
            filter_repo_layer(&input.table, grant)?;
        }
        for (key, value) in entries {
            if !insert_path(&mut merged, &key, value.clone()) {
                return Err(crate::error::invalid(&key, "conflicts with another key"));
            }
            provenance
                .entries
                .insert(key, (input.layer, input.origin.clone()));
        }
    }
    Ok(Loaded {
        config: build(&merged)?,
        provenance,
    })
}
