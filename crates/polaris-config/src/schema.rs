//! The typed configuration and its defaults.

use polaris_sdk::effort::Effort;
use polaris_sdk::ids::ModelId;

use crate::error::{ConfigError, invalid};
use crate::layer::lookup;
use crate::secret_ref::SecretRef;

/// Built-in defaults (the `Defaults` layer).
///
/// Model ids are the Token Factory catalog ids verified in the PLAN premise check. Rate-limit and
/// concurrency numbers are **unverified starting points** (chatlog msg 17; Token Factory limits are
/// unknown until spike S0-5) — every one of them is overridable.
pub const DEFAULTS_TOML: &str = r#"
[ratelimit]
rpm = 60
tpm = 200000

[concurrency]
inference = 6
web = 10
docs = 16
disk = 64
s1 = 3
sandbox = 4

[storage]
level = "global"

[sandbox]
tier = "auto"

[nebius]
base_url = "https://api.tokenfactory.nebius.com/v1/"

[nebius.effort]
low = "nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B"
medium = "nvidia/Nemotron-3_5-Lightning"
high = "nvidia/nemotron-3-super-120b-a12b"
xhigh = "nvidia/nemotron-3-super-120b-a12b"
max = "nvidia/Nemotron-3-Ultra-550b-a55b"

[search]

[log]
level = "info"

[budget]

[ui]
theme = "dark"
"#;

/// Every key of the schema (dotted paths of leaves).
pub const KNOWN_KEYS: &[&str] = &[
    "ratelimit.rpm",
    "ratelimit.tpm",
    "concurrency.inference",
    "concurrency.web",
    "concurrency.docs",
    "concurrency.disk",
    "concurrency.s1",
    "concurrency.sandbox",
    "storage.level",
    "sandbox.tier",
    "nebius.base_url",
    "nebius.api_key",
    "nebius.effort.low",
    "nebius.effort.medium",
    "nebius.effort.high",
    "nebius.effort.xhigh",
    "nebius.effort.max",
    "search.tavily_api_key",
    "search.searxng_url",
    "log.level",
    "budget.usd",
    "budget.tokens",
    "ui.theme",
];

/// Request and token rate limits for the model provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitConfig {
    /// Requests per minute.
    pub rpm: u32,
    /// Tokens per minute.
    pub tpm: u32,
}

/// Concurrency permits per resource pool (`$concurrency`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConcurrencyConfig {
    /// Model inference.
    pub inference: u32,
    /// Web requests.
    pub web: u32,
    /// Document parsing.
    pub docs: u32,
    /// Disk I/O.
    pub disk: u32,
    /// System-One worker.
    pub s1: u32,
    /// Sandbox operations.
    pub sandbox: u32,
}

/// Where Polaris stores state (`$storage-level`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageLevel {
    /// Shared `~/.polaris`.
    Global,
    /// Workspace-local `.polaris`.
    Isolated,
}

/// Storage settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageConfig {
    /// Storage level.
    pub level: StorageLevel,
}

/// Execution tier (`$sandbox`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxTier {
    /// Choose per task.
    Auto,
    /// OpenShell docker driver (WSL2).
    Openshell,
    /// OpenShell vm driver.
    Microvm,
    /// Token Factory Sandboxes (Contree).
    Contree,
    /// Remote OpenShell gateway.
    Remote,
}

/// Sandbox settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxConfig {
    /// Default tier.
    pub tier: SandboxTier,
}

/// Models per effort level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffortModels {
    /// Effort 1.
    pub low: ModelId,
    /// Effort 2.
    pub medium: ModelId,
    /// Effort 3.
    pub high: ModelId,
    /// Effort 4.
    pub xhigh: ModelId,
    /// Effort 5.
    pub max: ModelId,
}

impl EffortModels {
    /// The model for an effort level.
    #[must_use]
    pub const fn model(&self, effort: Effort) -> &ModelId {
        match effort {
            Effort::Low => &self.low,
            Effort::Medium => &self.medium,
            Effort::High => &self.high,
            Effort::XHigh => &self.xhigh,
            Effort::Max => &self.max,
        }
    }
}

