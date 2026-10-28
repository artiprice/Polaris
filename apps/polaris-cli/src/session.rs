//! Trusted asynchronous adapters over the real local kernel, registry and bounded CAS.
//!
//! The adapter itself lives in `polaris-ui-session` so the desktop frontend shares it; this
//! module re-exports it and maps its error into [`CliError`] without changing any message.
use crate::CliError;
pub use polaris_ui_session::{OfflineSession, RecordTextKind, ResolvedRecord, SessionHostError};

impl From<SessionHostError> for CliError {
    fn from(error: SessionHostError) -> Self {
        match error {
            SessionHostError::Ui(message) => Self::Ui(message),
            SessionHostError::Unavailable(message) => Self::Unavailable(message),
            SessionHostError::Kernel(error) => Self::Kernel(error),
            SessionHostError::Control(error) => Self::Control(error),
        }
    }
}
