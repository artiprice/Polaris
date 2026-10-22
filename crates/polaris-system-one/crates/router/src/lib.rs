//! System-One routing advice with bounded scoring and deterministic safe fallbacks.
#![forbid(unsafe_code)]
use polaris_sdk::{
    effort::Effort,
    error::ParseError,
    registry::{DecisionEngine, DecisionError},
    system_one::{Answer, Decision, DecisionInput, Probability},
};
use polaris_system_one_schemas::{Schema, SchemaCatalog, SchemaError, Verdict, apply_threshold};
use std::{collections::BTreeSet, path::PathBuf, time::Duration};
use thiserror::Error;

/// Stable tool identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ToolId(String);
impl ToolId {
    /// Validates a nonempty printable identifier.
    /// # Errors
    /// Returns InvalidCandidate when the identifier is empty or contains controls.
    pub fn new(value: impl Into<String>) -> Result<Self, RouterError> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(RouterError::InvalidCandidate("invalid tool id".into()));
        }
        Ok(Self(value))
    }
    /// Identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// Repository facts used to classify task needs.
#[derive(Debug, Clone, Default)]
pub struct RepoFacts {
    /// Task includes untrusted code.
    pub untrusted_code: bool,
    /// Task needs GUI or browser work.
    pub gui_or_browser: bool,
    /// Task needs vision input.
    pub needs_vision: bool,
    /// Number of known independent work streams.
    pub independent_workstreams: u8,
    /// Task runs a server or persistent service.
    pub runs_server: bool,
}
/// Caller directives applied as hard routing constraints.
#[derive(Debug, Clone, Default)]
pub struct DirectiveSnapshot {
    /// Optional model effort allowlist.
    pub allowed_efforts: Option<Vec<Effort>>,
    /// Exact direct tool to recommend, if separately authorized by caller policy.
    pub direct_tool: Option<ToolId>,
    /// Sandbox directive.
    pub sandbox: Option<SandboxDirective>,
}
/// Task input. Text and goal must already be PII-masked.
#[derive(Debug, Clone)]
pub struct RouteInput {
    /// PII-masked task text.
    pub text: String,
    /// Optional PII-masked top-level goal.
    pub root_goal: Option<String>,
    /// Repository and task facts.
    pub repo_facts: RepoFacts,
    /// Caller constraints.
    pub directives: DirectiveSnapshot,
}
/// Advisory target for a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutePath {
    /// Advisory direct-tool suggestion. Dispatcher must enforce permission.
    DirectTool {
        /// Exact tool identifier.
        tool: ToolId,
    },
    /// Send to S2 reasoning.
    S2,
    /// Ask the user to clarify.
    AskClarification,
}
/// Parallelization advice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnAdvice {
    /// Whether multiple independent branches are useful.
    pub parallel_branches: bool,
    /// Maximum nested delegation depth.
    pub max_depth: u8,
}
/// Source of a routing result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteSource {
    /// Validated S1 output.
    S1,
    /// Deterministic fallback.
    DeterministicFallback,
}
/// Task routing advice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    /// Suggested target; does not grant authority.
    pub path: RoutePath,
    /// Effort constrained by router and caller allowlists.
    pub effort: Effort,
    /// Whether vision is needed.
    pub needs_vision: bool,
    /// Parallelization advice.
    pub spawn: SpawnAdvice,
    /// Confidence score.
    pub confidence: Probability,
    /// Source of the advice.
    pub source: RouteSource,
}
/// Untrusted tool candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCandidate {
    /// Tool identifier.
    pub id: ToolId,
    /// Tool name.
    pub name: String,
    /// Untrusted description.
    pub summary: String,
}
/// File candidate from caller-side search.
#[derive(Debug, Clone, PartialEq)]
pub struct FileCandidate {
    /// Candidate path.
    pub path: PathBuf,
    /// Signature or excerpt.
    pub signature_text: String,
    /// Caller lexical score in 0..=1.
    pub lexical_score: f32,
}
/// Ranking source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankReason {
    /// S1 accepted semantic score.
    Semantic,
    /// Deterministic lexical score used.
    LexicalFallback,
}
/// Ranked item with validated score.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ranked<T> {
    /// Item.
    pub item: T,
    /// Relevance score.
    pub score: Probability,
    /// Ranking basis.
    pub reason: RankReason,
}
/// Nonzero output bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TopK(u16);
impl TopK {
    /// Creates a nonzero bound.
    /// # Errors
    /// Returns TopKZero for zero.
    pub fn new(value: u16) -> Result<Self, RouterError> {
        if value == 0 {
            Err(RouterError::TopKZero)
        } else {
            Ok(Self(value))
        }
    }
    /// Bound value.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}
/// Facts gating sandbox tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SandboxFacts {
    /// Writes files or executes processes.
    pub requires_write_or_exec: bool,
    /// Runs a persistent service.
    pub runs_server: bool,
    /// Needs GUI or browser.
    pub gui_or_browser: bool,
    /// Executes untrusted code.
    pub untrusted_code: bool,
    /// Benchmark batch.
    pub bench_batch: bool,
    /// Nested KVM is available.
    pub nested_kvm_ok: bool,
    /// Contree is available and credentials are present.
    pub contree_available_with_credentials: bool,
    /// Remote OpenShell gateway is available.
    pub remote_gateway_available: bool,
    /// Windows AppContainer tier is available.
    pub appcontainer_available: bool,
}
/// Execution isolation tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SandboxTier {
    /// In-process read-only.
    E0,
    /// OpenShell container.
    E1,
    /// OpenShell microVM.
    E2,
    /// Contree remote sandbox.
    E3,
    /// Remote OpenShell gateway.
    E4,
    /// Windows AppContainer.
    E1w,
}
/// Caller sandbox directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxDirective {
    /// Select from facts and configured tiers.
    Auto,
    /// Require this tier if available and safe.
    Require(SandboxTier),
}
/// Router bounds and allowlists.
#[derive(Debug, Clone)]
pub struct RouterConfig {
    /// Nonempty model effort allowlist.
    pub allowed_efforts: BTreeSet<Effort>,
    /// Exact IDs eligible for direct-tool recommendations.
    pub direct_tool_allowlist: BTreeSet<ToolId>,
    /// Maximum lexical candidates submitted to S1.
    pub max_candidates: usize,
    /// Maximum retained chars per candidate description.
    pub max_candidate_chars: usize,
    /// Maximum suggested delegation depth.
    pub max_spawn_depth: u8,
    /// Enabled sandbox tiers.
    pub allowed_sandbox_tiers: BTreeSet<SandboxTier>,
}
impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            allowed_efforts: [Effort::Low, Effort::Medium, Effort::High]
                .into_iter()
                .collect(),
            direct_tool_allowlist: BTreeSet::new(),
            max_candidates: 32,
            max_candidate_chars: 2048,
            max_spawn_depth: 2,
            allowed_sandbox_tiers: [SandboxTier::E0, SandboxTier::E1].into_iter().collect(),
        }
    }
}
/// Invalid router input or policy.
#[derive(Debug, Error)]
pub enum RouterError {
    /// Decision engine error.
    #[error("decision engine failed: {0}")]
    Engine(#[from] DecisionError),
    /// Schema or decision-shape error.
    #[error("schema error: {0}")]
    Schema(#[from] SchemaError),
    /// Candidate set is empty.
    #[error("candidate set is empty")]
    EmptyCandidates,
    /// TopK is zero.
    #[error("TopK must be nonzero")]
    TopKZero,
    /// Router policy is invalid.
    #[error("invalid router config: {0}")]
    InvalidConfig(String),
    /// Caller input or candidate is invalid.
    #[error("invalid routing input: {0}")]
    InvalidCandidate(String),
    /// Lexical score outside probability range.
    #[error("invalid lexical score")]
    InvalidLexicalScore,
    /// No allowed effort remains.
    #[error("no allowed model effort remains")]
    NoAllowedEffort,
    /// Forced sandbox unavailable or unsafe.
    #[error("sandbox unavailable: {0:?}")]
    SandboxUnavailable(SandboxTier),
    /// No available tier satisfies required isolation.
    #[error("no safe sandbox tier available")]
    NoSafeSandbox,
    /// Probability construction failed.
    #[error("invalid probability: {0}")]
    Probability(#[from] ParseError),
}
/// S1 router with injected engine and catalog.
#[derive(Debug)]
pub struct Router<D, C> {
    engine: D,
    catalog: C,
    config: RouterConfig,
}
impl<D: DecisionEngine, C: SchemaCatalog> Router<D, C> {
    /// Create a router with explicit effort and sandbox policies.
    /// # Errors
    /// Rejects empty effort allowlists and zero candidate bounds.
    pub fn new(engine: D, catalog: C, config: RouterConfig) -> Result<Self, RouterError> {
        if config.allowed_efforts.is_empty()
            || config.max_candidates == 0
            || config.max_candidate_chars == 0
        {
            return Err(RouterError::InvalidConfig(
                "effort and candidate bounds must be nonzero".into(),
            ));
        }
        Ok(Self {
            engine,
            catalog,
            config,
        })
    }
    /// Classify intent and effort. Uncertain routing defaults to S2.
    ///
    /// S1 never grants direct execution. Any direct-tool result is only a recommendation
    /// for an exact configured ID; the dispatcher must recheck authorization.
    /// Input text must be PII-masked.
    /// # Errors
    /// Returns an error when hard caller constraints conflict with router policy.
    pub async fn route(&self, input: &RouteInput) -> Result<Route, RouterError> {
        let allowed = self.allowed_efforts(&input.directives)?;
        let fallback = *allowed.iter().next().ok_or(RouterError::NoAllowedEffort)?;
        let mut confidence = Probability::new(0.0)?;
        let mut source = RouteSource::DeterministicFallback;
        if input.text.trim().is_empty() {
            return Ok(Route {
                path: RoutePath::AskClarification,
                effort: fallback,
                needs_vision: input.repo_facts.needs_vision,
                spawn: SpawnAdvice {
                    parallel_branches: false,
                    max_depth: 0,
                },
                confidence,
                source,
            });
        }
        if let Some(schema) = self.catalog.latest("route.intent") {
            if let Some(d) = self
                .decide(schema, &input.text, input.root_goal.as_deref())
                .await
            {
                if apply_threshold(schema, &d).is_ok() {
                    if let Some(p) = choice_confidence(&d) {
                        confidence = p;
                        source = RouteSource::S1;
                    }
                }
            }
        }
        let mut effort = fallback;
        if let Some(schema) = self.catalog.latest("route.effort") {
            if let Some(d) = self
                .decide(schema, &input.text, input.root_goal.as_deref())
                .await
            {
                if let Ok(v) = apply_threshold(schema, &d) {
                    if matches!(v.first(), Some((_, Verdict::Accept))) {
                        if let Some((value, certainty)) = score(&d) {
                            if let Ok(value) = u8::try_from(value) {
                                if let Ok(wanted) = Effort::try_from(value) {
                                    effort = choose_effort(wanted, &allowed)?;
                                    confidence = certainty;
                                    source = RouteSource::S1;
                                }
                            }
                        }
                    }
                }
            }
        }
        let path = match input.directives.direct_tool.as_ref() {
            Some(id) if self.config.direct_tool_allowlist.contains(id) => {
                RoutePath::DirectTool { tool: id.clone() }
            }
            _ => RoutePath::S2,
        };
        let parallel = input.repo_facts.independent_workstreams > 1;
        Ok(Route {
            path,
            effort,
            needs_vision: input.repo_facts.needs_vision,
            spawn: SpawnAdvice {
                parallel_branches: parallel,
                max_depth: if parallel {
                    self.config.max_spawn_depth
                } else {
                    0
                },
            },
            confidence,
            source,
        })
    }
    /// Bounded lexical prefilter plus semantic tool scoring. On S1 uncertainty, preserve the
    /// deterministic lexical shortlist for caller policy.
    /// # Errors
    /// Returns error for zero candidates or invalid TopK.
    pub async fn prune_tools(
        &self,
        step: &str,
        tools: &[ToolCandidate],
        k: TopK,
    ) -> Result<Vec<Ranked<ToolId>>, RouterError> {
        if tools.is_empty() {
            return Err(RouterError::EmptyCandidates);
        }
        let mut shortlist = tools
            .iter()
            .map(|tool| {
                (
                    tool,
                    overlap(
                        step,
                        &format!(
                            "{} {}",
                            tool.name,
                            truncate(&tool.summary, self.config.max_candidate_chars)
                        ),
                    ),
                )
            })
            .collect::<Vec<_>>();
        shortlist.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
        shortlist.truncate(self.config.max_candidates);
        let Some(schema) = self.catalog.latest("route.tool") else {
            return tool_fallback(&shortlist, k);
        };
        let mut results = Vec::new();
        for (tool, _) in &shortlist {
            let ctx = format!(
                "Tool name: {}\nTool description: {}",
                tool.name,
                truncate(&tool.summary, self.config.max_candidate_chars)
            );
            let Some(d) = self.decide(schema, step, Some(&ctx)).await else {
                return tool_fallback(&shortlist, k);
            };
            let Ok(v) = apply_threshold(schema, &d) else {
                return tool_fallback(&shortlist, k);
            };
            let Some((_, verdict)) = v.first() else {
                return tool_fallback(&shortlist, k);
            };
            match yes(&d) {
                Some(p) if *verdict == Verdict::Accept => results.push(Ranked {
                    item: tool.id.clone(),
                    score: p,
                    reason: RankReason::Semantic,
                }),
                Some(_) if *verdict == Verdict::Reject => {}
                _ => return tool_fallback(&shortlist, k),
            }
        }
        results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.item.cmp(&b.item)));
        results.truncate(usize::from(k.get()));
        Ok(results)
    }
    /// Bounded semantic file triage. On S1 uncertainty, retain the deterministic lexical shortlist.
    /// # Errors
    /// Returns error for no candidates or invalid caller lexical scores.
    pub async fn rank_files(
        &self,
        query: &str,
        files: &[FileCandidate],
        k: TopK,
    ) -> Result<Vec<Ranked<PathBuf>>, RouterError> {
        if files.is_empty() {
            return Err(RouterError::EmptyCandidates);
        }
        let mut shortlist = Vec::new();
        for file in files {
            let score = Probability::new(file.lexical_score)
                .map_err(|_| RouterError::InvalidLexicalScore)?;
            shortlist.push((file, score));
        }
        shortlist.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.path.cmp(&b.0.path)));
        shortlist.truncate(self.config.max_candidates);
        let Some(schema) = self.catalog.latest("route.file") else {
            return file_fallback(&shortlist, k);
        };
        let mut results = Vec::new();
        for (file, _) in &shortlist {
            let ctx = truncate(&file.signature_text, self.config.max_candidate_chars);
            let Some(d) = self.decide(schema, query, Some(&ctx)).await else {
                return file_fallback(&shortlist, k);
            };
            let Ok(v) = apply_threshold(schema, &d) else {
                return file_fallback(&shortlist, k);
            };
            let Some((_, verdict)) = v.first() else {
                return file_fallback(&shortlist, k);
            };
            match yes(&d) {
                Some(p) if *verdict == Verdict::Accept => results.push(Ranked {
                    item: file.path.clone(),
                    score: p,
                    reason: RankReason::Semantic,
                }),
                Some(_) if *verdict == Verdict::Reject => {}
                _ => return file_fallback(&shortlist, k),
            }
        }
        results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.item.cmp(&b.item)));
        results.truncate(usize::from(k.get()));
        Ok(results)
    }
    /// Deterministically select isolation. S1 may only escalate E0 to E1.
    /// # Errors
    /// Rejects unsafe forced tiers and unavailable isolation.
    pub async fn route_sandbox(
        &self,
        f: &SandboxFacts,
        d: SandboxDirective,
    ) -> Result<SandboxTier, RouterError> {
        let tier = match d {
            SandboxDirective::Require(t) => {
                if tier_allowed(t, f, &self.config.allowed_sandbox_tiers) {
                    t
                } else {
                    return Err(RouterError::SandboxUnavailable(t));
                }
            }
            SandboxDirective::Auto => {
                if f.bench_batch
                    && f.contree_available_with_credentials
                    && self.config.allowed_sandbox_tiers.contains(&SandboxTier::E3)
                {
                    SandboxTier::E3
                } else if f.untrusted_code || f.gui_or_browser {
                    if f.nested_kvm_ok
                        && self.config.allowed_sandbox_tiers.contains(&SandboxTier::E2)
                    {
                        SandboxTier::E2
                    } else if f.remote_gateway_available
                        && self.config.allowed_sandbox_tiers.contains(&SandboxTier::E4)
                    {
                        SandboxTier::E4
                    } else {
                        return Err(RouterError::NoSafeSandbox);
                    }
                } else if f.requires_write_or_exec || f.runs_server {
                    if self.config.allowed_sandbox_tiers.contains(&SandboxTier::E1) {
                        SandboxTier::E1
                    } else if f.remote_gateway_available
                        && self.config.allowed_sandbox_tiers.contains(&SandboxTier::E4)
                    {
                        SandboxTier::E4
                    } else {
                        return Err(RouterError::NoSafeSandbox);
                    }
                } else if self.config.allowed_sandbox_tiers.contains(&SandboxTier::E0) {
                    SandboxTier::E0
                } else if self.config.allowed_sandbox_tiers.contains(&SandboxTier::E1) {
                    SandboxTier::E1
                } else {
                    return Err(RouterError::NoSafeSandbox);
                }
            }
        };
        if tier == SandboxTier::E0 {
            if let Some(s) = self.catalog.latest("route.sandbox") {
                let c = format!("Deterministic tier E0. Facts: {f:?}");
                if let Some(decision) = self.decide(s, &c, None).await {
                    if let Ok(verdicts) = apply_threshold(s, &decision) {
                        let accepts = verdicts
                            .iter()
                            .all(|(_, verdict)| *verdict == Verdict::Accept);
                        if accepts
                            && yes(&decision).is_some_and(|p| p.get() >= 0.9)
                            && self.config.allowed_sandbox_tiers.contains(&SandboxTier::E1)
                        {
                            return Ok(SandboxTier::E1);
                        }
                    }
                }
            }
        }
        Ok(tier)
    }
    fn allowed_efforts(&self, d: &DirectiveSnapshot) -> Result<BTreeSet<Effort>, RouterError> {
        let result = match &d.allowed_efforts {
            Some(values) => {
                let set = values.iter().copied().collect::<BTreeSet<_>>();
                self.config
                    .allowed_efforts
                    .intersection(&set)
                    .copied()
                    .collect()
            }
            None => self.config.allowed_efforts.clone(),
        };
        if result.is_empty() {
            Err(RouterError::NoAllowedEffort)
        } else {
            Ok(result)
        }
    }
    async fn decide(&self, s: &Schema, text: &str, context: Option<&str>) -> Option<Decision> {
        if text.chars().count() > usize::try_from(s.max_chars).ok()? {
            return None;
        }
        let input = DecisionInput {
            text: text.to_owned(),
            context: context.map(str::to_owned),
        };
        match tokio::time::timeout(
            Duration::from_millis(u64::from(s.fail.deadline_ms)),
            self.engine.decide(&s.questions, &input),
        )
        .await
        {
            Ok(Ok(d)) => Some(d),
            Ok(Err(_)) | Err(_) => None,
        }
    }
}
fn choose_effort(wanted: Effort, allowed: &BTreeSet<Effort>) -> Result<Effort, RouterError> {
    allowed
        .iter()
        .copied()
        .find(|x| *x >= wanted)
        .or_else(|| allowed.iter().next_back().copied())
        .ok_or(RouterError::NoAllowedEffort)
}
fn choice_confidence(d: &Decision) -> Option<Probability> {
    d.answers.iter().find_map(|a| match &a.answer {
        Answer::Choice { selected, probs } => probs
            .iter()
            .find(|p| &p.label == selected)
            .map(|p| p.probability),
        _ => None,
    })
}
fn score(d: &Decision) -> Option<(i32, Probability)> {
    d.answers.iter().find_map(|a| match &a.answer {
        Answer::Score { value, confidence } => Some((*value, *confidence)),
        _ => None,
    })
}
fn yes(d: &Decision) -> Option<Probability> {
    d.answers.iter().find_map(|a| match &a.answer {
        Answer::Noul { yes } => Some(*yes),
        _ => None,
    })
}
fn overlap(a: &str, b: &str) -> f32 {
    let terms = words(a);
    if terms.is_empty() {
        return 0.0;
    }
    let candidates = words(b);
    let mut count = 0_u16;
    let mut hits = 0_u16;
    for term in terms {
        count = count.saturating_add(1);
        if candidates.contains(&term) {
            hits = hits.saturating_add(1);
        }
    }
    if count == 0 {
        0.0
    } else {
        f32::from(hits) / f32::from(count)
    }
}
fn words(s: &str) -> BTreeSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|x| x.chars().count() > 1)
        .map(str::to_lowercase)
        .collect()
}
fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
fn tool_fallback(
    xs: &[(&ToolCandidate, f32)],
    k: TopK,
) -> Result<Vec<Ranked<ToolId>>, RouterError> {
    let mut out = Vec::new();
    for (x, score) in xs {
        out.push(Ranked {
            item: x.id.clone(),
            score: Probability::new(*score)?,
            reason: RankReason::LexicalFallback,
        });
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.item.cmp(&b.item)));
    out.truncate(usize::from(k.get()));
    Ok(out)
}
fn file_fallback(
    xs: &[(&FileCandidate, Probability)],
    k: TopK,
) -> Result<Vec<Ranked<PathBuf>>, RouterError> {
    let mut out = xs
        .iter()
        .map(|(x, score)| Ranked {
            item: x.path.clone(),
            score: *score,
            reason: RankReason::LexicalFallback,
        })
        .collect::<Vec<_>>();
    out.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.item.cmp(&b.item)));
    out.truncate(usize::from(k.get()));
    Ok(out)
}
fn tier_allowed(t: SandboxTier, f: &SandboxFacts, a: &BTreeSet<SandboxTier>) -> bool {
    a.contains(&t)
        && match t {
            SandboxTier::E0 => {
                !f.requires_write_or_exec
                    && !f.runs_server
                    && !f.gui_or_browser
                    && !f.untrusted_code
                    && !f.bench_batch
            }
            SandboxTier::E1 => !f.gui_or_browser && !f.untrusted_code && !f.bench_batch,
            SandboxTier::E2 => f.nested_kvm_ok && (f.gui_or_browser || f.untrusted_code),
            SandboxTier::E3 => f.bench_batch && f.contree_available_with_credentials,
            SandboxTier::E4 => f.remote_gateway_available,
            SandboxTier::E1w => {
                f.appcontainer_available && !f.untrusted_code && !f.gui_or_browser && !f.bench_batch
            }
        }
}
