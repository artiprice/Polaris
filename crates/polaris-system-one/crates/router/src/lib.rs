//! System-One routing advice with bounded scoring and deterministic safe fallbacks.
#![forbid(unsafe_code)]
use polaris_sdk::{
    effort::Effort,
    error::ParseError,
    registry::{DecisionEngine, DecisionError},
    system_one::{Answer, Decision, DecisionInput, Probability, QuestionKind},
};
use polaris_system_one_schemas::{
    Consumer, Schema, SchemaCatalog, SchemaError, SchemaKind, Verdict, apply_threshold,
};
use std::{collections::BTreeSet, path::PathBuf, time::Duration};
use thiserror::Error;

const HARD_MAX_CANDIDATES: usize = 256;
const HARD_MAX_INPUT_CANDIDATES: usize = 4_096;
const HARD_MAX_CANDIDATE_CHARS: usize = 4_096;
const HARD_MAX_TOOL_NAME_CHARS: usize = 512;
const HARD_MAX_QUERY_CHARS: usize = 16_384;
const HARD_MAX_ID_CHARS: usize = 256;
const HARD_MAX_REPO_CHARS: usize = 256;
const HARD_MAX_GIT_OID_CHARS: usize = 64;

/// Stable tool identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ToolId(String);
impl ToolId {
    /// Validates a nonempty printable identifier.
    /// # Errors
    /// Returns InvalidCandidate when the identifier is empty or contains controls.
    pub fn new(value: impl Into<String>) -> Result<Self, RouterError> {
        let value = value.into();
        if value.trim().is_empty()
            || value.len() > HARD_MAX_ID_CHARS
            || value.chars().any(char::is_control)
        {
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
/// Exact identifier for an image supplied by a trusted environment catalog.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageId(String);
impl ImageId {
    /// Validates a bounded, nonempty image identifier.
    /// # Errors
    /// Returns InvalidCandidate for an empty, oversized, or control-containing value.
    pub fn new(value: impl Into<String>) -> Result<Self, RouterError> {
        let value = value.into();
        if value.trim().is_empty()
            || value.len() > HARD_MAX_ID_CHARS
            || value.chars().any(char::is_control)
        {
            return Err(RouterError::InvalidCandidate("invalid image id".into()));
        }
        Ok(Self(value))
    }
    /// Returns the exact catalog identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
/// Trusted repository identity and bounded environment requirements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoManifestFacts {
    repo: String,
    commit: Option<String>,
    lockfile_hash: Option<String>,
    requirements: String,
}
impl RepoManifestFacts {
    /// Creates repository facts with a full Git object ID and/or a full lockfile hash.
    /// # Errors
    /// Returns InvalidCandidate for malformed or oversized identity or requirement fields.
    pub fn new(
        repo: impl Into<String>,
        commit: Option<&str>,
        lockfile_hash: Option<&str>,
        requirements: impl Into<String>,
    ) -> Result<Self, RouterError> {
        let repo = repo.into();
        let commit = commit.map(str::to_owned);
        let lockfile_hash = lockfile_hash.map(str::to_owned);
        let requirements = requirements.into();
        validate_repo(&repo)?;
        if commit.is_none() && lockfile_hash.is_none() {
            return Err(RouterError::InvalidCandidate(
                "repository facts need a full commit or lockfile hash".into(),
            ));
        }
        if let Some(value) = &commit {
            validate_oid(value)?;
        }
        if let Some(value) = &lockfile_hash {
            validate_hash(value)?;
        }
        validate_text(&requirements, HARD_MAX_QUERY_CHARS, "requirements")?;
        Ok(Self {
            repo,
            commit,
            lockfile_hash,
            requirements,
        })
    }
}
/// Trusted catalog metadata associated with an actual image identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvImageCandidate {
    id: ImageId,
    repo: Option<String>,
    commit: Option<String>,
    lockfile_hash: Option<String>,
    metadata: String,
}
impl EnvImageCandidate {
    /// Creates an image candidate from trusted catalog annotations.
    /// # Errors
    /// Returns InvalidCandidate for malformed identity or oversized metadata.
    pub fn new(
        id: ImageId,
        repo: Option<&str>,
        commit: Option<&str>,
        lockfile_hash: Option<&str>,
        metadata: impl Into<String>,
    ) -> Result<Self, RouterError> {
        let repo = repo.map(str::to_owned);
        let commit = commit.map(str::to_owned);
        let lockfile_hash = lockfile_hash.map(str::to_owned);
        let metadata = metadata.into();
        if let Some(value) = &repo {
            validate_repo(value)?;
        }
        if commit.is_some() && repo.is_none() {
            return Err(RouterError::InvalidCandidate(
                "commit identity requires a repository".into(),
            ));
        }
        if let Some(value) = &commit {
            validate_oid(value)?;
        }
        if let Some(value) = &lockfile_hash {
            validate_hash(value)?;
        }
        validate_text(&metadata, HARD_MAX_CANDIDATE_CHARS, "image metadata")?;
        Ok(Self {
            id,
            repo,
            commit,
            lockfile_hash,
            metadata,
        })
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
    /// Exact repository identity or lockfile hash match.
    ExactIdentity,
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
            || config.max_candidates > HARD_MAX_CANDIDATES
            || config.max_candidate_chars > HARD_MAX_CANDIDATE_CHARS
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
        let context_len = input.root_goal.as_ref().map_or(0, String::len);
        if input
            .text
            .len()
            .checked_add(context_len)
            .is_none_or(|bytes| bytes > 262_144)
        {
            return Err(RouterError::InvalidCandidate(
                "task exceeds routing byte bound".into(),
            ));
        }
        let allowed = self.allowed_efforts(&input.directives)?;
        let fallback = choose_effort(Effort::Medium, &allowed)?;
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
        let mut effort = fallback;
        let mut inferred_path = RoutePath::S2;
        let mut inferred_parallel = false;
        let mut inferred_vision = false;
        if let Some(schema) = self.catalog.latest("route.task")
            && schema.fail.consumer == polaris_system_one_schemas::Consumer::EffortRouter
            && schema.questions.len() == 4
            && schema
                .questions
                .iter()
                .map(|q| q.id.to_string())
                .collect::<BTreeSet<_>>()
                == ["intent", "effort", "parallel", "vision"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            && let Some(decision) = self
                .decide(schema, &input.text, input.root_goal.as_deref())
                .await
            && let Ok(verdicts) = apply_threshold(schema, &decision)
        {
            if accepted(&verdicts, "intent")
                && let Some(Answer::Choice { selected, probs }) = answer_for(&decision, "intent")
            {
                inferred_path = match selected.as_str() {
                    "clarification" => RoutePath::AskClarification,
                    "trivial_direct" | "s2" => RoutePath::S2,
                    _ => RoutePath::S2,
                };
                // A model cannot invent a direct tool. Only the hard configured
                // directive below may select an exact allowlisted target.
                if let Some(probability) = probs.iter().find(|p| p.label == *selected) {
                    confidence = probability.probability;
                    source = RouteSource::S1;
                }
            }
            if accepted(&verdicts, "effort")
                && let Some(Answer::Score {
                    value,
                    confidence: certainty,
                }) = answer_for(&decision, "effort")
                && let Ok(value) = u8::try_from(*value)
                && let Ok(wanted) = Effort::try_from(value)
            {
                effort = choose_effort(wanted, &allowed)?;
                confidence = *certainty;
                source = RouteSource::S1;
            }
            inferred_parallel = accepted(&verdicts, "parallel")
                && matches!(answer_for(&decision, "parallel"), Some(Answer::Noul { .. }));
            inferred_vision = accepted(&verdicts, "vision")
                && matches!(answer_for(&decision, "vision"), Some(Answer::Noul { .. }));
        }
        let path = match input.directives.direct_tool.as_ref() {
            Some(id) if self.config.direct_tool_allowlist.contains(id) => {
                RoutePath::DirectTool { tool: id.clone() }
            }
            _ => inferred_path,
        };
        let parallel = input.repo_facts.independent_workstreams > 1 || inferred_parallel;
        Ok(Route {
            path,
            effort,
            needs_vision: input.repo_facts.needs_vision || inferred_vision,
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
        validate_candidate_count(tools.len())?;
        validate_text(step, HARD_MAX_QUERY_CHARS, "tool query")?;
        let mut ids = BTreeSet::new();
        for tool in tools {
            if !ids.insert(&tool.id) {
                return Err(RouterError::InvalidCandidate("duplicate tool id".into()));
            }
            validate_text(&tool.name, HARD_MAX_TOOL_NAME_CHARS, "tool name")?;
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
        validate_candidate_count(files.len())?;
        validate_text(query, HARD_MAX_QUERY_CHARS, "file query")?;
        let mut paths = BTreeSet::new();
        for file in files {
            if file.path.as_os_str().len() > HARD_MAX_CANDIDATE_CHARS || !paths.insert(&file.path) {
                return Err(RouterError::InvalidCandidate(
                    "invalid or duplicate file path".into(),
                ));
            }
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
    /// Rank environment images with exact identity matches before bounded S1 scoring.
    ///
    /// Only identifiers joined to trusted catalog metadata supplied by the caller can be
    /// returned. A repository match without the full commit or lockfile hash is a near miss.
    /// # Errors
    /// Returns an error for an empty set, duplicate IDs, or inputs beyond hard bounds.
    pub async fn rank_env_images(
        &self,
        manifest: &RepoManifestFacts,
        images: &[EnvImageCandidate],
    ) -> Result<Vec<Ranked<ImageId>>, RouterError> {
        if images.is_empty() {
            return Err(RouterError::EmptyCandidates);
        }
        validate_candidate_count(images.len())?;
        let mut ids = BTreeSet::new();
        for image in images {
            if !ids.insert(&image.id) {
                return Err(RouterError::InvalidCandidate("duplicate image id".into()));
            }
        }
        let mut exact = Vec::new();
        for image in images {
            let commit_match = manifest.commit.as_ref().is_some_and(|commit| {
                image.repo.as_ref() == Some(&manifest.repo) && image.commit.as_ref() == Some(commit)
            });
            let hash_match = manifest.lockfile_hash.as_ref().is_some_and(|hash| {
                image.repo.as_ref() == Some(&manifest.repo)
                    && image.lockfile_hash.as_ref() == Some(hash)
            });
            if commit_match || hash_match {
                exact.push(image);
            }
        }
        if !exact.is_empty() {
            let score = Probability::new(1.0)?;
            let mut results = exact
                .into_iter()
                .map(|image| Ranked {
                    item: image.id.clone(),
                    score,
                    reason: RankReason::ExactIdentity,
                })
                .collect::<Vec<_>>();
            results.sort_by(|a, b| a.item.cmp(&b.item));
            return Ok(results);
        }
        let query = format!(
            "Repository: {}\nRequirements: {}",
            manifest.repo, manifest.requirements
        );
        let mut shortlist = images
            .iter()
            .map(|image| {
                let metadata = format!(
                    "{} {}",
                    image
                        .repo
                        .as_deref()
                        .map_or_else(String::new, str::to_owned),
                    truncate(&image.metadata, self.config.max_candidate_chars)
                );
                let score = overlap(&query, &metadata);
                (image, score)
            })
            .collect::<Vec<_>>();
        shortlist.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
        shortlist.truncate(self.config.max_candidates);
        let Some(schema) = self.catalog.latest("route.env") else {
            return image_fallback(&shortlist, self.config.max_candidates);
        };
        if schema.kind != SchemaKind::EnvMatch
            || schema.fail.consumer != Consumer::EnvMatch
            || schema.questions.len() != 1
            || schema.questions.iter().any(|question| {
                question.id.to_string() != "matches"
                    || !matches!(&question.kind, QuestionKind::Noul { .. })
            })
        {
            return image_fallback(&shortlist, self.config.max_candidates);
        }
        let mut results = Vec::new();
        for (image, _) in &shortlist {
            let context = format!(
                "Repository: {}\nCommit: {}\nLockfile hash: {}\nImage metadata: {}",
                image
                    .repo
                    .as_deref()
                    .map_or_else(String::new, str::to_owned),
                image
                    .commit
                    .as_deref()
                    .map_or_else(String::new, str::to_owned),
                image
                    .lockfile_hash
                    .as_deref()
                    .map_or_else(String::new, str::to_owned),
                truncate(&image.metadata, self.config.max_candidate_chars)
            );
            let Some(decision) = self.decide(schema, &query, Some(&context)).await else {
                return image_fallback(&shortlist, self.config.max_candidates);
            };
            let Ok(verdicts) = apply_threshold(schema, &decision) else {
                return image_fallback(&shortlist, self.config.max_candidates);
            };
            let Some((_, verdict)) = verdicts.first() else {
                return image_fallback(&shortlist, self.config.max_candidates);
            };
            match yes(&decision) {
                Some(score) if *verdict == Verdict::Accept => results.push(Ranked {
                    item: image.id.clone(),
                    score,
                    reason: RankReason::Semantic,
                }),
                Some(_) if *verdict == Verdict::Reject => {}
                _ => return image_fallback(&shortlist, self.config.max_candidates),
            }
        }
        results.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.item.cmp(&b.item)));
        results.truncate(self.config.max_candidates);
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
        if tier == SandboxTier::E0
            && let Some(s) = self.catalog.latest("route.sandbox")
        {
            let c = format!("Deterministic tier E0. Facts: {f:?}");
            if let Some(decision) = self.decide(s, &c, None).await
                && let Ok(verdicts) = apply_threshold(s, &decision)
            {
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
        let context_bytes = context.map_or(0, str::len);
        if text
            .len()
            .checked_add(context_bytes)
            .is_none_or(|size| size > 262_144)
            || s.fail.deadline_ms == 0
            || s.fail.deadline_ms > 120_000
        {
            return None;
        }
        let chars = text
            .chars()
            .count()
            .checked_add(context.map_or(0, |value| value.chars().count()))?;
        if chars > usize::try_from(s.max_chars).ok()? {
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
fn answer_for<'a>(decision: &'a Decision, id: &str) -> Option<&'a Answer> {
    decision
        .answers
        .iter()
        .find(|answer| answer.question.to_string() == id)
        .map(|answer| &answer.answer)
}
fn accepted(verdicts: &[(polaris_sdk::system_one::QuestionId, Verdict)], id: &str) -> bool {
    verdicts
        .iter()
        .any(|(question, verdict)| question.to_string() == id && *verdict == Verdict::Accept)
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
fn image_fallback(
    xs: &[(&EnvImageCandidate, f32)],
    k: usize,
) -> Result<Vec<Ranked<ImageId>>, RouterError> {
    let mut out = Vec::new();
    for (image, score) in xs {
        out.push(Ranked {
            item: image.id.clone(),
            score: Probability::new(*score)?,
            reason: RankReason::LexicalFallback,
        });
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.item.cmp(&b.item)));
    out.truncate(k);
    Ok(out)
}
fn validate_candidate_count(count: usize) -> Result<(), RouterError> {
    if count > HARD_MAX_INPUT_CANDIDATES {
        Err(RouterError::InvalidCandidate(
            "candidate set exceeds processing bound".into(),
        ))
    } else {
        Ok(())
    }
}
fn validate_text(value: &str, max_chars: usize, field: &str) -> Result<(), RouterError> {
    if value.len() > max_chars.saturating_mul(4)
        || value.chars().take(max_chars.saturating_add(1)).count() > max_chars
    {
        Err(RouterError::InvalidCandidate(format!(
            "{field} exceeds processing bound"
        )))
    } else {
        Ok(())
    }
}
fn validate_repo(value: &str) -> Result<(), RouterError> {
    if value.is_empty()
        || value.len() > HARD_MAX_REPO_CHARS
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace())
        || value.split('/').count() != 2
        || value.split('/').any(str::is_empty)
    {
        Err(RouterError::InvalidCandidate(
            "invalid repository identity".into(),
        ))
    } else {
        Ok(())
    }
}
fn validate_oid(value: &str) -> Result<(), RouterError> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Err(RouterError::InvalidCandidate(
            "commit must be a full lowercase git object id".into(),
        ))
    } else {
        Ok(())
    }
}
fn validate_hash(value: &str) -> Result<(), RouterError> {
    if value.len() != HARD_MAX_GIT_OID_CHARS
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Err(RouterError::InvalidCandidate(
            "lockfile hash must be a full lowercase sha256 digest".into(),
        ))
    } else {
        Ok(())
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
