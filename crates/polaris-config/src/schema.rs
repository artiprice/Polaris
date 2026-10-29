//! The typed configuration and its defaults.

use polaris_sdk::effort::Effort;
use polaris_sdk::ids::ModelId;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{ConfigError, invalid};
use crate::layer::lookup;
use crate::secret_ref::SecretRef;
use crate::sources::Dirs;

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

[provider]
order = ["token_factory", "openrouter", "local"]

[nebius]
base_url = "https://api.tokenfactory.nebius.com/v1/"
api_key = "env:NEBIUS_API_KEY"
context_window = 262144

[nebius.effort]
low = "nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B"
medium = "nvidia/Nemotron-3_5-Lightning"
high = "nvidia/nemotron-3-super-120b-a12b"
xhigh = "nvidia/nemotron-3-super-120b-a12b"
max = "nvidia/Nemotron-3-Ultra-550b-a55b"

[openrouter]
base_url = "https://openrouter.ai/api/v1/"
api_key = "env:OPENROUTER_API_KEY"
context_window = 262144

[openrouter.effort]
low = "nvidia/nemotron-3.5-lightning:free"
medium = "nvidia/nemotron-3.5-lightning:free"
high = "nvidia/nemotron-3-ultra-550b-a55b:free"
xhigh = "nvidia/nemotron-3-ultra-550b-a55b:free"
max = "nvidia/nemotron-3-ultra-550b-a55b:free"

[local]
base_url = "http://127.0.0.1:11434/v1/"
context_window = 8192

[search]

[log]
level = "info"

[budget]

[ui]
theme = "dark"

[ui.cli]
theme = "dark"
color = "auto"
color_depth = "auto"
ascii = false
reduce_motion = false
panel = "closed"
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
    "provider.order",
    "nebius.base_url",
    "nebius.api_key",
    "nebius.context_window",
    "nebius.effort.low",
    "nebius.effort.medium",
    "nebius.effort.high",
    "nebius.effort.xhigh",
    "nebius.effort.max",
    "openrouter.base_url",
    "openrouter.api_key",
    "openrouter.context_window",
    "openrouter.effort.low",
    "openrouter.effort.medium",
    "openrouter.effort.high",
    "openrouter.effort.xhigh",
    "openrouter.effort.max",
    "local.base_url",
    "local.model",
    "local.context_window",
    "system_one.worker",
    "system_one.decide_model_dir",
    "system_one.pii_model_dir",
    "system_one.ort_dylib",
    "search.tavily_api_key",
    "search.searxng_url",
    "log.level",
    "budget.usd",
    "budget.tokens",
    "ui.theme",
    "ui.cli.theme",
    "ui.cli.color",
    "ui.cli.color_depth",
    "ui.cli.ascii",
    "ui.cli.reduce_motion",
    "ui.cli.panel",
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
    /// Context capacity assumed for prompt fitting (input plus reserved output).
    pub context_tokens: u32,
    /// Effort → model map.
    pub effort: EffortModels,
}

/// A model provider in the selection chain (`provider.order`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderName {
    /// Nebius Token Factory (`[nebius]`).
    TokenFactory,
    /// OpenRouter (`[openrouter]`).
    OpenRouter,
    /// A local OpenAI-compatible server such as Ollama or llama.cpp (`[local]`).
    Local,
}

impl ProviderName {
    /// The configuration spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TokenFactory => "token_factory",
            Self::OpenRouter => "openrouter",
            Self::Local => "local",
        }
    }
}

/// Local System One (GLiNER) worker paths. S1 runs only when the decision model and ONNX Runtime
/// are configured; otherwise frames run in degraded mode. Paths are host-local and are never
/// accepted from an untrusted repository layer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SystemOneConfig {
    /// `polaris-worker` executable; when unset, the binary next to the running executable is used.
    pub worker: Option<std::path::PathBuf>,
    /// fp32 GLiNER2.5-Decide fragment directory.
    pub decide_model_dir: Option<std::path::PathBuf>,
    /// Optional fp32 GLiNER2 PII companion fragment directory.
    pub pii_model_dir: Option<std::path::PathBuf>,
    /// ONNX Runtime shared library (1.23 or newer).
    pub ort_dylib: Option<std::path::PathBuf>,
}

/// Provider selection: the first usable provider in `order` serves the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderConfig {
    /// Ordered, duplicate-free candidates; an empty list means offline.
    pub order: Vec<ProviderName>,
}