/// Nebius Token Factory settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NebiusConfig {
    /// OpenAI-compatible base URL (ends with `/`).
    pub base_url: String,
    /// API key reference.
    pub api_key: Option<SecretRef>,
    /// Effort → model map.
    pub effort: EffortModels,
}

/// Web search settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchConfig {
    /// Tavily API key reference.
    pub tavily_api_key: Option<SecretRef>,
    /// Local SearXNG base URL.
    pub searxng_url: Option<String>,
}

/// Log verbosity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Errors only.
    Error,
    /// Warnings and errors.
    Warn,
    /// Informational.
    Info,
    /// Debug.
    Debug,
    /// Everything.
    Trace,
}

/// Logging settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogConfig {
    /// Verbosity.
    pub level: LogLevel,
}

/// Spending limits (`$budget`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetConfig {
    /// US dollars as a decimal string (parsed exactly by telemetry).
    pub usd: Option<String>,
    /// Token budget.
    pub tokens: Option<u64>,
}

/// UI theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    /// Dark (default; the demo design).
    Dark,
    /// Light.
    Light,
}

/// UI settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiConfig {
    /// Theme.
    pub theme: Theme,
}

/// The resolved configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolarisConfig {
    /// Rate limits.
    pub ratelimit: RateLimitConfig,
    /// Concurrency permits.
    pub concurrency: ConcurrencyConfig,
    /// Storage.
    pub storage: StorageConfig,
    /// Sandbox.
    pub sandbox: SandboxConfig,
    /// Nebius Token Factory.
    pub nebius: NebiusConfig,
    /// Web search.
    pub search: SearchConfig,
    /// Logging.
    pub log: LogConfig,
    /// Budget.
    pub budget: BudgetConfig,
    /// UI.
    pub ui: UiConfig,
}

/// Typed readers over a merged table.
struct Reader<'t>(&'t toml::Table);

impl Reader<'_> {
    fn value(&self, key: &str) -> Option<&toml::Value> {
        lookup(self.0, key)
    }

    fn str(&self, key: &str) -> Result<&str, ConfigError> {
        self.value(key)
            .and_then(toml::Value::as_str)
            .ok_or_else(|| invalid(key, "expected a string"))
    }

    fn opt_str(&self, key: &str) -> Result<Option<&str>, ConfigError> {
        match self.value(key) {
            None => Ok(None),
            Some(v) => v
                .as_str()
                .map(Some)
                .ok_or_else(|| invalid(key, "expected a string")),
        }
    }

    fn positive_u32(&self, key: &str) -> Result<u32, ConfigError> {
        let n = self
            .value(key)
            .and_then(toml::Value::as_integer)
            .ok_or_else(|| invalid(key, "expected an integer"))?;
        u32::try_from(n)
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| invalid(key, "expected an integer in 1..=4294967295"))
    }

    fn opt_u64(&self, key: &str) -> Result<Option<u64>, ConfigError> {
        match self.value(key) {
            None => Ok(None),
            Some(v) => v
                .as_integer()
                .and_then(|n| u64::try_from(n).ok())
                .map(Some)
                .ok_or_else(|| invalid(key, "expected a non-negative integer")),
        }
    }

    /// Exact decimal text: a decimal string or a non-negative integer. TOML floats are rejected
    /// because they have already been rounded to binary.
    fn opt_decimal(&self, key: &str) -> Result<Option<String>, ConfigError> {
        match self.value(key) {
            None => Ok(None),
            Some(toml::Value::String(text)) if is_decimal(text) => Ok(Some(text.clone())),
            Some(toml::Value::Integer(n)) if *n >= 0 => Ok(Some(n.to_string())),
            Some(_) => Err(invalid(
                key,
                "expected a decimal string such as \"5.25\" or an integer",
            )),
        }
    }

    fn choice<T: Copy>(&self, key: &str, table: &[(&str, T)]) -> Result<T, ConfigError> {
        let word = self.str(key)?;
        table
            .iter()
            .find(|(name, _)| *name == word)
            .map(|(_, v)| *v)
            .ok_or_else(|| {
                let names: Vec<&str> = table.iter().map(|(n, _)| *n).collect();
                invalid(key, format!("expected one of {}", names.join(", ")))
            })
    }

    fn model(&self, key: &str) -> Result<ModelId, ConfigError> {
        ModelId::new(self.str(key)?).map_err(|e| invalid(key, e.to_string()))
    }

    fn secret(&self, key: &str) -> Result<Option<SecretRef>, ConfigError> {
        self.opt_str(key)?
            .map(|v| SecretRef::parse_for(key, v))
            .transpose()
    }
}

