//! Bounded, non-sensitive loader errors; never include provider credentials or model prompts.
use polaris_sdk::error::{ErrorKind, HasKind};

/// Failure reported by an actual local model/worker loader.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// The real backend is not reachable or is temporarily out of resources.
    #[error("model loader unavailable")]
    Unavailable,
    /// Loader operation exceeded its configured duration.
    #[error("model loader operation timed out")]
    Timeout,
    /// Checkpoint format, provider configuration or requested execution mode is invalid.
    #[error("model loader rejected its configuration")]
    InvalidConfiguration,
    /// A load/unload task failed outside the normal backend result path.
    #[error("model loader task failed")]
    TaskFailed,
}

impl HasKind for LoadError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Unavailable | Self::Timeout => ErrorKind::Transient,
            Self::InvalidConfiguration => ErrorKind::Permanent,
            Self::TaskFailed => ErrorKind::Fatal,
        }
    }
}

/// Model selection, policy or lifecycle failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelsError {
    /// A name does not resolve to an explicitly registered model.
    #[error("model is not registered")]
    UnknownModel,
    /// Duplicate id, alias, invalid model type or malformed policy data.
    #[error("invalid model registry: {0}")]
    Config(&'static str),
    /// No model is allowed at the requested effort under the current constraints.
    #[error("requested model effort is not allowed")]
    EffortDenied,
    /// Real backend load/unload failure.
    #[error(transparent)]
    Load(#[from] LoadError),
    /// Admissions are paused while the model is being drained.
    #[error("model slot is draining or paused")]
    Draining,
    /// Permanent shutdown has closed the slot.
    #[error("model slot is closed")]
    Closed,
    /// Configured concurrent model lease bound was reached.
    #[error("model slot lease capacity reached")]
    LeaseLimit,
    /// State synchronization or private lease accounting was corrupted.
    #[error("model slot state integrity failure")]
    Integrity,
    /// A loader-owned operation requires an active Tokio runtime.
    #[error("model lifecycle requires an active runtime")]
    NoRuntime,
}

impl HasKind for ModelsError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::UnknownModel | Self::Config(_) | Self::NoRuntime => ErrorKind::Permanent,
            Self::EffortDenied => ErrorKind::Policy,
            Self::Load(error) => error.kind(),
            Self::Draining | Self::Closed | Self::LeaseLimit => ErrorKind::Transient,
            Self::Integrity => ErrorKind::Fatal,
        }
    }
}
