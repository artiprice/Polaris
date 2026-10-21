//! Typed session directives (PLAN directive catalog) and their conversion from parsed arguments.

pub use crate::args::Arg;
use crate::effort::model_efforts_from_args;
use crate::error::DirectiveError;
use polaris_sdk::effort::Effort;

/// On / off switch (a bare `$name` means on).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Toggle {
    /// Enabled.
    On,
    /// Disabled.
    Off,
}

/// A switch that can also carry a list (`$links(a, b)`, `$mcp(server)`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ListArg {
    /// Enabled with defaults.
    On,
    /// Disabled.
    Off,
    /// Enabled for these items.
    Items(Vec<String>),
}

/// `$nvidia` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NvidiaMode {
    /// Detect the NVIDIA stack and load matching advisor packs.
    Auto,
    /// List detected components only.
    List,
    /// Disabled.
    Off,
}

/// `$nvidia-docs` argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NvidiaDocsArg {
    /// Synchronise the local NVIDIA docs index.
    Sync,
}

/// `$sandbox` / `$nebius-sandbox` execution tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SandboxMode {
    /// Choose per task.
    Auto,
    /// OpenShell gateway (WSL2, docker driver).
    Openshell,
    /// OpenShell vm driver (libkrun).
    Microvm,
    /// Token Factory Sandboxes (Contree).
    Contree,
    /// Remote OpenShell gateway on Nebius AI Cloud.
    Remote,
}

/// `$concurrency` pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pool {
    /// Model inference.
    Inference,
    /// Web requests.
    Web,
    /// Document parsing.
    Docs,
    /// Disk I/O (`io` is an alias).
    Disk,
    /// System-One worker.
    S1,
    /// Sandbox operations.
    Sandbox,
}

/// `$budget` limit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Budget {
    /// US dollars as a decimal string (parsed exactly by telemetry).
    Usd(String),
    /// Total tokens.
    Tokens(u64),
}

/// `$storage-level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageLevel {
    /// Shared `~/.polaris` storage.
    Global,
    /// Workspace-local `.polaris` storage.
    Isolated,
}

/// `$env` execution environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvKind {
    /// Nix flake.
    Nix,
    /// Docker image.
    Docker,
    /// The host toolchain.
    Host,
}

/// `$ci` backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CiBackend {
    /// Dagger pipelines.
    Dagger,
}

/// One `$subagent-models` entry: a model with an effort range.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelEffort {
    /// Model id (validated later by `polaris-models`).
    pub model: String,
    /// Lowest effort.
    pub lo: Effort,
    /// Highest effort (`lo <= hi`).
    pub hi: Effort,
}

/// A session-scoped directive (D11: applies from where it appears; persisted for resume).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SessionDirective {
    /// `$websearch`.
    WebSearch(Toggle),
    /// `$subagents`.
    Subagents(Toggle),
    /// `$subagent-models(model(lo-hi), …)`.
    SubagentModels(Vec<ModelEffort>),
    /// `$links`.
    Links(ListArg),
    /// `$skills`.
    Skills(ListArg),
    /// `$mcp`.
    Mcp(ListArg),
    /// `$rag`.
    Rag(ListArg),
    /// `$nvidia`.
    Nvidia(NvidiaMode),
    /// `$nvidia-docs(sync)`.
    NvidiaDocs(NvidiaDocsArg),
    /// `$sandbox` (alias `$nebius-sandbox`).
    Sandbox(SandboxMode),
    /// `$nebius-branching(on, max_branches=N)`.
    NebiusBranching {
        /// Branching enabled.
        on: bool,
        /// Upper bound on parallel branches.
        max_branches: Option<u8>,
    },
    /// `$nebius-datalab`.
    NebiusDatalab(Toggle),
    /// `$nebius-speculative` (cascade drafting).
    NebiusSpeculative(Toggle),
    /// `$concurrency(pool=N, …)`.
    Concurrency(Vec<(Pool, u32)>),
    /// `$ratelimit(rpm=N, tpm=N)`.
    RateLimit {
        /// Requests per minute.
        rpm: Option<u32>,
        /// Tokens per minute.
        tpm: Option<u32>,
    },
    /// `$budget(usd=…)` / `$budget(tokens=…)`.
    Budget(Budget),
    /// `$storage-level`.
    StorageLevel(StorageLevel),
    /// `$browser`.
    Browser(Toggle),
    /// `$needle`.
    Needle(Toggle),
    /// `$env`.
    Env(EnvKind),
    /// `$ci`.
    Ci(CiBackend),
    /// Any other `$name(args)`: kept as session metadata, never an error.
    Custom {
        /// Directive name.
        name: String,
        /// Arguments as parsed.
        args: Vec<Arg>,
    },
}

