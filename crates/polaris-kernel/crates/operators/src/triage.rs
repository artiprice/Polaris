//! S1 triage adapter over the real router with masked-only inputs.
use crate::MaskedText;
use polaris_sdk::{
    error::{ErrorKind, HasKind},
    registry::{DecisionEngine, FunctionalOperator},
};
use polaris_system_one_router::{
    DirectiveSnapshot, RepoFacts, Route, RouteInput, Router, RouterError,
};
use polaris_system_one_schemas::SchemaCatalog;

/// Bounded, already-sanitized task facts for the routing stage.
#[derive(Debug, Clone)]
pub struct TriageInput {
    /// PII-masked primary task text.
    pub text: MaskedText,
    /// Optional PII-masked root goal.
    pub root_goal: Option<MaskedText>,
    /// Caller-provided repository facts.
    pub repo_facts: RepoFacts,
    /// Directives already validated/applied by the owning session layer.
    pub directives: DirectiveSnapshot,
}

/// Router failure. The router itself handles S1 timeouts by returning its deterministic route.
#[derive(Debug, thiserror::Error)]
#[error("triage router rejected the supplied constraints")]
pub struct TriageError(#[source] RouterError);
impl HasKind for TriageError {
    /// Preserve retry/policy classification from the router; invalid caller inputs are not fatal integrity failures.
    fn kind(&self) -> ErrorKind {
        match &self.0 {
            RouterError::Engine(_) => ErrorKind::Transient,
            RouterError::InvalidCandidate(_)
            | RouterError::EmptyCandidates
            | RouterError::TopKZero => ErrorKind::Permanent,
            RouterError::NoAllowedEffort
            | RouterError::SandboxUnavailable(_)
            | RouterError::NoSafeSandbox => ErrorKind::Policy,
            RouterError::Schema(_)
            | RouterError::InvalidConfig(_)
            | RouterError::InvalidLexicalScore
            | RouterError::Probability(_) => ErrorKind::Fatal,
        }
    }
}
impl TriageError {
    /// Underlying router failure for structured caller reporting.
    #[must_use]
    pub fn source_error(&self) -> &RouterError {
        &self.0
    }
}
impl From<RouterError> for TriageError {
    fn from(error: RouterError) -> Self {
        Self(error)
    }
}

/// Calls the injected System-One router. The result is advisory and does not grant tool access.
#[derive(Debug)]
pub struct S1TriageOp<D, C> {
    router: Router<D, C>,
}
impl<D, C> S1TriageOp<D, C> {
    /// Wrap an actual configured router and its real S1 engine/schema catalog.
    #[must_use]
    pub const fn new(router: Router<D, C>) -> Self {
        Self { router }
    }
}
impl<D, C> FunctionalOperator for S1TriageOp<D, C>
where
    D: DecisionEngine,
    C: SchemaCatalog,
{
    type In = TriageInput;
    type Out = Route;
    type Err = TriageError;
    async fn apply(&self, input: Self::In) -> Result<Self::Out, Self::Err> {
        let route_input = RouteInput {
            text: input.text.as_str().to_owned(),
            root_goal: input.root_goal.map(|goal| goal.as_str().to_owned()),
            repo_facts: input.repo_facts,
            directives: input.directives,
        };
        self.router
            .route(&route_input)
            .await
            .map_err(TriageError::from)
    }
}
