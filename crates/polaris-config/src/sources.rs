//! Where layers come from: directories, files, the environment and session overrides.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{ErrorKind as IoKind, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{ConfigError, invalid};
use crate::layer::{Layer, LayerInput, insert_path, leaves};

/// Config file name inside a `.polaris` directory.
pub const CONFIG_FILE: &str = "config.toml";

/// Environment variable prefix; `__` separates nesting levels.
pub const ENV_PREFIX: &str = "POLARIS_";

/// Maximum size of each on-disk TOML layer before parsing.
pub const MAX_CONFIG_FILE_BYTES: usize = 1_048_576;
const MAX_CONFIG_FILE_BYTES_U64: u64 = 1_048_576;

/// The configuration directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    /// `<home>/.polaris`.
    pub global: PathBuf,
    /// Nearest `.polaris/` at or above the working directory (canonical, no `\\?\` prefix),
    /// excluding the global directory itself.
    pub repo: Option<PathBuf>,
}

fn io_error(path: &Path, err: &std::io::Error) -> ConfigError {
    ConfigError::Io {
        path: path.to_path_buf(),
        reason: err.to_string(),
    }
}

/// Finds the global and repository directories for `cwd` with an explicit `home`.
///
/// The repository directory is the nearest `.polaris/` walking up from `cwd`. The walk stops at
/// `home` (exclusive): `~/.polaris` is the global layer, and directories above home are never
/// projects. Outside home the walk continues to the filesystem root.
///
/// # Errors
/// [`ConfigError::Io`] if `cwd` cannot be canonicalised.
pub fn discover_from(cwd: &Path, home: &Path) -> Result<Dirs, ConfigError> {
    let home_canonical = dunce::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    let start = dunce::canonicalize(cwd).map_err(|e| io_error(cwd, &e))?;
    let repo = start
        .ancestors()
        .take_while(|dir| *dir != home_canonical)
        .map(|dir| dir.join(".polaris"))
        .filter(|candidate| candidate.is_dir())
        .find_map(|candidate| dunce::canonicalize(candidate).ok());
    Ok(Dirs {
        global: home.join(".polaris"),
        repo,
    })
}

/// [`discover_from`] with the current user's home directory.
///
/// # Errors
/// [`ConfigError::NoHome`], or as [`discover_from`].
pub fn discover(cwd: &Path) -> Result<Dirs, ConfigError> {
    let home = etcetera::home_dir().map_err(|_| ConfigError::NoHome)?;
    discover_from(cwd, &home)
}

/// Read attempts for a file another process holds open without sharing (editors, antivirus).
const READ_ATTEMPTS: u32 = 8;
/// First retry delay; doubles up to [`READ_MAX_DELAY`] (about 0.7 s of waiting in total).
const READ_BASE_DELAY: Duration = Duration::from_millis(10);
/// Longest single retry delay.
const READ_MAX_DELAY: Duration = Duration::from_millis(200);

/// "File in use" errors: Windows `ERROR_SHARING_VIOLATION` (32) and `ERROR_LOCK_VIOLATION` (33).
fn is_in_use(error: &std::io::Error) -> bool {
    cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33))
}

/// Reads `path`, retrying "file in use" errors with a bounded backoff.
enum ReadFailure {
    Io(std::io::Error),
    TooLarge,
}

fn read_once(path: &Path) -> Result<String, ReadFailure> {
    let file = std::fs::File::open(path).map_err(ReadFailure::Io)?;
    let mut text = String::new();
    let take_limit = MAX_CONFIG_FILE_BYTES_U64
        .checked_add(1)
        .ok_or(ReadFailure::TooLarge)?;
    file.take(take_limit)
        .read_to_string(&mut text)
        .map_err(ReadFailure::Io)?;
    if text.len() > MAX_CONFIG_FILE_BYTES {
        return Err(ReadFailure::TooLarge);
    }
    Ok(text)
}

fn read_with_retry(path: &Path) -> Result<String, ReadFailure> {
    let mut delay = READ_BASE_DELAY;
    let mut attempt = 1;
    loop {
        match read_once(path) {
            Err(ReadFailure::Io(e)) if is_in_use(&e) && attempt < READ_ATTEMPTS => {
                std::thread::sleep(delay);
                delay = delay.saturating_mul(2).min(READ_MAX_DELAY);
                attempt += 1;
            }
            result => return result,
        }
    }
}

fn read_file_layer(layer: Layer, dir: &Path) -> Result<Option<LayerInput>, ConfigError> {
    let path = dir.join(CONFIG_FILE);
    match read_with_retry(&path) {
        Ok(text) => LayerInput::parse(layer, &text, Some(path)).map(Some),
        Err(ReadFailure::Io(e)) if e.kind() == IoKind::NotFound => Ok(None),
        Err(ReadFailure::Io(e)) => Err(io_error(&path, &e)),
        Err(ReadFailure::TooLarge) => Err(ConfigError::FileTooLarge {
            path,
            limit: MAX_CONFIG_FILE_BYTES,
        }),
    }
}

