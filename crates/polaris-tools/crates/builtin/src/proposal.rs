//! CAS-backed submission of validated diffs as unverified kernel commands.

use core::{future::Future, pin::Pin};
use std::{io::Read, path::Path};

use cap_primitives::fs::FollowSymlinks;
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use polaris_sdk::{
    contracts::{Command, CommandBody, TokenBytes, Unverified},
    ids::{Cid, CommandId, FrameId, SessionId},
    registry::{CommandReceipt, KernelApi, KernelError},
};
use polaris_storage_cas::{Cas, content_id};
use serde_json::{Value, json};

use crate::{
    ToolLimits,
    patch::apply_unified_patch,
    path::{WorkspacePath, is_secret_path},
};

/// Obtains the invocation's real capability token from the composition root.
///
/// Implementations must resolve the current token for the supplied session/frame. The builtin
/// crate never creates, substitutes, or caches a permit for proposal submission.
pub trait ProposalTokenSource: Send + Sync {
    /// Resolve a capability token for this command.
    fn token_for<'a>(
        &'a self,
        session: SessionId,
        frame: FrameId,
    ) -> Pin<Box<dyn Future<Output = Result<TokenBytes, KernelError>> + Send + 'a>>;
}

impl<T: ProposalTokenSource + ?Sized> ProposalTokenSource for std::sync::Arc<T> {
    fn token_for<'a>(
        &'a self,
        session: SessionId,
        frame: FrameId,
    ) -> Pin<Box<dyn Future<Output = Result<TokenBytes, KernelError>> + Send + 'a>> {
        Box::pin(async move { self.as_ref().token_for(session, frame).await })
    }
}

/// The model-facing proposal operations supported by the P2 builtin registry.
///
/// Implementations must submit through the unverified kernel command path and must not persist
/// the candidate bytes directly. The expected base is mandatory so the caller cannot silently
/// overwrite a workspace version it did not inspect.
pub trait ProposalToolBackend: Send + Sync {
    /// Submit a complete replacement of an existing UTF-8 file as a kernel proposal.
    fn write_file(
        &self,
        context: &polaris_sdk::registry::ToolContext<'_>,
        path: &str,
        contents: &str,
        expected_base: Cid,
        limits: &ToolLimits,
    ) -> impl Future<Output = Result<Value, polaris_sdk::registry::ToolError>> + Send;

    /// Submit one exact-match edit as a kernel proposal.
    fn edit_file(
        &self,
        context: &polaris_sdk::registry::ToolContext<'_>,
        request: EditFileRequest<'_>,
        limits: &ToolLimits,
    ) -> impl Future<Output = Result<Value, polaris_sdk::registry::ToolError>> + Send;

    /// Submit a caller-provided patch against the expected current base.
    fn apply_patch(
        &self,
        context: &polaris_sdk::registry::ToolContext<'_>,
        path: &str,
        patch: &str,
        expected_base: Cid,
        limits: &ToolLimits,
    ) -> impl Future<Output = Result<Value, polaris_sdk::registry::ToolError>> + Send;
}

/// Exact-match edit request bound to a previously inspected file version.
#[derive(Debug, Clone, Copy)]
pub struct EditFileRequest<'a> {
    /// Workspace-relative target path.
    pub path: &'a str,
    /// Non-empty text that must match exactly.
    pub old_text: &'a str,
    /// Replacement text.
    pub new_text: &'a str,
    /// Replace every exact occurrence instead of requiring a unique match.
    pub replace_all: bool,
    /// Content id returned by the earlier read.
    pub expected_base: Cid,
}

/// P1 registry marker whose P2 proposal routes fail closed.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoProposalBackend;

impl ProposalToolBackend for NoProposalBackend {
    async fn write_file(
        &self,
        _context: &polaris_sdk::registry::ToolContext<'_>,
        _path: &str,
        _contents: &str,
        _expected_base: Cid,
        _limits: &ToolLimits,
    ) -> Result<Value, polaris_sdk::registry::ToolError> {
        Err(polaris_sdk::registry::ToolError::Forbidden(
            "proposal service is not configured".to_owned(),
        ))
    }

