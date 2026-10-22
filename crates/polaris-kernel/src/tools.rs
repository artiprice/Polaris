//! P1 readonly tool composition against the active kernel's real stores.
use crate::KernelCtx;
use polaris_sdk::{ids::SessionId, registry::ToolError};
use polaris_tools::builtin::{CasRefResolver, RecallBackend, ToolLimits, ToolRegistry};
use std::{sync::Arc, time::Duration};

/// Session-bound history adapter. SQL filtering happens before the result limit.
#[derive(Debug)]
pub struct KernelRecall {
    ctx: Arc<KernelCtx>,
    session: SessionId,
}
impl RecallBackend for KernelRecall {
    fn recall(&self, query: &str, max_results: usize) -> Result<serde_json::Value, ToolError> {
        self.ctx
            .ensure_open()
            .map_err(|_| ToolError::Io("kernel closed".to_owned()))?;
        let limit = u32::try_from(max_results)
            .map_err(|_| ToolError::InvalidInput("recall result bound is too large".to_owned()))?;
        let records = self
            .ctx
            .registry
            .search_history_for_session(self.session, query, limit)
            .map_err(|_| ToolError::Io("history search failed".to_owned()))?;
        Ok(
            serde_json::json!({"session":self.session,"records":records,"source":"persisted_session_history"}),
        )
    }
}
/// Concrete P1 inspection tools, sharing the kernel's CAS and session history.
pub type LocalTools = ToolRegistry<CasRefResolver, KernelRecall>;
pub(crate) fn build(ctx: Arc<KernelCtx>, session: SessionId) -> Result<LocalTools, ToolError> {
    let limits = ToolLimits {
        max_read_bytes: 1 << 20,
        max_results: 256,
        max_output_bytes: 2 << 20,
        max_input_bytes: 64 << 10,
        regex_size_limit: 2 << 20,
        timeout: Duration::from_secs(5),
    };
    ToolRegistry::new(
        limits,
        CasRefResolver::new(ctx.cas.clone()),
        KernelRecall { ctx, session },
    )
}