/// A hosted OpenAI-compatible provider reached with a bearer key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedProviderConfig {
    /// HTTPS base URL (ends with `/`).
    pub base_url: String,
    /// API key reference; the provider is skipped when it is absent or resolves to nothing.
    pub api_key: Option<SecretRef>,
    /// Context capacity assumed for prompt fitting (input plus reserved output).
    pub context_tokens: u32,
    /// Effort → model map.
    pub effort: EffortModels,
}

/// A local OpenAI-compatible server on a literal loopback address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalProviderConfig {
    /// `http://127.0.0.1:<port>/…/` base URL (ends with `/`).
    pub base_url: String,
    /// Served model name; the provider is skipped while this is unset.
    pub model: Option<ModelId>,
    /// Context capacity configured on the server.
    pub context_tokens: u32,
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
    /// Independent terminal-client preferences.
    pub cli: CliPreferences,
}

/// CLI canvas theme, independent of the legacy `ui.theme` setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliTheme {
    /// Black application canvas.
    Dark,
    /// Inherit terminal colors.
    Terminal,
}

/// CLI color policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliColorPolicy {
    /// Detect terminal support and `NO_COLOR`.
    Auto,
    /// Always emit the selected colors.
    Always,
    /// Emit no application colors.
    Never,
}

/// CLI color depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliColorDepth {
    /// Detect terminal truecolor support.
    Auto,
    /// Use RGB colors.
    Truecolor,
    /// Use the ANSI palette.
    Ansi,
}

/// Default panel visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliPanelDefault {
    /// Start with the panel open.
    Open,
    /// Start with the panel closed.
    Closed,
}

/// Complete CLI UI preference set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliPreferences {
    /// Canvas theme.
    pub theme: CliTheme,
    /// Whether to render application colors.
    pub color: CliColorPolicy,
    /// Color palette depth.
    pub color_depth: CliColorDepth,
    /// Use ASCII-only drawing characters.
    pub ascii: bool,
    /// Reduce or disable nonessential motion.
    pub reduce_motion: bool,
    /// Initial panel visibility.
    pub panel: CliPanelDefault,
}

impl Default for CliPreferences {
    fn default() -> Self {
        Self {
            theme: CliTheme::Dark,
            color: CliColorPolicy::Auto,
            color_depth: CliColorDepth::Auto,
            ascii: false,
            reduce_motion: false,
            panel: CliPanelDefault::Closed,
        }
    }
}

/// One persisted global CLI preference field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferenceField {
    /// `ui.cli.theme`.
    Theme,
    /// `ui.cli.color`.
    Color,
    /// `ui.cli.color_depth`.
    ColorDepth,
    /// `ui.cli.ascii`.
    Ascii,
    /// `ui.cli.reduce_motion`.
    ReduceMotion,
    /// `ui.cli.panel`.
    Panel,
}

