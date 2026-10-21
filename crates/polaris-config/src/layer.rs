//! Configuration layers and TOML table helpers.

use std::path::PathBuf;

use crate::error::ConfigError;

/// A configuration layer, in ascending precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    /// Built-in defaults.
    Defaults,
    /// `~/.polaris/config.toml`.
    Global,
    /// `<repo>/.polaris/config.toml` (untrusted: filtered).
    Repo,
    /// `POLARIS_*` environment variables.
    Env,
    /// Session overrides from `$directives` (D11).
    Session,
}

/// One parsed layer.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerInput {
    /// Which layer.
    pub layer: Layer,
    /// The layer's table.
    pub table: toml::Table,
    /// Source file, if any.
    pub origin: Option<PathBuf>,
}

impl LayerInput {
    /// Parses TOML text (a leading UTF-8 BOM and CRLF line endings are accepted).
    ///
    /// # Errors
    /// [`ConfigError::Parse`] with the file (or `<Layer>`) and 1-based line.
    pub fn parse(layer: Layer, text: &str, origin: Option<PathBuf>) -> Result<Self, ConfigError> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        match toml::from_str::<toml::Table>(text) {
            Ok(table) => Ok(Self {
                layer,
                table,
                origin,
            }),
            Err(e) => {
                let line = e
                    .span()
                    .and_then(|span| text.get(..span.start))
                    .map_or(1, |before| before.matches('\n').count().saturating_add(1));
                Err(ConfigError::Parse {
                    file: origin.unwrap_or_else(|| PathBuf::from(format!("<{layer:?}>"))),
                    line: u32::try_from(line).unwrap_or(u32::MAX),
                    reason: e.message().to_owned(),
                })
            }
        }
    }
}

/// Every leaf (non-table value) of `table` as `(dotted.path, value)`.
pub(crate) fn leaves(table: &toml::Table) -> Vec<(String, &toml::Value)> {
    fn walk<'a>(prefix: &str, table: &'a toml::Table, out: &mut Vec<(String, &'a toml::Value)>) {
        for (key, value) in table {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match value {
                toml::Value::Table(inner) => walk(&path, inner, out),
                leaf => out.push((path, leaf)),
            }
        }
    }
    let mut out = Vec::new();
    walk("", table, &mut out);
    out
}

/// The value at a dotted path.
pub(crate) fn lookup<'a>(table: &'a toml::Table, path: &str) -> Option<&'a toml::Value> {
    let mut segments = path.split('.');
    let first = table.get(segments.next()?)?;
    segments.try_fold(first, |value, segment| value.as_table()?.get(segment))
}

/// Inserts `value` at a dotted path, creating intermediate tables.
///
/// Returns `false` (and changes nothing at the conflict) when the path runs through a scalar or
/// would replace a table with a scalar, e.g. `a = 1` together with `a.b = 2`.
#[must_use]
pub(crate) fn insert_path(table: &mut toml::Table, path: &str, value: toml::Value) -> bool {
    let mut segments: Vec<&str> = path.split('.').collect();
    let Some(last) = segments.pop() else {
        return false;
    };
    let mut current = table;
    for segment in segments {
        let entry = current
            .entry(segment.to_owned())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let toml::Value::Table(next) = entry else {
            return false;
        };
        current = next;
    }
    if current.get(last).is_some_and(toml::Value::is_table) && !value.is_table() {
        return false;
    }
    current.insert(last.to_owned(), value);
    true
}