impl SessionDirective {
    /// Canonical name (the state key): `websearch`, `sandbox`, …, or the custom name.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::WebSearch(_) => "websearch",
            Self::Subagents(_) => "subagents",
            Self::SubagentModels(_) => "subagent-models",
            Self::Links(_) => "links",
            Self::Skills(_) => "skills",
            Self::Mcp(_) => "mcp",
            Self::Rag(_) => "rag",
            Self::Nvidia(_) => "nvidia",
            Self::NvidiaDocs(_) => "nvidia-docs",
            Self::Sandbox(_) => "sandbox",
            Self::NebiusBranching { .. } => "nebius-branching",
            Self::NebiusDatalab(_) => "nebius-datalab",
            Self::NebiusSpeculative(_) => "nebius-speculative",
            Self::Concurrency(_) => "concurrency",
            Self::RateLimit { .. } => "ratelimit",
            Self::Budget(_) => "budget",
            Self::StorageLevel(_) => "storage-level",
            Self::Browser(_) => "browser",
            Self::Needle(_) => "needle",
            Self::Env(_) => "env",
            Self::Ci(_) => "ci",
            Self::Custom { name, .. } => name,
        }
    }
}

/// Context for typed conversion errors.
struct Ctx<'n> {
    name: &'n str,
    at: usize,
}

impl Ctx<'_> {
    fn bad(&self, reason: &'static str) -> DirectiveError {
        DirectiveError::BadArg {
            directive: format!("${}", self.name),
            at: self.at,
            reason,
        }
    }
}

/// The single lowercase word of `(word)`, if that is all the arguments are.
fn single_word(args: &[Arg]) -> Option<String> {
    match args {
        [Arg::Atom(word)] => Some(word.to_ascii_lowercase()),
        _ => None,
    }
}

fn toggle(ctx: &Ctx<'_>, args: Option<&[Arg]>) -> Result<Toggle, DirectiveError> {
    match args {
        None => Ok(Toggle::On),
        Some(args) => match single_word(args).as_deref() {
            Some("on") => Ok(Toggle::On),
            Some("off") => Ok(Toggle::Off),
            _ => Err(ctx.bad("expected (on) or (off)")),
        },
    }
}

fn list(ctx: &Ctx<'_>, args: Option<&[Arg]>) -> Result<ListArg, DirectiveError> {
    let Some(args) = args else {
        return Ok(ListArg::On);
    };
    match single_word(args).as_deref() {
        Some("on") => return Ok(ListArg::On),
        Some("off") => return Ok(ListArg::Off),
        _ => {}
    }
    if args.is_empty() {
        return Err(ctx.bad("expected (on), (off) or a list of items"));
    }
    let items = args
        .iter()
        .map(|a| a.text().map(str::to_owned))
        .collect::<Option<Vec<_>>>();
    items
        .map(ListArg::Items)
        .ok_or_else(|| ctx.bad("list items must be words or quoted strings"))
}