/// Reads the file, environment and session layers; missing files and empty layers are skipped.
///
/// # Errors
/// [`ConfigError::Io`], [`ConfigError::Parse`] or [`ConfigError::Invalid`].
pub fn read_layers(
    dirs: &Dirs,
    env: &EnvSnapshot,
    session: &SessionOverrides,
) -> Result<Vec<LayerInput>, ConfigError> {
    let mut layers = Vec::new();
    layers.extend(read_file_layer(Layer::Global, &dirs.global)?);
    if let Some(repo) = &dirs.repo {
        layers.extend(read_file_layer(Layer::Repo, repo)?);
    }
    for layer in [env.to_layer()?, session.to_layer()?] {
        if !layer.table.is_empty() {
            layers.push(layer);
        }
    }
    Ok(layers)
}

/// The `POLARIS_*` environment variables, captured once.
///
/// `Debug` prints variable names only: a misconfigured variable may hold a literal secret.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct EnvSnapshot {
    vars: BTreeMap<String, String>,
}

impl fmt::Debug for EnvSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.vars.keys()).finish()
    }
}

impl EnvSnapshot {
    /// No variables.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Keeps the `POLARIS_*` pairs of `pairs`.
    #[must_use]
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let vars = pairs
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .filter(|(k, _)| k.starts_with(ENV_PREFIX))
            .collect();
        Self { vars }
    }

    /// Captures the process environment (non-UTF-8 variables are skipped).
    #[must_use]
    pub fn capture() -> Self {
        Self::from_pairs(
            std::env::vars_os()
                .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))),
        )
    }

    /// `POLARIS_RATELIMIT__RPM=600` → `ratelimit.rpm = 600`. Integers and `true`/`false` are typed;
    /// everything else is a string.
    ///
    /// # Errors
    /// [`ConfigError::Invalid`] when two variables map to conflicting paths.
    pub fn to_layer(&self) -> Result<LayerInput, ConfigError> {
        let mut table = toml::Table::new();
        for (name, raw) in &self.vars {
            let rest = name.strip_prefix(ENV_PREFIX).unwrap_or(name);
            let key = rest
                .split("__")
                .map(str::to_ascii_lowercase)
                .collect::<Vec<_>>()
                .join(".");
            let value = match raw.as_str() {
                "true" => toml::Value::Boolean(true),
                "false" => toml::Value::Boolean(false),
                other => other.parse::<i64>().map_or_else(
                    |_| toml::Value::String(other.to_owned()),
                    toml::Value::Integer,
                ),
            };
            if !insert_path(&mut table, &key, value) {
                return Err(invalid(
                    &key,
                    format!("{name} conflicts with another POLARIS_ variable"),
                ));
            }
        }
        Ok(LayerInput {
            layer: Layer::Env,
            table,
            origin: None,
        })
    }
}

/// Session-scoped overrides set by `$directives` (D11); persisted with the session for resume.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionOverrides {
    values: BTreeMap<String, toml::Value>,
}

impl SessionOverrides {
    /// Sets `key` (a dotted path) for the rest of the session.
    pub fn set(&mut self, key: impl Into<String>, value: toml::Value) -> &mut Self {
        self.values.insert(key.into(), value);
        self
    }

    /// Removes an override, returning the previous value.
    pub fn remove(&mut self, key: &str) -> Option<toml::Value> {
        self.values.remove(key)
    }

    /// The override for `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&toml::Value> {
        self.values.get(key)
    }

    /// `true` when nothing is overridden.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    fn table(&self) -> Result<toml::Table, ConfigError> {
        let mut table = toml::Table::new();
        for (key, value) in &self.values {
            if !insert_path(&mut table, key, value.clone()) {
                return Err(invalid(key, "conflicts with another session override"));
            }
        }
        Ok(table)
    }

    /// The overrides as a TOML document.
    ///
    /// # Errors
    /// [`ConfigError::Invalid`] for conflicting keys, [`ConfigError::Encode`] if TOML encoding fails.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string(&self.table()?).map_err(|e| ConfigError::Encode(e.to_string()))
    }

    /// Restores overrides saved with [`SessionOverrides::to_toml`].
    ///
    /// # Errors
    /// [`ConfigError::Parse`].
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        let input = LayerInput::parse(Layer::Session, text, None)?;
        let values = leaves(&input.table)
            .into_iter()
            .map(|(k, v)| (k, v.clone()))
            .collect();
        Ok(Self { values })
    }

    /// The overrides as the [`Layer::Session`] layer.
    ///
    /// # Errors
    /// [`ConfigError::Invalid`] for conflicting keys.
    pub fn to_layer(&self) -> Result<LayerInput, ConfigError> {
        Ok(LayerInput {
            layer: Layer::Session,
            table: self.table()?,
            origin: None,
        })
    }
}