/// Typed value accepted when persisting one CLI preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferenceValue {
    /// CLI theme value.
    Theme(CliTheme),
    /// CLI color policy value.
    Color(CliColorPolicy),
    /// CLI color depth value.
    ColorDepth(CliColorDepth),
    /// Boolean preference value.
    Boolean(bool),
    /// CLI initial panel visibility.
    Panel(CliPanelDefault),
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
    /// Model provider selection chain.
    pub provider: ProviderConfig,
    /// Nebius Token Factory.
    pub nebius: NebiusConfig,
    /// OpenRouter.
    pub openrouter: HostedProviderConfig,
    /// Local OpenAI-compatible server.
    pub local: LocalProviderConfig,
    /// Local System One worker.
    pub system_one: SystemOneConfig,
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

    fn bool(&self, key: &str) -> Result<bool, ConfigError> {
        self.value(key)
            .and_then(toml::Value::as_bool)
            .ok_or_else(|| invalid(key, "expected a boolean"))
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

    fn opt_path(&self, key: &str) -> Result<Option<std::path::PathBuf>, ConfigError> {
        match self.opt_str(key)? {
            None => Ok(None),
            Some(value) => {
                let path = std::path::PathBuf::from(value);
                if path.is_absolute() {
                    Ok(Some(path))
                } else {
                    Err(invalid(key, "expected an absolute path"))
                }
            }
        }
    }

    fn opt_model(&self, key: &str) -> Result<Option<ModelId>, ConfigError> {
        self.opt_str(key)?
            .map(|value| ModelId::new(value).map_err(|e| invalid(key, e.to_string())))
            .transpose()
    }

    fn effort_models(&self, prefix: &str) -> Result<EffortModels, ConfigError> {
        Ok(EffortModels {
            low: self.model(&format!("{prefix}.effort.low"))?,
            medium: self.model(&format!("{prefix}.effort.medium"))?,
            high: self.model(&format!("{prefix}.effort.high"))?,
            xhigh: self.model(&format!("{prefix}.effort.xhigh"))?,
            max: self.model(&format!("{prefix}.effort.max"))?,
        })
    }

    fn provider_order(&self, key: &str) -> Result<Vec<ProviderName>, ConfigError> {
        let items = self
            .value(key)
            .and_then(toml::Value::as_array)
            .ok_or_else(|| invalid(key, "expected an array of provider names"))?;
        let mut order = Vec::with_capacity(items.len());
        for item in items {
            let name = match item.as_str() {
                Some("token_factory") => ProviderName::TokenFactory,
                Some("openrouter") => ProviderName::OpenRouter,
                Some("local") => ProviderName::Local,
                _ => {
                    return Err(invalid(
                        key,
                        "expected only token_factory, openrouter or local",
                    ));
                }
            };
            if order.contains(&name) {
                return Err(invalid(key, "provider names must not repeat"));
            }
            order.push(name);
        }
        Ok(order)
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
    let openrouter_url = r.str("openrouter.base_url")?;
    if !openrouter_url.starts_with("https://") || !openrouter_url.ends_with('/') {
        return Err(invalid(
            "openrouter.base_url",
            "expected an https:// URL ending with /",
        ));
    }
    let local_url = r.str("local.base_url")?;
    if !(local_url.starts_with("http://127.0.0.1:") || local_url.starts_with("http://[::1]:"))
        || !local_url.ends_with('/')
    {
        return Err(invalid(
            "local.base_url",
            "expected an http://127.0.0.1:<port>/ or http://[::1]:<port>/ URL ending with /",
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
        provider: ProviderConfig {
            order: r.provider_order("provider.order")?,
        },
        nebius: NebiusConfig {
            base_url: base_url.to_owned(),
            api_key: r.secret("nebius.api_key")?,
            context_tokens: r.positive_u32("nebius.context_window")?,
            effort: r.effort_models("nebius")?,
        },
        openrouter: HostedProviderConfig {
            base_url: openrouter_url.to_owned(),
            api_key: r.secret("openrouter.api_key")?,
            context_tokens: r.positive_u32("openrouter.context_window")?,
            effort: r.effort_models("openrouter")?,
        },
        local: LocalProviderConfig {
            base_url: local_url.to_owned(),
            model: r.opt_model("local.model")?,
            context_tokens: r.positive_u32("local.context_window")?,
        },
        system_one: SystemOneConfig {
            worker: r.opt_path("system_one.worker")?,
            decide_model_dir: r.opt_path("system_one.decide_model_dir")?,
            pii_model_dir: r.opt_path("system_one.pii_model_dir")?,
            ort_dylib: r.opt_path("system_one.ort_dylib")?,
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
            cli: CliPreferences {
                theme: r.choice(
                    "ui.cli.theme",
                    &[("dark", CliTheme::Dark), ("terminal", CliTheme::Terminal)],
                )?,
                color: r.choice(
                    "ui.cli.color",
                    &[
                        ("auto", CliColorPolicy::Auto),
                        ("always", CliColorPolicy::Always),
                        ("never", CliColorPolicy::Never),
                    ],
                )?,
                color_depth: r.choice(
                    "ui.cli.color_depth",
                    &[
                        ("auto", CliColorDepth::Auto),
                        ("truecolor", CliColorDepth::Truecolor),
                        ("ansi", CliColorDepth::Ansi),
                    ],
                )?,
                ascii: r.bool("ui.cli.ascii")?,
                reduce_motion: r.bool("ui.cli.reduce_motion")?,
                panel: r.choice(
                    "ui.cli.panel",
                    &[
                        ("open", CliPanelDefault::Open),
                        ("closed", CliPanelDefault::Closed),
                    ],
                )?,
            },
        },
    })
}

/// Read only the persisted global CLI preference table. Unknown sibling config keys are ignored.
///
/// # Errors
/// Returns an I/O, size, parse, or preference validation error.
pub fn read_global_cli_preferences(dirs: &Dirs) -> Result<CliPreferences, ConfigError> {
    let path = dirs.global.join("config.toml");
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CliPreferences::default());
        }
        Err(error) => return Err(config_io(&path, error)),
    };
    let limit = u64::try_from(crate::MAX_CONFIG_FILE_BYTES)
        .map_err(|_| invalid("ui.cli", "configuration size limit is invalid"))?;
    let mut text = String::new();
    file.take(limit.saturating_add(1))
        .read_to_string(&mut text)
        .map_err(|error| config_io(&path, error))?;
    if text.len() > crate::MAX_CONFIG_FILE_BYTES {
        return Err(ConfigError::FileTooLarge {
            path,
            limit: crate::MAX_CONFIG_FILE_BYTES,
        });
    }
    let table = toml::from_str::<toml::Table>(&text).map_err(|error| ConfigError::Parse {
        file: path.clone(),
        line: line_number(error.span().and_then(|span| text.get(..span.start))),
        reason: error.message().to_owned(),
    })?;
    let value = lookup_value(&table, "ui.cli");
    let Some(preferences) = value.and_then(toml::Value::as_table) else {
        if value.is_some() {
            return Err(invalid("ui.cli", "expected a table"));
        }
        return Ok(CliPreferences::default());
    };
    Ok(CliPreferences {
        theme: pref_choice(
            preferences,
            "theme",
            "ui.cli.theme",
            CliTheme::Dark,
            &[("dark", CliTheme::Dark), ("terminal", CliTheme::Terminal)],
        )?,
        color: pref_choice(
            preferences,
            "color",
            "ui.cli.color",
            CliColorPolicy::Auto,
            &[
                ("auto", CliColorPolicy::Auto),
                ("always", CliColorPolicy::Always),
                ("never", CliColorPolicy::Never),
            ],
        )?,
        color_depth: pref_choice(
            preferences,
            "color_depth",
            "ui.cli.color_depth",
            CliColorDepth::Auto,
            &[
                ("auto", CliColorDepth::Auto),
                ("truecolor", CliColorDepth::Truecolor),
                ("ansi", CliColorDepth::Ansi),
            ],
        )?,
        ascii: pref_bool(preferences, "ascii", "ui.cli.ascii")?.is_some_and(|value| value),
        reduce_motion: pref_bool(preferences, "reduce_motion", "ui.cli.reduce_motion")?
            .is_some_and(|value| value),
        panel: pref_choice(
            preferences,
            "panel",
            "ui.cli.panel",
            CliPanelDefault::Closed,
            &[
                ("open", CliPanelDefault::Open),
                ("closed", CliPanelDefault::Closed),
            ],
        )?,
    })
}

