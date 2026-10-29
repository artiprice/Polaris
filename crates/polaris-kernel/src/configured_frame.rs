//! Configured non-UI frame execution over actual host-owned S1 and kernel evidence owners.
use crate::{
    FrameToolAccess, FrameVerificationError, KernelFrameJournal, KernelFrameJournalError,
    KernelFrameVerifier, KernelHandle, KernelPromptAssembler, KernelPromptError,
    PromptAssemblySettings, PromptContext, SemanticPrivacyLimits, VerifiedFrameCompletion,
};
use polaris_runtime::frames::{FrameError, FrameJournalLimits, FrameRunInput, FrameRunOutcome};
use polaris_sdk::registry::{ChatModel, PromptTokenizer};
use polaris_security::capability::Token;
use polaris_system_one::watchdog::RoleTable;
use std::{fmt, sync::Arc};

/// Explicit host inputs for an already-authorized frame. No authority is minted by this API.
/// The same model owner performs both chat-template counting and actual chat dispatch.
pub struct ConfiguredFrameServices<M, T> {
    /// Exact capability captured by the active frame and its kernel runner.
    pub token: Token,
    /// Sealed kernel tool runner, with its existing mutation/test/read authority.
    pub runner: Arc<T>,
    /// Actual model and exact tokenizer owner, including any retained model lease.
    pub model: Arc<M>,
    /// Trusted system policy and untrusted source/scratchpad snapshots.
    pub context: PromptContext,
    /// Bounded prompt settings and optional actual restored privacy map.
    pub prompt: PromptAssemblySettings,
    /// Host-authored advisory role policy; capabilities still authorize every tool separately.
    pub role_table: RoleTable,
    /// Total semantic masking deadline and field bounds.
    pub semantic_privacy: SemanticPrivacyLimits,
}
impl<M, T> fmt::Debug for ConfiguredFrameServices<M, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ConfiguredFrameServices(<redacted>)")
    }
}

/// Explicit measured watchdog artifacts supplied by the trusted host.
#[derive(Debug, Clone)]
pub struct ConfiguredWatchdogPolicy {
    /// Actual measured schema catalog including deterministic watchdog configuration.
    pub schemas: polaris_system_one::schemas::SchemaSet,
    /// Expected fixture and exact native checkpoint identities.
    pub calibration: polaris_system_one::schemas::CalibrationContext,
}

/// Optional host recovery and measured advisory policy for one actual frame execution.
#[derive(Debug, Default)]
pub struct ConfiguredFramePolicy {
    /// Required encrypted private journal checkpoints when configured.
    pub checkpoints: Option<Arc<crate::PrivateFrameCheckpointStore>>,
    /// Measured watchdog policy; absence retains explicitly uncalibrated built-in advice.
    pub watchdog: Option<ConfiguredWatchdogPolicy>,
}

/// Independent completion result, separate from the model's terminal response.
#[derive(Debug)]
pub enum FrameCompletionCheck {
    /// Actual same-frame current-source ordinary/independent test proof was validated.
    Verified(VerifiedFrameCompletion),
    /// Execution ended, but the actual current-tree proof gate did not validate.
    Unverified(FrameVerificationError),
}

/// Whether the frame ran with the local System One host or in its deterministic degraded mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemOneCoverage {
    /// Semantic PII detection, watchdog advice and tool pruning ran on the owned S1 host.
    Active,
    /// No S1 host: PII masking used the deterministic DFA/entropy vault only, and the advisory
    /// watchdog and tool pruning were off (fail-open: the full permitted catalog was offered).
    Degraded,
}

/// Actual run and current-tree completion check. A normal terminal response alone is not done.
#[derive(Debug)]
pub struct ConfiguredFrameOutcome {
    /// Bounded driver result with actual attempts and tool rounds.
    pub execution: FrameRunOutcome,
    /// Separate proof check, never fabricated from an S1/model opinion.
    pub completion: FrameCompletionCheck,
    /// System One coverage for this run.
    pub system_one: SystemOneCoverage,
    /// Observed coverage of the last successful prompt sanitization. Deterministic fallback
    /// remains explicit when the optional PII companion is unavailable or its work budget ends.
    pub semantic_privacy: Option<crate::SemanticPrivacyReport>,
    /// Actual reversible marker map retained for the trusted host. To support session resume,
    /// persist this through the authenticated private snapshot store with the host's recovered
    /// seal key; this handle must never be serialized to a model, log or public task summary.
    pub privacy_vault: Arc<polaris_security::pii::PiiVault>,
}