fn one_of<T: Copy>(
    ctx: &Ctx<'_>,
    args: Option<&[Arg]>,
    bare: Option<T>,
    table: &[(&str, T)],
    reason: &'static str,
) -> Result<T, DirectiveError> {
    match args {
        None => bare.ok_or_else(|| ctx.bad(reason)),
        Some(args) => {
            let word = single_word(args).ok_or_else(|| ctx.bad(reason))?;
            table
                .iter()
                .find(|(name, _)| *name == word)
                .map(|(_, v)| *v)
                .ok_or_else(|| ctx.bad(reason))
        }
    }
}

fn number<T: core::str::FromStr>(
    ctx: &Ctx<'_>,
    arg: &Arg,
    reason: &'static str,
) -> Result<T, DirectiveError> {
    match arg {
        Arg::Atom(text) if text.bytes().all(|b| b.is_ascii_digit()) => {
            text.parse().map_err(|_| ctx.bad(reason))
        }
        _ => Err(ctx.bad(reason)),
    }
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

fn branching(ctx: &Ctx<'_>, args: Option<&[Arg]>) -> Result<SessionDirective, DirectiveError> {
    let mut on = true;
    let mut max_branches = None;
    for arg in args.unwrap_or(&[]) {
        match arg {
            Arg::Atom(w) if w.eq_ignore_ascii_case("on") => on = true,
            Arg::Atom(w) if w.eq_ignore_ascii_case("off") => on = false,
            Arg::Kv { key, value } if key == "max_branches" => {
                max_branches = Some(number(ctx, value, "max_branches must be 0-255")?)
            }
            _ => return Err(ctx.bad("expected (on|off, max_branches=N)")),
        }
    }
    Ok(SessionDirective::NebiusBranching { on, max_branches })
}

const POOLS: &[(&str, Pool)] = &[
    ("inference", Pool::Inference),
    ("web", Pool::Web),
    ("docs", Pool::Docs),
    ("disk", Pool::Disk),
    ("io", Pool::Disk),
    ("s1", Pool::S1),
    ("sandbox", Pool::Sandbox),
];

fn concurrency(ctx: &Ctx<'_>, args: Option<&[Arg]>) -> Result<SessionDirective, DirectiveError> {
    let args = args.unwrap_or(&[]);
    if args.is_empty() {
        return Err(ctx.bad("expected (pool=N, …)"));
    }
    let mut pools = Vec::with_capacity(args.len());
    for arg in args {
        let Arg::Kv { key, value } = arg else {
            return Err(ctx.bad("expected pool=N"));
        };
        let pool = POOLS
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, p)| *p)
            .ok_or_else(|| ctx.bad("unknown pool (inference, web, docs, disk|io, s1, sandbox)"))?;
        pools.push((
            pool,
            number(ctx, value, "pool size must be a non-negative integer")?,
        ));
    }
    Ok(SessionDirective::Concurrency(pools))
}

fn ratelimit(ctx: &Ctx<'_>, args: Option<&[Arg]>) -> Result<SessionDirective, DirectiveError> {
    let (mut rpm, mut tpm) = (None, None);
    for (i, arg) in args.unwrap_or(&[]).iter().enumerate() {
        match arg {
            Arg::Kv { key, value } if key == "rpm" => {
                rpm = Some(number(ctx, value, "rpm must be an integer")?)
            }
            Arg::Kv { key, value } if key == "tpm" => {
                tpm = Some(number(ctx, value, "tpm must be an integer")?)
            }
            Arg::Atom(_) if i == 0 => rpm = Some(number(ctx, arg, "rpm must be an integer")?),
            Arg::Atom(_) if i == 1 => tpm = Some(number(ctx, arg, "tpm must be an integer")?),
            _ => return Err(ctx.bad("expected (rpm=N, tpm=N) or (N, N)")),
        }
    }
    Ok(SessionDirective::RateLimit { rpm, tpm })
}