    async fn edit_file(
        &self,
        _context: &polaris_sdk::registry::ToolContext<'_>,
        _request: EditFileRequest<'_>,
        _limits: &ToolLimits,
    ) -> Result<Value, polaris_sdk::registry::ToolError> {
        Err(polaris_sdk::registry::ToolError::Forbidden(
            "proposal service is not configured".to_owned(),
        ))
    }

    async fn apply_patch(
        &self,
        _context: &polaris_sdk::registry::ToolContext<'_>,
        _path: &str,
        _patch: &str,
        _expected_base: Cid,
        _limits: &ToolLimits,
    ) -> Result<Value, polaris_sdk::registry::ToolError> {
        Err(polaris_sdk::registry::ToolError::Forbidden(
            "proposal service is not configured".to_owned(),
        ))
    }
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

/// Workspace facts reconstructed from an immutable, already-stored unified diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedDiff {
    /// Normalized workspace-relative target path.
    pub path: String,
    /// Exact base bytes observed through the workspace directory capability.
    pub old_source: Vec<u8>,
    /// Exact candidate bytes produced by applying the diff to the base snapshot.
    pub new_source: Vec<u8>,
}

/// Safely resolve a one-file diff against the current workspace and compute its candidate bytes.
/// This only reads files; callers still need the hybrid verifier and a lease/OCC boundary before
/// recording approval or performing any later mutation.
pub fn prepare_workspace_diff(
    root: &Path,
    diff: &[u8],
    max_source_bytes: u64,
) -> Result<PreparedDiff, polaris_sdk::registry::ToolError> {
    if u64::try_from(diff.len()).map_or(true, |size| size > max_source_bytes) {
        return Err(polaris_sdk::registry::ToolError::InvalidInput(
            "proposal diff exceeds configured bound".to_owned(),
        ));
    }
    let text = std::str::from_utf8(diff).map_err(|_| {
        polaris_sdk::registry::ToolError::InvalidInput("proposal diff is not UTF-8".to_owned())
    })?;
    let mut headers = text.lines();
    let old_path = headers
        .next()
        .and_then(|line| line.strip_prefix("--- a/"))
        .ok_or_else(|| {
            polaris_sdk::registry::ToolError::InvalidInput(
                "proposal diff header is invalid".to_owned(),
            )
        })?;
    let new_path = headers
        .next()
        .and_then(|line| line.strip_prefix("+++ b/"))
        .ok_or_else(|| {
            polaris_sdk::registry::ToolError::InvalidInput(
                "proposal diff header is invalid".to_owned(),
            )
        })?;
    let path = WorkspacePath::parse(old_path).map_err(|_| {
        polaris_sdk::registry::ToolError::InvalidInput("invalid proposal path".to_owned())
    })?;
    if old_path != new_path || path.as_str() != old_path || is_secret_path(&path) {
        return Err(polaris_sdk::registry::ToolError::Forbidden(
            "proposal path denied".to_owned(),
        ));
    }
    let old_source = read_bounded_workspace_file(root, path.as_str(), max_source_bytes)?;
    let new_source = apply_unified_patch(&old_source, text).map_err(|_| {
        polaris_sdk::registry::ToolError::InvalidInput(
            "patch does not apply to current file".to_owned(),
        )
    })?;
    Ok(PreparedDiff {
        path: path.as_str().to_owned(),
        old_source,
        new_source,
    })
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
        if is_secret_path(&normalized) {
            return Err(polaris_sdk::registry::ToolError::Forbidden(
                "proposal path denied".to_owned(),
            ));
        }
        let base_bytes = read_bounded_workspace_file(
            context.workspace_root,
            normalized.as_str(),
            limits.max_read_bytes,
        )?;
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

impl<K, T> ProposalToolBackend for CasKernelProposalBackend<K, T>
where
    K: KernelApi,
    T: ProposalTokenSource,
{
    async fn write_file(
        &self,
        context: &polaris_sdk::registry::ToolContext<'_>,
        path: &str,
        contents: &str,
        expected_base: Cid,
        limits: &ToolLimits,
    ) -> Result<Value, polaris_sdk::registry::ToolError> {
        let (resolved, base_bytes) = proposal_source(context, path, expected_base, limits)?;
        let diff = full_replacement_diff(path, &base_bytes, contents)?;
        let (proposal, _receipt) = self
            .submit_patch(context, &resolved, &diff, Some(expected_base), limits)
            .await?;
        Ok(proposal_output(&proposal))
    }

    async fn edit_file(
        &self,
        context: &polaris_sdk::registry::ToolContext<'_>,
        request: EditFileRequest<'_>,
        limits: &ToolLimits,
    ) -> Result<Value, polaris_sdk::registry::ToolError> {
        if request.old_text.is_empty() {
            return Err(polaris_sdk::registry::ToolError::InvalidInput(
                "edit text must be non-empty".to_owned(),
            ));
        }
        let (resolved, base_bytes) =
            proposal_source(context, request.path, request.expected_base, limits)?;
        let source = std::str::from_utf8(&base_bytes).map_err(|_| {
            polaris_sdk::registry::ToolError::InvalidInput(
                "proposal source is not UTF-8 text".to_owned(),
            )
        })?;
        let updated = crate::text_match::replace_text(
            source,
            request.old_text,
            request.new_text,
            request.replace_all,
        )
        .map_err(|error| polaris_sdk::registry::ToolError::InvalidInput(error.to_string()))?
        .text;
        let diff = full_replacement_diff(request.path, &base_bytes, &updated)?;
        let (proposal, _receipt) = self
            .submit_patch(
                context,
                &resolved,
                &diff,
                Some(request.expected_base),
                limits,
            )
            .await?;
        Ok(proposal_output(&proposal))
    }

    async fn apply_patch(
        &self,
        context: &polaris_sdk::registry::ToolContext<'_>,
        path: &str,
        patch: &str,
        expected_base: Cid,
        limits: &ToolLimits,
    ) -> Result<Value, polaris_sdk::registry::ToolError> {
        let (proposal, _receipt) = self
            .submit_patch(context, path, patch, Some(expected_base), limits)
            .await?;
        Ok(proposal_output(&proposal))
    }
}

fn proposal_source(
    context: &polaris_sdk::registry::ToolContext<'_>,
    path: &str,
    expected_base: Cid,
    limits: &ToolLimits,
) -> Result<(String, Vec<u8>), polaris_sdk::registry::ToolError> {
    let normalized = WorkspacePath::parse(path).map_err(|_| {
        polaris_sdk::registry::ToolError::InvalidInput("invalid proposal path".to_owned())
    })?;
    if is_secret_path(&normalized) {
        return Err(polaris_sdk::registry::ToolError::Forbidden(
            "proposal path denied".to_owned(),
        ));
    }
    let bytes = read_bounded_workspace_file(
        context.workspace_root,
        normalized.as_str(),
        limits.max_read_bytes,
    )?;
    if content_id(&bytes) != expected_base {
        return Err(polaris_sdk::registry::ToolError::InvalidInput(
            "base content changed".to_owned(),
        ));
    }
    Ok((normalized.as_str().to_owned(), bytes))
}

fn full_replacement_diff(
    path: &str,
    old: &[u8],
    new: &str,
) -> Result<String, polaris_sdk::registry::ToolError> {
    let old = std::str::from_utf8(old).map_err(|_| {
        polaris_sdk::registry::ToolError::InvalidInput(
            "proposal source is not UTF-8 text".to_owned(),
        )
    })?;
    if path.contains(['\r', '\n']) || new.contains('\0') {
        return Err(polaris_sdk::registry::ToolError::InvalidInput(
            "invalid proposal path or contents".to_owned(),
        ));
    }
    let old_lines = diff_lines(old);
    let new_lines = diff_lines(new);
    let old_start = if old_lines.is_empty() { 0 } else { 1 };
    let new_start = if new_lines.is_empty() { 0 } else { 1 };
    let mut patch = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -{old_start},{} +{new_start},{} @@\n",
        old_lines.len(),
        new_lines.len()
    );
    for (index, line) in old_lines.iter().enumerate() {
        patch.push('-');
        patch.push_str(line);
        patch.push('\n');
        if index == old_lines.len().saturating_sub(1) && !old.ends_with('\n') {
            patch.push_str("\\ No newline at end of file\n");
        }
    }
    for (index, line) in new_lines.iter().enumerate() {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
        if index == new_lines.len().saturating_sub(1) && !new.ends_with('\n') {
            patch.push_str("\\ No newline at end of file\n");
        }
    }
    Ok(patch)
}