fn is_http_url(url: &str) -> bool {
    url.starts_with("https://")
        || url.starts_with("http://localhost")
        || url.starts_with("http://127.0.0.1")
}

fn is_decimal(text: &str) -> bool {
    let mut parts = text.splitn(2, '.');
    let int_ok = parts
        .next()
        .is_some_and(|i| !i.is_empty() && i.bytes().all(|b| b.is_ascii_digit()));
    let frac_ok = parts
        .next()
        .is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()));
    int_ok && frac_ok
}

/// Builds the typed configuration from a fully merged table (defaults included).
pub(crate) fn build(table: &toml::Table) -> Result<PolarisConfig, ConfigError> {
    let r = Reader(table);
    let base_url = r.str("nebius.base_url")?;
    if !is_http_url(base_url) || !base_url.ends_with('/') {
        return Err(invalid(
            "nebius.base_url",
            "expected an https:// (or local http://) URL ending with /",
        ));
    }
    let searxng_url = r.opt_str("search.searxng_url")?;
    if searxng_url.is_some_and(|u| !(u.starts_with("http://") || u.starts_with("https://"))) {
        return Err(invalid("search.searxng_url", "expected an http(s) URL"));
    }
    Ok(PolarisConfig {
        ratelimit: RateLimitConfig {
            rpm: r.positive_u32("ratelimit.rpm")?,
            tpm: r.positive_u32("ratelimit.tpm")?,
        },
        concurrency: ConcurrencyConfig {
            inference: r.positive_u32("concurrency.inference")?,
            web: r.positive_u32("concurrency.web")?,
            docs: r.positive_u32("concurrency.docs")?,
            disk: r.positive_u32("concurrency.disk")?,
            s1: r.positive_u32("concurrency.s1")?,
            sandbox: r.positive_u32("concurrency.sandbox")?,
        },
        storage: StorageConfig {
            level: r.choice(
                "storage.level",
                &[
                    ("global", StorageLevel::Global),
                    ("isolated", StorageLevel::Isolated),
                ],
            )?,
        },
        sandbox: SandboxConfig {
            tier: r.choice(
                "sandbox.tier",
                &[
                    ("auto", SandboxTier::Auto),
                    ("openshell", SandboxTier::Openshell),
                    ("microvm", SandboxTier::Microvm),
                    ("contree", SandboxTier::Contree),
                    ("remote", SandboxTier::Remote),
                ],
            )?,
        },
        nebius: NebiusConfig {
            base_url: base_url.to_owned(),
            api_key: r.secret("nebius.api_key")?,
            effort: EffortModels {
                low: r.model("nebius.effort.low")?,
                medium: r.model("nebius.effort.medium")?,
                high: r.model("nebius.effort.high")?,
                xhigh: r.model("nebius.effort.xhigh")?,
                max: r.model("nebius.effort.max")?,
            },
        },
        search: SearchConfig {
            tavily_api_key: r.secret("search.tavily_api_key")?,
            searxng_url: searxng_url.map(str::to_owned),
        },
        log: LogConfig {
            level: r.choice(
                "log.level",
                &[
                    ("error", LogLevel::Error),
                    ("warn", LogLevel::Warn),
                    ("info", LogLevel::Info),
                    ("debug", LogLevel::Debug),
                    ("trace", LogLevel::Trace),
                ],
            )?,
        },
        budget: BudgetConfig {
            usd: r.opt_decimal("budget.usd")?,
            tokens: r.opt_u64("budget.tokens")?,
        },
        ui: UiConfig {
            theme: r.choice(
                "ui.theme",
                &[("dark", Theme::Dark), ("light", Theme::Light)],
            )?,
        },
    })
}