/// Persist one preference in `dirs.global/config.toml`, preserving all other TOML keys.
/// Only [`Dirs::global`] is used. Repository and session overrides remain memory-only and are not
/// accepted by this API.
///
/// # Errors
/// Returns an I/O, size, parse, encoding, or field/value mismatch error. A failed write leaves
/// the existing config file untouched.
pub fn save_global_cli_preference(
    dirs: &Dirs,
    field: PreferenceField,
    value: PreferenceValue,
) -> Result<(), ConfigError> {
    let path = dirs.global.join("config.toml");
    let original = read_config_bytes(&path)?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let source = std::str::from_utf8(&original).map_err(|_| ConfigError::Parse {
        file: path.clone(),
        line: 1,
        reason: "configuration is not UTF-8".to_owned(),
    })?;
    let mut table = toml::from_str::<toml::Table>(source).map_err(|error| ConfigError::Parse {
        file: path.clone(),
        line: line_number(error.span().and_then(|span| source.get(..span.start))),
        reason: error.message().to_owned(),
    })?;
    let (key, toml_value) = preference_value(field, value)?;
    if !crate::layer::insert_path(&mut table, key, toml_value) {
        return Err(invalid(key, "conflicts with an existing TOML value"));
    }
    let encoded =
        toml::to_string(&table).map_err(|error| ConfigError::Encode(error.to_string()))?;
    if encoded.len() > crate::MAX_CONFIG_FILE_BYTES {
        return Err(ConfigError::FileTooLarge {
            path,
            limit: crate::MAX_CONFIG_FILE_BYTES,
        });
    }
    atomic_replace(&path, encoded.as_bytes())
}