fn diff_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    if text.ends_with('\n') {
        let _removed_final_empty = lines.pop();
    }
    lines
        .into_iter()
        .map(|line| match line.strip_suffix('\r') {
            Some(stripped) => stripped,
            None => line,
        })
        .collect()
}

fn proposal_output(proposal: &DiffProposal) -> Value {
    json!({
        "path": proposal.path,
        "base": proposal.base.to_hex(),
        "diff_cid": content_id(&proposal.diff).to_hex(),
        "submitted": true,
    })
}

fn read_bounded_workspace_file(
    root: &Path,
    relative: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, polaris_sdk::registry::ToolError> {
    let root_metadata = std::fs::symlink_metadata(root).map_err(|_| {
        polaris_sdk::registry::ToolError::Io("workspace root metadata failed".to_owned())
    })?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(polaris_sdk::registry::ToolError::Forbidden(
            "workspace root is not a trusted directory".to_owned(),
        ));
    }
    let canonical_root = root.canonicalize().map_err(|_| {
        polaris_sdk::registry::ToolError::Io("workspace root resolution failed".to_owned())
    })?;
    let mut directory =
        Dir::open_ambient_dir(&canonical_root, ambient_authority()).map_err(|_| {
            polaris_sdk::registry::ToolError::Io("workspace root open failed".to_owned())
        })?;
    let mut components = relative.split('/').peekable();
    while let Some(component) = components.next() {
        let metadata = directory.symlink_metadata(component).map_err(|_| {
            polaris_sdk::registry::ToolError::Io("proposal path resolution failed".to_owned())
        })?;
        if metadata.file_type().is_symlink() {
            return Err(polaris_sdk::registry::ToolError::Forbidden(
                "proposal path denied".to_owned(),
            ));
        }
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return Err(polaris_sdk::registry::ToolError::Forbidden(
                    "proposal parent is not a directory".to_owned(),
                ));
            }
            let parent = directory.into_std_file();
            let opened = cap_primitives::fs::open_dir_nofollow(&parent, Path::new(component))
                .map_err(|_| {
                    polaris_sdk::registry::ToolError::Forbidden("proposal path denied".to_owned())
                })?;
            directory = Dir::from_std_file(opened);
            continue;
        }
        if !metadata.is_file() || metadata.len() > max_bytes {
            return Err(polaris_sdk::registry::ToolError::InvalidInput(
                "proposal source exceeds configured bound".to_owned(),
            ));
        }
        let mut options = OpenOptions::new();
        options.read(true)._cap_fs_ext_follow(FollowSymlinks::No);
        let file = directory.open_with(component, &options).map_err(|_| {
            polaris_sdk::registry::ToolError::Forbidden("proposal path denied".to_owned())
        })?;
        let opened_metadata = file.metadata().map_err(|_| {
            polaris_sdk::registry::ToolError::Io("proposal source metadata failed".to_owned())
        })?;
        if !opened_metadata.is_file() {
            return Err(polaris_sdk::registry::ToolError::Forbidden(
                "proposal path denied".to_owned(),
            ));
        }
        let capacity = usize::try_from(opened_metadata.len()).map_err(|_| {
            polaris_sdk::registry::ToolError::InvalidInput(
                "proposal source is too large".to_owned(),
            )
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
        return Ok(bytes);
    }
    Err(polaris_sdk::registry::ToolError::InvalidInput(
        "proposal path is empty".to_owned(),
    ))
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

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-tools/crates/builtin/src/proposal.rs"]
mod tests;