fn budget(ctx: &Ctx<'_>, args: Option<&[Arg]>) -> Result<SessionDirective, DirectiveError> {
    match args {
        Some([Arg::Kv { key, value }]) if key == "usd" => match value.as_ref() {
            Arg::Atom(v) if is_decimal(v) => Ok(SessionDirective::Budget(Budget::Usd(v.clone()))),
            _ => Err(ctx.bad("usd must be a decimal number")),
        },
        Some([Arg::Kv { key, value }]) if key == "tokens" => Ok(SessionDirective::Budget(
            Budget::Tokens(number(ctx, value, "tokens must be an integer")?),
        )),
        _ => Err(ctx.bad("expected (usd=N) or (tokens=N)")),
    }
}

/// Converts a parsed `$name` / `$name(args)` (`args = None` for the bare form) into a typed directive.
pub(crate) fn typed(
    name: &str,
    args: Option<Vec<Arg>>,
    at: usize,
) -> Result<SessionDirective, DirectiveError> {
    let ctx = Ctx { name, at };
    let a = args.as_deref();
    let d = match name {
        "websearch" => SessionDirective::WebSearch(toggle(&ctx, a)?),
        "subagents" => SessionDirective::Subagents(toggle(&ctx, a)?),
        "subagent-models" => {
            let entries = a.ok_or_else(|| ctx.bad("expected (model(lo-hi), …)"))?;
            SessionDirective::SubagentModels(model_efforts_from_args(entries, at)?)
        }
        "links" => SessionDirective::Links(list(&ctx, a)?),
        "skills" => SessionDirective::Skills(list(&ctx, a)?),
        "mcp" => SessionDirective::Mcp(list(&ctx, a)?),
        "rag" => SessionDirective::Rag(list(&ctx, a)?),
        "nvidia" => SessionDirective::Nvidia(one_of(
            &ctx,
            a,
            Some(NvidiaMode::Auto),
            &[
                ("auto", NvidiaMode::Auto),
                ("list", NvidiaMode::List),
                ("off", NvidiaMode::Off),
            ],
            "expected (auto), (list) or (off)",
        )?),
        "nvidia-docs" => SessionDirective::NvidiaDocs(one_of(
            &ctx,
            a,
            None,
            &[("sync", NvidiaDocsArg::Sync)],
            "expected (sync)",
        )?),
        "sandbox" | "nebius-sandbox" => SessionDirective::Sandbox(one_of(
            &ctx,
            a,
            Some(SandboxMode::Auto),
            &[
                ("auto", SandboxMode::Auto),
                ("openshell", SandboxMode::Openshell),
                ("microvm", SandboxMode::Microvm),
                ("contree", SandboxMode::Contree),
                ("remote", SandboxMode::Remote),
            ],
            "expected (auto|openshell|microvm|contree|remote)",
        )?),
        "nebius-branching" => branching(&ctx, a)?,
        "nebius-datalab" => SessionDirective::NebiusDatalab(toggle(&ctx, a)?),
        "nebius-speculative" => SessionDirective::NebiusSpeculative(toggle(&ctx, a)?),
        "concurrency" => concurrency(&ctx, a)?,
        "ratelimit" => ratelimit(&ctx, a)?,
        "budget" => budget(&ctx, a)?,
        "storage-level" => SessionDirective::StorageLevel(one_of(
            &ctx,
            a,
            None,
            &[
                ("global", StorageLevel::Global),
                ("isolated", StorageLevel::Isolated),
            ],
            "expected (global) or (isolated)",
        )?),
        "browser" => SessionDirective::Browser(toggle(&ctx, a)?),
        "needle" => SessionDirective::Needle(toggle(&ctx, a)?),
        "env" => SessionDirective::Env(one_of(
            &ctx,
            a,
            None,
            &[
                ("nix", EnvKind::Nix),
                ("docker", EnvKind::Docker),
                ("host", EnvKind::Host),
            ],
            "expected (nix), (docker) or (host)",
        )?),
        "ci" => SessionDirective::Ci(one_of(
            &ctx,
            a,
            None,
            &[("dagger", CiBackend::Dagger)],
            "expected (dagger)",
        )?),
        other => SessionDirective::Custom {
            name: other.to_owned(),
            args: args.unwrap_or_default(),
        },
    };
    Ok(d)
}
