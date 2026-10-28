//! Trusted composition of frame-bound mutation tools with the actual supervised S1 engine.
use crate::{FrameMutationConfig, KernelAgentToolRunner, KernelHandle};
use polaris_runtime::frames::FrameToolBinding;
use polaris_security::capability::PublicKeyBytes;
use polaris_system_one::{
    schemas::SchemaSet,
    verifier::{CalibrationBindings, SafetyScanner, Verifier, VerifierConfig},
};
use std::sync::Arc;

/// Native-process-resolving safety engine; it never retains a retired worker connection.
pub type ConfiguredMutationSafety = Verifier<ConfiguredDecisionEngine, SchemaSet, SafetyScanner>;

/// Shared actual S1 generation resolver used by configured safety consumers.
#[derive(Clone)]
pub struct ConfiguredDecisionEngine {
    engine: Arc<crate::CurrentS1Engine<polaris_platform::process::ProcessPlatform>>,
    checkpoint: Option<[u8; 32]>,
}
impl std::fmt::Debug for ConfiguredDecisionEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConfiguredDecisionEngine(<current supervised generation>)")
    }
}
impl ConfiguredDecisionEngine {
    pub(crate) fn new(
        engine: Arc<crate::CurrentS1Engine<polaris_platform::process::ProcessPlatform>>,
        checkpoint: Option<[u8; 32]>,
    ) -> Self {
        Self { engine, checkpoint }
    }
}
impl polaris_sdk::registry::DecisionEngine for ConfiguredDecisionEngine {
    async fn decide(
        &self,
        questions: &[polaris_sdk::system_one::Question],
        input: &polaris_sdk::system_one::DecisionInput,
    ) -> Result<polaris_sdk::system_one::Decision, polaris_sdk::registry::DecisionError> {
        let Some(checkpoint) = self.checkpoint else {
            return Err(polaris_sdk::registry::DecisionError::Unavailable(
                "calibrated checkpoint not configured".to_owned(),
            ));
        };
        self.engine
            .decide_pinned(checkpoint, questions, input)
            .await
    }
}

/// Complete kernel runner using actual reads, mutation receipts, human requests and sandbox tests.
/// Individual calls still require frame capabilities, role permission and any required host review.
pub type ConfiguredAgentToolRunner = KernelAgentToolRunner<ConfiguredMutationSafety>;

/// Sanitized failure constructing the production tool boundary.
#[derive(Debug, thiserror::Error)]
pub enum ConfiguredToolError {
    /// A real supervised System-One host has not been configured for this kernel.
    #[error("configured mutation tools require a System One worker")]
    MissingSystemOne,
    /// Scanner compilation, worker setup or verifier processing bounds were rejected.
    #[error("configured mutation safety initialization failed")]
    Safety,
    /// Actual frame, role, intent, cancellation or capability issuer binding was rejected.
    #[error("configured mutation tool binding rejected")]
    Binding,
}

impl polaris_sdk::error::HasKind for ConfiguredToolError {
    fn kind(&self) -> polaris_sdk::error::ErrorKind {
        match self {
            Self::MissingSystemOne | Self::Safety => polaris_sdk::error::ErrorKind::Permanent,
            Self::Binding => polaris_sdk::error::ErrorKind::Policy,
        }
    }
}

impl KernelHandle {
    /// Build the actual mutation/test/human/read runner without caller-injected safety verdicts.
    ///
    /// The trusted host supplies the schema catalog and expected calibration identities from its
    /// installed, measured artifacts. Empty or mismatched calibration remains a `NeedsApproval`
    /// outcome at the semantic boundary. This constructor never marks a schema calibrated,
    /// bypasses approval, issues a capability, or configures an execution backend implicitly.
    /// `run_tests` uses the kernel's separately configured sandbox execution/review service.
    ///
    /// # Errors
    /// Missing S1 host, foreign kernel root, scanner initialization, invalid verifier limits,
    /// cancelled frame or invalid mutation policy. No model/tool request is dispatched here.
    pub async fn configured_agent_tool_runner(
        &self,
        binding: FrameToolBinding,
        issuer: PublicKeyBytes,
        mutation: FrameMutationConfig,
        schemas: SchemaSet,
        safety_limits: VerifierConfig,
        calibration: CalibrationBindings,
    ) -> Result<ConfiguredAgentToolRunner, ConfiguredToolError> {
        if binding.root_frame_id() != self.frame() || mutation.cancellation.is_cancelled() {
            return Err(ConfiguredToolError::Binding);
        }
        let checkpoint = calibration
            .checkpoint_identity()
            .map_err(|_| ConfiguredToolError::Safety)?;
        let engine = self
            .s1_host()
            .ok_or(ConfiguredToolError::MissingSystemOne)?
            .engine();
        let owner = self
            .inspection_owner()
            .await
            .map_err(|_| ConfiguredToolError::Safety)?;
        let safety = tokio::task::spawn_blocking(move || {
            owner
                .ensure_open()
                .map_err(|_| ConfiguredToolError::Safety)?;
            let scanner = SafetyScanner::new().map_err(|_| ConfiguredToolError::Safety)?;
            Verifier::new(
                ConfiguredDecisionEngine::new(engine, checkpoint),
                schemas,
                scanner,
                safety_limits,
                calibration,
            )
            .map_err(|_| ConfiguredToolError::Safety)
        })
        .await
        .map_err(|_| ConfiguredToolError::Safety)??;
        KernelAgentToolRunner::new(self.clone(), binding, issuer, Arc::new(safety), mutation)
            .map_err(|_| ConfiguredToolError::Binding)
    }
}