fn preference_value(
    field: PreferenceField,
    value: PreferenceValue,
) -> Result<(&'static str, toml::Value), ConfigError> {
    let pair = match (field, value) {
        (PreferenceField::Theme, PreferenceValue::Theme(CliTheme::Dark)) => {
            ("ui.cli.theme", "dark")
        }
        (PreferenceField::Theme, PreferenceValue::Theme(CliTheme::Terminal)) => {
            ("ui.cli.theme", "terminal")
        }
        (PreferenceField::Color, PreferenceValue::Color(CliColorPolicy::Auto)) => {
            ("ui.cli.color", "auto")
        }
        (PreferenceField::Color, PreferenceValue::Color(CliColorPolicy::Always)) => {
            ("ui.cli.color", "always")
        }
        (PreferenceField::Color, PreferenceValue::Color(CliColorPolicy::Never)) => {
            ("ui.cli.color", "never")
        }
        (PreferenceField::ColorDepth, PreferenceValue::ColorDepth(CliColorDepth::Auto)) => {
            ("ui.cli.color_depth", "auto")
        }
        (PreferenceField::ColorDepth, PreferenceValue::ColorDepth(CliColorDepth::Truecolor)) => {
            ("ui.cli.color_depth", "truecolor")
        }
        (PreferenceField::ColorDepth, PreferenceValue::ColorDepth(CliColorDepth::Ansi)) => {
            ("ui.cli.color_depth", "ansi")
        }
        (PreferenceField::Ascii, PreferenceValue::Boolean(value)) => {
            return Ok(("ui.cli.ascii", toml::Value::Boolean(value)));
        }
        (PreferenceField::ReduceMotion, PreferenceValue::Boolean(value)) => {
            return Ok(("ui.cli.reduce_motion", toml::Value::Boolean(value)));
        }
        (PreferenceField::Panel, PreferenceValue::Panel(CliPanelDefault::Open)) => {
            ("ui.cli.panel", "open")
        }
        (PreferenceField::Panel, PreferenceValue::Panel(CliPanelDefault::Closed)) => {
            ("ui.cli.panel", "closed")
        }
        _ => return Err(invalid("ui.cli", "preference value does not match field")),
    };
    Ok((pair.0, toml::Value::String(pair.1.to_owned())))
}

fn lookup_value<'a>(table: &'a toml::Table, path: &str) -> Option<&'a toml::Value> {
    let mut pieces = path.split('.');
    let first = table.get(pieces.next()?)?;
    pieces.try_fold(first, |value, piece| value.as_table()?.get(piece))
}

fn pref_choice<T: Copy>(
    table: &toml::Table,
    key: &str,
    full_key: &str,
    default: T,
    choices: &[(&str, T)],
) -> Result<T, ConfigError> {
    let Some(value) = table.get(key) else {
        return Ok(default);
    };
    let Some(word) = value.as_str() else {
        return Err(invalid(full_key, "expected a string"));
    };
    choices
        .iter()
        .find(|choice| choice.0 == word)
        .map(|choice| choice.1)
        .ok_or_else(|| {
            invalid(
                full_key,
                format!(
                    "expected one of {}",
                    choices
                        .iter()
                        .map(|choice| choice.0)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        })
}

fn pref_bool(table: &toml::Table, key: &str, full_key: &str) -> Result<Option<bool>, ConfigError> {
    match table.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| invalid(full_key, "expected a boolean")),
    }
}

fn config_io(path: &Path, error: std::io::Error) -> ConfigError {
    ConfigError::Io {
        path: path.to_path_buf(),
        reason: error.to_string(),
    }
}

fn line_number(before: Option<&str>) -> u32 {
    before.map_or(1, |text| {
        text.bytes()
            .filter(|byte| *byte == b'\n')
            .fold(1_u32, |line, _| line.saturating_add(1))
    })
}

fn read_config_bytes(path: &Path) -> Result<Option<Vec<u8>>, ConfigError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(config_io(path, error)),
    };
    let limit = u64::try_from(crate::MAX_CONFIG_FILE_BYTES)
        .map_err(|_| invalid("ui.cli", "configuration size limit is invalid"))?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| config_io(path, error))?;
    if bytes.len() > crate::MAX_CONFIG_FILE_BYTES {
        return Err(ConfigError::FileTooLarge {
            path: path.to_path_buf(),
            limit: crate::MAX_CONFIG_FILE_BYTES,
        });
    }
    Ok(Some(bytes))
}

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("ui.cli", "global config path has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| config_io(parent, error))?;
    let mut attempts = 0_u8;
    loop {
        attempts = attempts.saturating_add(1);
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!("config.toml.tmp.{}.{}", std::process::id(), sequence);
        let temp = parent.join(name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = match options.open(&temp) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempts < 8 => {
                continue;
            }
            Err(error) => return Err(config_io(&temp, error)),
        };
        let write_result = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temp);
            return Err(config_io(&temp, error));
        }
        match fs::rename(&temp, path) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let _ = fs::remove_file(&temp);
                return Err(config_io(path, error));
            }
        }
    }
}