/// Composition and execution errors without raw model or credential payloads.
#[derive(Debug, thiserror::Error)]
pub enum ConfiguredFrameError {
    /// Frame belongs to another kernel or its token/runner binding is invalid.
    #[error("configured frame identity rejected")]
    Identity,
    /// This entry point requires an actual configured, owned S1 host.
    #[error("configured agent execution requires a System One worker profile")]
    MissingSystemOne,
    /// Actual prompt identity, privacy, context or tokenizer failure.
    #[error(transparent)]
    Prompt(#[from] KernelPromptError),
    /// Durable journal construction failed.
    #[error(transparent)]
    Journal(#[from] KernelFrameJournalError),
    /// Actual verifier could not bind to the frame.
    #[error(transparent)]
    Verifier(#[from] FrameVerificationError),
    /// Driver, model, tool, cancellation or journal execution failure.
    #[error(transparent)]
    Execution(#[from] FrameError),
}

impl KernelHandle {
    /// Run an actual authorized frame through the configured S1 detector/watchdog, exact
    /// tokenizer/chat owner, sealed runner, durable journal and independent workspace verifier.
    /// This entry point neither selects nor grants a model, role, tool, mutation or test right.
    /// Callers construct those through their existing trusted host policies before invoking it.
    /// Without an S1 host the frame runs in [`SystemOneCoverage::Degraded`] mode.
    ///
    /// # Errors
    /// Foreign identity rejects before model work. Setup, provider, tool, verifier,
    /// cancellation and journal errors propagate; successful execution still checks actual proof.
    pub async fn run_agent_frame<M, T>(
        &self,
        input: FrameRunInput<'_>,
        services: ConfiguredFrameServices<M, T>,
    ) -> Result<ConfiguredFrameOutcome, ConfiguredFrameError>
    where
        M: ChatModel + PromptTokenizer,
        T: FrameToolAccess + 'static,
    {
        self.run_agent_frame_inner(input, services, ConfiguredFramePolicy::default())
            .await
    }

    /// Execute with required host-private marker-map recovery at every durable step boundary.
    /// The explicit store retains the host key owner and must belong to this actual frame.
    /// Checkpoint failures stop event publication; a failed event can leave an encrypted orphan.
    /// # Errors
    /// All ordinary configured-frame errors, plus private checkpoint identity/persistence errors.
    pub async fn run_agent_frame_with_checkpoints<M, T>(
        &self,
        input: FrameRunInput<'_>,
        services: ConfiguredFrameServices<M, T>,
        checkpoints: Arc<crate::PrivateFrameCheckpointStore>,
    ) -> Result<ConfiguredFrameOutcome, ConfiguredFrameError>
    where
        M: ChatModel + PromptTokenizer,
        T: FrameToolAccess + 'static,
    {
        self.run_agent_frame_inner(
            input,
            services,
            ConfiguredFramePolicy {
                checkpoints: Some(checkpoints),
                watchdog: None,
            },
        )
        .await
    }

    /// Execute with explicit recovery and measured watchdog policy using the same actual owners.
    /// # Errors
    /// Ordinary setup/execution errors, including stale calibration and checkpoint failures.
    pub async fn run_agent_frame_with_policy<M, T>(
        &self,
        input: FrameRunInput<'_>,
        services: ConfiguredFrameServices<M, T>,
        policy: ConfiguredFramePolicy,
    ) -> Result<ConfiguredFrameOutcome, ConfiguredFrameError>
    where
        M: ChatModel + PromptTokenizer,
        T: FrameToolAccess + 'static,
    {
        self.run_agent_frame_inner(input, services, policy).await
    }

    async fn run_agent_frame_inner<M, T>(
        &self,
        input: FrameRunInput<'_>,
        services: ConfiguredFrameServices<M, T>,
        policy: ConfiguredFramePolicy,
    ) -> Result<ConfiguredFrameOutcome, ConfiguredFrameError>
    where
        M: ChatModel + PromptTokenizer,
        T: FrameToolAccess + 'static,
    {
        let binding = input.frame.prompt_binding()?;
        if binding.root_frame_id() != self.frame()
            || services
                .runner
                .validate_prompt_binding(self, &binding, &services.token)
                .is_err()
        {
            return Err(ConfiguredFrameError::Identity);
        }
        let host = self.s1_host();
        let journal = KernelFrameJournal::new(
            self.clone(),
            input.frame.tool_binding(),
            FrameJournalLimits::default(),
        )?;
        let verifier = KernelFrameVerifier::new(self.clone(), input.frame.tool_binding())?;
        let prune_limit = u16::try_from(services.prompt.limits.max_tools.min(12))
            .map_err(|_| KernelPromptError::ToolPolicy)?;
        let assembler = KernelPromptAssembler::new_with_privacy(
            self.clone(),
            binding,
            services.token,
            services.runner,
            services.model,
            services.context,
            services.prompt,
        )
        .await?;
        let (assembler, system_one) = match host {
            Some(host) => {
                let assembler =
                    assembler.with_semantic_privacy(host.detector(), services.semantic_privacy)?;
                let assembler = match policy.watchdog {
                    Some(watchdog) => assembler.with_current_watchdog_policy(
                        watchdog.schemas,
                        services.role_table,
                        watchdog.calibration,
                    )?,
                    None => assembler.with_current_watchdog(services.role_table)?,
                }
                .with_current_tool_pruning(prune_limit)?;
                (assembler, SystemOneCoverage::Active)
            }
            // Degraded mode: the vault's deterministic masking stays fail-closed; the advisory
            // watchdog and pruning are fail-open, so they are simply not attached.
            None => (assembler, SystemOneCoverage::Degraded),
        };
        let journal = match policy.checkpoints {
            Some(checkpoints) => journal
                .with_private_checkpoints(checkpoints, Arc::clone(assembler.privacy_vault()))?,
            None => journal,
        };
        let FrameRunInput {
            frame,
            goal,
            cancellation,
            limits,
        } = input;
        let execution = assembler
            .run_frame(
                FrameRunInput {
                    frame,
                    goal,
                    cancellation,
                    limits,
                },
                &verifier,
                &journal,
            )
            .await?;
        let completion = match verifier.validate_completion(&frame.view()).await {
            Ok(proof) => FrameCompletionCheck::Verified(proof),
            Err(reason) => FrameCompletionCheck::Unverified(reason),
        };
        Ok(ConfiguredFrameOutcome {
            execution,
            completion,
            system_one,
            semantic_privacy: assembler.semantic_privacy_report()?,
            privacy_vault: Arc::clone(assembler.privacy_vault()),
        })
    }
}
