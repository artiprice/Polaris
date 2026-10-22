//! CAS-backed submission of validated diffs as unverified kernel commands.

use core::future::Future;
use std::{fs::File, io::Read};

use polaris_sdk::{
    contracts::{Command, CommandBody, TokenBytes, Unverified},
    ids::{Cid, CommandId, FrameId, SessionId},
    registry::{CommandReceipt, KernelApi, KernelError},
};
use polaris_storage_cas::{Cas, content_id};

use crate::{
    ToolLimits,
    patch::apply_unified_patch,
    path::{WorkspacePath, resolve_in_workspace},
};

/// Obtains the invocation's real capability token from the composition root.
///
/// Implementations must resolve the current token for the supplied session/frame. The builtin
/// crate never creates, substitutes, or caches a permit for proposal submission.
pub trait ProposalTokenSource: Send + Sync {
    /// Resolve a capability token for this command.
    fn token_for(
        &self,
        session: SessionId,
        frame: FrameId,
    ) -> impl Future<Output = Result<TokenBytes, KernelError>> + Send;
}

/// Exact unified diff prepared by the caller for one workspace-relative path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffProposal {
    /// Normalized workspace-relative path.
    pub path: String,
    /// Canonical single-file unified diff bytes.
    pub diff: Vec<u8>,
    /// BLAKE3 content id of the base bytes used to validate the patch.
    pub base: Cid,
}

/// Build a CAS-backed proposal sender using the real CAS and kernel interfaces.
pub struct CasKernelProposalBackend<K, T> {
    cas: Cas,
    kernel: K,
    tokens: T,
}

impl<K, T> CasKernelProposalBackend<K, T>
where
    K: KernelApi,
    T: ProposalTokenSource,
{
    /// Construct with the composition root's CAS, kernel, and per-invocation token source.
    #[must_use]
    pub fn new(cas: Cas, kernel: K, tokens: T) -> Self {
        Self {
            cas,
            kernel,
            tokens,
        }
    }

    /// Validate a patch against the current file, store exact diff bytes in CAS, then submit an
    /// unverified `ProposeDiff` command. This never writes the target file; verifier approval and
    /// the lease manager remain responsible for any later mutation.
    pub async fn submit_patch(
        &self,
        context: &polaris_sdk::registry::ToolContext<'_>,
        path: &str,
        patch: &str,
        expected_base: Option<Cid>,
        limits: &ToolLimits,
    ) -> Result<(DiffProposal, CommandReceipt), polaris_sdk::registry::ToolError> {
        if u64::try_from(patch.len()).map_or(true, |length| length > limits.max_output_bytes) {
            return Err(polaris_sdk::registry::ToolError::InvalidInput(
                "patch exceeds configured output bound".to_owned(),
            ));
        }
        let normalized = WorkspacePath::parse(path).map_err(|_| {
            polaris_sdk::registry::ToolError::InvalidInput("invalid proposal path".to_owned())
        })?;
        let resolved =
            resolve_in_workspace(context.workspace_root, normalized.as_str()).map_err(|_| {
                polaris_sdk::registry::ToolError::Forbidden("proposal path denied".to_owned())
            })?;
        let base_bytes = read_bounded_file(&resolved, limits.max_read_bytes)?;
        let base = content_id(&base_bytes);
        if expected_base.is_some_and(|expected| expected != base) {
            return Err(polaris_sdk::registry::ToolError::InvalidInput(
                "base content changed".to_owned(),
            ));
        }
        let diff_text = canonical_diff(normalized.as_str(), patch)?;
        let _candidate = apply_unified_patch(&base_bytes, &diff_text).map_err(|_| {
            polaris_sdk::registry::ToolError::InvalidInput(
                "patch does not apply to current file".to_owned(),
            )
        })?;
        let diff = diff_text.into_bytes();
        let cid = self.cas.put(&diff).map_err(|_| {
            polaris_sdk::registry::ToolError::Io("diff CAS store failed".to_owned())
        })?;
        let token = self
            .tokens
            .token_for(context.session, context.frame)
            .await
            .map_err(|_| {
                polaris_sdk::registry::ToolError::Forbidden("proposal token unavailable".to_owned())
            })?;
        let command = Command::<Unverified>::new(
            CommandId::new_v7(),
            context.frame,
            token,
            CommandBody::ProposeDiff { diff: cid },
        );
        let receipt = self
            .kernel
            .submit(command)
            .await
            .map_err(kernel_tool_error)?;
        Ok((
            DiffProposal {
                path: normalized.as_str().to_owned(),
                diff,
                base,
            },
            receipt,
        ))
    }
}

fn read_bounded_file(
    path: &std::path::Path,
    max_bytes: u64,
) -> Result<Vec<u8>, polaris_sdk::registry::ToolError> {
    let file = File::open(path).map_err(|_| {
        polaris_sdk::registry::ToolError::Io("proposal source open failed".to_owned())
    })?;
    let metadata = file.metadata().map_err(|_| {
        polaris_sdk::registry::ToolError::Io("proposal source metadata failed".to_owned())
    })?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err(polaris_sdk::registry::ToolError::InvalidInput(
            "proposal source exceeds configured bound".to_owned(),
        ));
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| {
        polaris_sdk::registry::ToolError::InvalidInput("proposal source is too large".to_owned())
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| {
            polaris_sdk::registry::ToolError::Io("proposal source read failed".to_owned())
        })?;
    if u64::try_from(bytes.len()).map_or(true, |length| length > max_bytes) {
        return Err(polaris_sdk::registry::ToolError::InvalidInput(
            "proposal source grew beyond configured bound".to_owned(),
        ));
    }
    Ok(bytes)
}

fn canonical_diff(path: &str, patch: &str) -> Result<String, polaris_sdk::registry::ToolError> {
    if path.contains('\n') || path.contains('\r') {
        return Err(polaris_sdk::registry::ToolError::InvalidInput(
            "invalid diff path".to_owned(),
        ));
    }
    let mut hunks = String::new();
    let mut in_hunks = false;
    for line in patch.lines() {
        if line.starts_with("@@ ") {
            in_hunks = true;
        }
        if in_hunks {
            hunks.push_str(line);
            hunks.push('\n');
        }
    }
    if hunks.is_empty() {
        return Err(polaris_sdk::registry::ToolError::InvalidInput(
            "patch has no unified hunk".to_owned(),
        ));
    }
    Ok(format!("--- a/{path}\n+++ b/{path}\n{hunks}"))
}

fn kernel_tool_error(error: KernelError) -> polaris_sdk::registry::ToolError {
    match error {
        KernelError::Rejected(_) => {
            polaris_sdk::registry::ToolError::Forbidden("kernel rejected diff proposal".to_owned())
        }
        KernelError::Busy => polaris_sdk::registry::ToolError::Timeout,
        KernelError::Closed | KernelError::Internal(_) => {
            polaris_sdk::registry::ToolError::Io("kernel submission failed".to_owned())
        }
    }
}
