//! Frame-bound, verifier-gated workspace mutation over the real kernel CAS and event log.
//!
//! Filesystem containment follows the builtin resolver's stable-workspace assumption; hostile
//! concurrent replacement of directory entries is outside that guarantee. Once admitted to a
//! blocking owner, dropping the async caller does not drop the owner or the file lease.

use crate::{
    KernelError, KernelEvidence, KernelHandle,
    runtime::InspectionOwner,
    storage::cas::content_id,
    verifier::{
        ApprovalReason, HumanApproval, HybridMutationVerifier, MutationRequest, Rejection,
        VerificationDecision, VerifierConfig,
    },
};
use polaris_code_intel::{CodeIntelError, ParserPool, analyze_safety, churn_lines, metrics};
use polaris_runtime::{
    frames::{FrameToolBinding, role_allows_tool},
    leases::{LeaseGuard, LeaseKey, LeaseManager, LeaseRequest, LeaseScope},
};
use polaris_sdk::{
    contracts::{Command, CommandBody, Event, TokenBytes, Unverified},
    ids::{Cid, CommandId, FrameId},
    registry::Effects,
};
use polaris_security::capability::{
    Authorization, Facts, PublicKeyBytes, ResourcePath, Right, SystemClock, Token, authorize,
};
use polaris_system_one::verifier::{MutationSafety, SafetyRequest};
use polaris_tools::builtin::{
    MatchLevel, WorkspacePath, apply_unified_patch, is_secret_path, replace_text,
    resolve_in_workspace,
};
use serde::Serialize;
use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

const MAX_PATH_BYTES: usize = 4096;
const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DIFF_BYTES: usize = 4 * 1024 * 1024;
const MAX_INTENT_BYTES: usize = 16 * 1024;

/// Typed mutation proposal. `expected_base: None` means the target must not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameMutationRequest {
    /// Replace or create one UTF-8 file. Unsupported newline conversions are rejected.
    WriteFile {
        /// Normalized workspace-relative target.
        path: String,
        /// Exact proposed UTF-8 bytes.
        contents: String,
        /// Current file digest; absence means create-new only.
        expected_base: Option<Cid>,
    },
    /// Replace the first exact text occurrence, or all occurrences when requested.
    EditFile {
        /// Normalized workspace-relative target.
        path: String,
        /// Nonempty exact source text to replace.
        old_text: String,
        /// Replacement text.
        new_text: String,
        /// Replace all matching occurrences when true.
        replace_all: bool,
        /// Digest of the exact existing source.
        expected_base: Cid,
    },
    /// Apply one bounded unified patch to an existing file.
    ApplyPatch {
        /// Normalized workspace-relative target named by the patch headers.
        path: String,
        /// Single-file unified patch.
        patch: String,
        /// Digest of the exact existing source.
        expected_base: Cid,
    },
}

/// Verified result persisted after the actual workspace bytes are committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameMutationReceipt {
    command: CommandId,
    frame: FrameId,
    path: String,
    base: Option<Cid>,
    new_cid: Cid,
    diff: Cid,
    approval_evidence: Cid,
    receipt_cid: Cid,
}

impl FrameMutationReceipt {
    /// Command identity checked by the hybrid verifier.
    #[must_use]
    pub const fn command(&self) -> CommandId {
        self.command
    }
    /// Proposing frame identity.
    #[must_use]
    pub const fn frame(&self) -> FrameId {
        self.frame
    }
    /// Normalized workspace-relative target.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
    /// Original file CID, or `None` for a create-new operation.
    #[must_use]
    pub const fn base(&self) -> Option<Cid> {
        self.base
    }
    /// CID of the bytes read back from the committed file.
    #[must_use]
    pub const fn new_cid(&self) -> Cid {
        self.new_cid
    }
    /// CID of the exact unified diff verified by the hybrid verifier.
    #[must_use]
    pub const fn diff(&self) -> Cid {
        self.diff
    }
    /// CAS evidence created by the real hybrid verifier after approval.
    #[must_use]
    pub const fn approval_evidence(&self) -> Cid {
        self.approval_evidence
    }
    /// CID of the durable read-back-verified receipt record.
    #[must_use]
    pub const fn cid(&self) -> Cid {
        self.receipt_cid
    }
}

/// Immutable proposal bound to one service, token, frame and target.
pub struct PreparedFrameMutation {
    state: Arc<MutationState>,
    command: Command<Unverified>,
    path: String,
    base: Option<Cid>,
    lease_base: Cid,
    old: Vec<u8>,
    new: Vec<u8>,
    diff_bytes: Vec<u8>,
    token_cid: Cid,
    symbol: Option<String>,
    churn_lines: u32,
    touches_unsafe: bool,
    mutation_binding: Cid,
    /// How an `edit_file` target was located; `None` for whole-file writes and patches.
    match_level: Option<MatchLevel>,
}

impl PreparedFrameMutation {
    /// Normalized workspace-relative target.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
    /// Exact base snapshot; empty when the proposal creates the file.
    #[must_use]
    pub fn old_source(&self) -> &[u8] {
        &self.old
    }
    /// Exact proposed bytes.
    #[must_use]
    pub fn new_source(&self) -> &[u8] {
        &self.new
    }
    /// Digest binding command, frame, target, diff and both snapshots; a human request for this
    /// proposal names it so the answer cannot be replayed onto another mutation.
    #[must_use]
    pub const fn binding(&self) -> Cid {
        self.mutation_binding
    }
    /// The same proposal, kept while a human decides on the original's escalation.
    pub(crate) fn review_copy(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            command: self.command.clone(),
            path: self.path.clone(),
            base: self.base,
            lease_base: self.lease_base,
            old: self.old.clone(),
            new: self.new.clone(),
            diff_bytes: self.diff_bytes.clone(),
            token_cid: self.token_cid,
            symbol: self.symbol.clone(),
            churn_lines: self.churn_lines,
            touches_unsafe: self.touches_unsafe,
            mutation_binding: self.mutation_binding,
            match_level: self.match_level,
        }
    }
}

impl fmt::Debug for PreparedFrameMutation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedFrameMutation")
            .field("command", &self.command.id())
            .field("frame", &self.command.frame())
            .field("path", &self.path)
            .field("base", &self.base)
            .field("diff", &content_id(&self.diff_bytes))
            .finish_non_exhaustive()
    }
}

struct MutationState {
    kernel: KernelHandle,
    binding: FrameToolBinding,
    issuer: PublicKeyBytes,
    leases: LeaseManager,
    safety: Arc<dyn SafetyBoundary>,
    config: VerifierConfig,
    intent: String,
    cancellation: CancellationToken,
    _cancellation_owner: crate::frame_tools::RunnerCancellation,
}

impl fmt::Debug for MutationState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MutationState(<redacted>)")
    }
}

// Object safety is not available for MutationSafety's RPIT method; this wrapper keeps `S`
// generic at construction while sharing it with the blocking worker.
trait SafetyBoundary: Send + Sync + fmt::Debug {
    fn assess_owned<'a>(
        &'a self,
        request: &'a SafetyRequest<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        polaris_system_one::verifier::SafetyAssessment,
                        polaris_system_one::verifier::VerifierError,
                    >,
                > + Send
                + 'a,
        >,
    >;
}

struct SharedSafety<S>(Arc<S>);
impl<S: MutationSafety> SafetyBoundary for SharedSafety<S> {
    fn assess_owned<'a>(
        &'a self,
        request: &'a SafetyRequest<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        polaris_system_one::verifier::SafetyAssessment,
                        polaris_system_one::verifier::VerifierError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(self.0.assess(request))
    }
}
impl<S> fmt::Debug for SharedSafety<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedSafety(<redacted>)")
    }
}

/// Trusted mutation policy, semantic verification limits and frame cancellation owner.
#[derive(Clone)]
pub struct FrameMutationConfig {
    /// Limits and deadline for the real hybrid verifier.
    pub verifier: VerifierConfig,
    /// Trusted task intent supplied to semantic assessment, never model tool arguments.
    pub intent: String,
    /// The actual cancellation signal owned by this frame's runtime.
    pub cancellation: CancellationToken,
}
impl fmt::Debug for FrameMutationConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FrameMutationConfig(<redacted>)")
    }
}

/// Production mutation service pinned to the actual kernel, frame binding, issuer, leases and S1.
#[derive(Debug)]
pub struct FrameMutationService<S> {
    state: Arc<MutationState>,
    _safety_type: std::marker::PhantomData<fn() -> S>,
}

impl<S: MutationSafety + 'static> FrameMutationService<S> {
    /// Bind an actual workspace and verifier stack to a sealed frame identity.
    /// # Errors
    /// Rejects a foreign root, excessive intent, or cancelled frame.
    pub fn new(
        kernel: KernelHandle,
        binding: FrameToolBinding,
        issuer: PublicKeyBytes,
        safety: Arc<S>,
        policy: FrameMutationConfig,
    ) -> Result<Self, FrameMutationError> {
        let leases = kernel.workspace_leases();
        let FrameMutationConfig {
            verifier: config,
            intent,
            cancellation,
        } = policy;
        if !role_allows_tool(
            binding.role(),
            Effects {
                writes_workspace: true,
                ..Effects::READ_ONLY
            },
        ) || binding.root_frame_id() != kernel.frame()
            || intent.is_empty()
            || intent.len() > MAX_INTENT_BYTES
            || cancellation.is_cancelled()
        {
            return Err(FrameMutationError::Binding);
        }
        let cancellation_owner =
            crate::frame_tools::RunnerCancellation::new(&kernel, binding.clone(), cancellation)
                .map_err(|_| FrameMutationError::Binding)?;
        let cancellation = cancellation_owner.token();
        let boundary = Arc::new(SharedSafety(safety));
        Ok(Self {
            state: Arc::new(MutationState {
                kernel,
                binding,
                issuer,
                leases,
                safety: boundary,
                config,
                intent,
                cancellation,
                _cancellation_owner: cancellation_owner,
            }),
            _safety_type: std::marker::PhantomData,
        })
    }

    /// Resolve and analyze an exact candidate without writing the workspace or CAS.
    /// # Errors
    /// Capability, frame, path, size, parser, or workspace checks fail.
    pub async fn prepare(
        &self,
        token: &Token,
        request: FrameMutationRequest,
    ) -> Result<PreparedFrameMutation, FrameMutationError> {
        self.prepare_with_id(token, CommandId::new_v7(), request)
            .await
    }

    /// Prepare using the command id already authorized by the trusted capability issuer. This is
    /// required when the issuer pins `Scope::diff_blake3` to the exact mutation digest.
    /// # Errors
    /// Capability, frame, path, size, parser, or workspace checks fail.
    pub async fn prepare_with_id(
        &self,
        token: &Token,
        command_id: CommandId,
        request: FrameMutationRequest,
    ) -> Result<PreparedFrameMutation, FrameMutationError> {
        self.check_token(token)?;
        let owner = self
            .state
            .kernel
            .inspection_owner()
            .await
            .map_err(|_| FrameMutationError::Owner)?;
        let state = Arc::clone(&self.state);
        let token = token.clone();
        tokio::task::spawn_blocking(move || prepare_owned(state, owner, token, command_id, request))
            .await
            .map_err(|_| FrameMutationError::Owner)?
    }

    /// Run real hybrid verification, preserve the lease through compare-and-swap commit, and
    /// return a receipt only after actual bytes and durable CAS evidence have been read back.
    /// # Errors
    /// Any deterministic denial, stale base, explicit review requirement, verifier error, or I/O error.
    pub async fn apply(
        &self,
        prepared: PreparedFrameMutation,
        token: &Token,
    ) -> Result<FrameMutationReceipt, FrameMutationError> {
        self.apply_reviewed(prepared, token, None).await
    }

    /// Apply a proposal whose escalation a human approved. The caller must have created the
    /// human request for this exact proposal (see [`PreparedFrameMutation::binding`]) and checked
    /// that the committed answer approves it. Every deterministic check still runs under the
    /// write lease, and a semantic block still rejects.
    /// # Errors
    /// As [`Self::apply`], except that an escalation is accepted instead of returned.
    pub async fn apply_with_human_approval(
        &self,
        prepared: PreparedFrameMutation,
        token: &Token,
        approval: HumanApproval,
    ) -> Result<FrameMutationReceipt, FrameMutationError> {
        self.apply_reviewed(prepared, token, Some(approval)).await
    }

    async fn apply_reviewed(
        &self,
        prepared: PreparedFrameMutation,
        token: &Token,
        human: Option<HumanApproval>,
    ) -> Result<FrameMutationReceipt, FrameMutationError> {
        self.check_token(token)?;
        if !Arc::ptr_eq(&prepared.state, &self.state)
            || prepared.token_cid != content_id(token.to_bytes().as_bytes())
            || prepared.command.frame() != self.state.binding.frame_id()
        {
            return Err(FrameMutationError::Binding);
        }
        let key = LeaseKey::from_resolved_relative(&prepared.path)
            .map_err(|_| FrameMutationError::Path)?;
        let shutdown = self.state.kernel.shutdown_token();
        // A queued writer must not consume an immutable-read slot. Both frame cancellation
        // and kernel shutdown remove the lease waiter before any inspection or file mutation.
        let guard = tokio::select! {
            biased;
            () = self.state.cancellation.cancelled() => return Err(FrameMutationError::Binding),
            () = shutdown.cancelled() => return Err(FrameMutationError::Owner),
            result = self.state.leases.acquire(LeaseRequest {
                agent: self.state.binding.frame_id(), key, scope: LeaseScope::EntireFile,
                base: Some(prepared.lease_base),
            }) => result.map_err(|_| FrameMutationError::Lease)?,
        };
        self.check_token(token)?;
        let owner = self
            .state
            .kernel
            .inspection_owner()
            .await
            .map_err(|_| FrameMutationError::Owner)?;
        let state = Arc::clone(&self.state);
        let command_token = token.clone();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            apply_owned(state, owner, guard, handle, prepared, command_token, human)
        })
        .await
        .map_err(|_| FrameMutationError::Owner)?
    }

    fn check_token(&self, token: &Token) -> Result<(), FrameMutationError> {
        if self.state.cancellation.is_cancelled() || !self.state.binding.matches_token(token) {
            return Err(FrameMutationError::Binding);
        }
        Ok(())
    }
}

fn prepare_owned(
    state: Arc<MutationState>,
    owner: InspectionOwner,
    token: Token,
    command_id: CommandId,
    request: FrameMutationRequest,
) -> Result<PreparedFrameMutation, FrameMutationError> {
    owner.ensure_open().map_err(|_| FrameMutationError::Owner)?;
    if state.cancellation.is_cancelled() {
        return Err(FrameMutationError::Binding);
    }
    let (raw_path, expected_base, old_text, new_text, mode) = match request {
        FrameMutationRequest::WriteFile {
            path,
            contents,
            expected_base,
        } => (path, expected_base, None, contents, ProposalMode::Write),
        FrameMutationRequest::EditFile {
            path,
            old_text,
            new_text,
            replace_all,
            expected_base,
        } => (
            path,
            Some(expected_base),
            Some((old_text, replace_all)),
            new_text,
            ProposalMode::Edit,
        ),
        FrameMutationRequest::ApplyPatch {
            path,
            patch,
            expected_base,
        } => (path, Some(expected_base), None, patch, ProposalMode::Patch),
    };
    if raw_path.len() > MAX_PATH_BYTES {
        return Err(FrameMutationError::Limit);
    }
    let normalized = WorkspacePath::parse(&raw_path).map_err(|_| FrameMutationError::Path)?;
    if is_secret_path(&normalized)
        || normalized
            .as_str()
            .split('/')
            .any(|part| part.eq_ignore_ascii_case(".git") || part.eq_ignore_ascii_case(".polaris"))
    {
        return Err(FrameMutationError::Path);
    }
    let target = normalized.as_str().to_owned();
    let root = owner.workspace_root().to_owned();
    let MutationCandidate {
        base,
        old,
        new,
        match_level,
    } = snapshot_candidate(&root, &target, expected_base, old_text, new_text, mode)?;
    if old.len() > MAX_SOURCE_BYTES || new.len() > MAX_SOURCE_BYTES {
        return Err(FrameMutationError::Limit);
    }
    let diff_bytes = render_diff(&target, &old, &new)?;
    if diff_bytes.len() > MAX_DIFF_BYTES {
        return Err(FrameMutationError::Limit);
    }
    let (symbol, churn_lines, touches_unsafe) = candidate_facts(&target, &old, &new)?;
    let diff = content_id(&diff_bytes);
    // The token fact must match the hybrid verifier's content identity for its empty old snapshot;
    // `base: None` and `lease_base` separately preserve the actual absence semantics.
    let old_cid = content_id(&old);
    let new_cid = content_id(&new);
    let mutation_binding = crate::verifier::mutation_digest(
        command_id,
        state.binding.frame_id(),
        &target,
        diff,
        old_cid,
        new_cid,
    )
    .map_err(|_| FrameMutationError::Analysis)?;
    let resource = ResourcePath::new(&target).map_err(|_| FrameMutationError::Path)?;
    let facts = Facts {
        resource: &resource,
        symbol: symbol.as_deref(),
        churn_lines,
        touches_unsafe,
        diff_blake3: Some(mutation_binding),
        right: Right::ProposeDiff,
    };
    match authorize(&token, &facts, &state.issuer, &SystemClock)
        .map_err(|_| FrameMutationError::Authorization)?
    {
        Authorization::Allowed => {}
        Authorization::Denied { .. } => return Err(FrameMutationError::Authorization),
    }
    let command = Command::new(
        command_id,
        state.binding.frame_id(),
        TokenBytes::from(token.to_bytes().as_bytes().to_vec()),
        CommandBody::ProposeDiff { diff },
    );
    let _ = owner;
    Ok(PreparedFrameMutation {
        state: Arc::clone(&state),
        command,
        path: target,
        base,
        lease_base: match base {
            Some(cid) => cid,
            None => absent_base(),
        },
        old,
        new,
        diff_bytes,
        token_cid: content_id(token.to_bytes().as_bytes()),
        symbol,
        churn_lines,
        touches_unsafe,
        mutation_binding,
        match_level,
    })
}

/// Capability facts for a candidate: the single changed symbol, line churn and whether the
/// unsafe inventory changed. A language the analyzer cannot parse has no symbols and no unsafe
/// inventory; the hybrid verifier then asks for a human decision.
fn candidate_facts(
    target: &str,
    old: &[u8],
    new: &[u8],
) -> Result<(Option<String>, u32, bool), FrameMutationError> {
    let parser = ParserPool::new();
    let parsed = (
        parser.parse_file(Path::new(target), old),
        parser.parse_file(Path::new(target), new),
    );
    let (old_parsed, new_parsed) = match parsed {
        (Ok(old_parsed), Ok(new_parsed)) => (old_parsed, new_parsed),
        (Err(CodeIntelError::UnsupportedLanguage), _)
        | (_, Err(CodeIntelError::UnsupportedLanguage)) => {
            return Ok((None, churn_lines(old, new), false));
        }
        _ => return Err(FrameMutationError::Analysis),
    };
    let measured = metrics(&old_parsed, &new_parsed).map_err(|_| FrameMutationError::Analysis)?;
    let old_safety = analyze_safety(&old_parsed).map_err(|_| FrameMutationError::Analysis)?;
    let new_safety = analyze_safety(&new_parsed).map_err(|_| FrameMutationError::Analysis)?;
    let symbol = match measured.changed_symbols.as_slice() {
        [symbol] => Some(symbol.name.clone()),
        _ => None,
    };
    Ok((
        symbol,
        measured.churn_lines,
        unsafe_observations(&old_parsed, &old_safety)
            != unsafe_observations(&new_parsed, &new_safety),
    ))
}

#[derive(Debug, Clone, Copy)]
enum ProposalMode {
    Write,
    Edit,
    Patch,
}

struct MutationCandidate {
    base: Option<Cid>,
    old: Vec<u8>,
    new: Vec<u8>,
    match_level: Option<MatchLevel>,
}

fn snapshot_candidate(
    root: &Path,
    target: &str,
    expected: Option<Cid>,
    old_text: Option<(String, bool)>,
    input: String,
    mode: ProposalMode,
) -> Result<MutationCandidate, FrameMutationError> {
    if old_text
        .as_ref()
        .is_some_and(|(text, _)| text.len() > MAX_SOURCE_BYTES)
    {
        return Err(FrameMutationError::Limit);
    }
    let existing = resolve_in_workspace(root, target);
    let (_path, old, base) = match existing {
        Ok(path) => {
            let metadata =
                fs::symlink_metadata(&path).map_err(|_| FrameMutationError::Workspace)?;
            if !metadata.is_file() {
                return Err(FrameMutationError::Path);
            }
            let old = read_source_bounded(&path)?;
            let base = content_id(&old);
            if expected != Some(base) {
                return Err(FrameMutationError::StaleBase);
            }
            (path, old, Some(base))
        }
        Err(_) if expected.is_none() && matches!(mode, ProposalMode::Write) => {
            let candidate = root.join(target);
            if fs::symlink_metadata(&candidate).is_ok() {
                return Err(FrameMutationError::Path);
            }
            let parent = Path::new(target).parent().ok_or(FrameMutationError::Path)?;
            let parent = if parent.as_os_str().is_empty() {
                "."
            } else {
                parent.to_str().ok_or(FrameMutationError::Path)?
            };
            let parent_path = resolve_parent(root, parent)?;
            let file_name = Path::new(target)
                .file_name()
                .ok_or(FrameMutationError::Path)?;
            (parent_path.join(file_name), Vec::new(), None)
        }
        Err(_) => return Err(FrameMutationError::StaleBase),
    };
    if old.len() > MAX_SOURCE_BYTES || input.len() > MAX_SOURCE_BYTES {
        return Err(FrameMutationError::Limit);
    }
    let mut match_level = None;
    let new = match mode {
        ProposalMode::Write => input.into_bytes(),
        ProposalMode::Edit => {
            let (needle, replace_all) = old_text.ok_or(FrameMutationError::InvalidProposal)?;
            let source =
                std::str::from_utf8(&old).map_err(|_| FrameMutationError::InvalidProposal)?;
            let replacement = replace_text(source, &needle, &input, replace_all)
                .map_err(|error| FrameMutationError::EditTarget(error.to_string()))?;
            match_level = Some(replacement.level);
            replacement.text.into_bytes()
        }
        ProposalMode::Patch => {
            let patch = input;
            if !patch_header_matches(&patch, target) {
                return Err(FrameMutationError::InvalidProposal);
            }
            apply_unified_patch(&old, &patch).map_err(|_| FrameMutationError::InvalidProposal)?
        }
    };
    if old == new {
        return Err(FrameMutationError::InvalidProposal);
    }
    Ok(MutationCandidate {
        base,
        old,
        new,
        match_level,
    })
}

fn patch_header_matches(patch: &str, target: &str) -> bool {
    let old = format!("--- a/{target}");
    let new = format!("+++ b/{target}");
    patch.lines().any(|line| line == old) && patch.lines().any(|line| line == new)
}

fn render_diff(path: &str, old: &[u8], new: &[u8]) -> Result<Vec<u8>, FrameMutationError> {
    let old_text = std::str::from_utf8(old).map_err(|_| FrameMutationError::InvalidProposal)?;
    let new_text = std::str::from_utf8(new).map_err(|_| FrameMutationError::InvalidProposal)?;
    let old_lines: Vec<&str> = old_text.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new_text.split_inclusive('\n').collect();
    let old_start = if old_lines.is_empty() { 0 } else { 1 };
    let new_start = if new_lines.is_empty() { 0 } else { 1 };
    let mut out = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -{old_start},{} +{new_start},{} @@\n",
        old_lines.len(),
        new_lines.len()
    );
    for (marker, lines) in [('-', old_lines), ('+', new_lines)] {
        for line in lines {
            out.push(marker);
            out.push_str(line);
            if !line.ends_with('\n') {
                out.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
    // The builtin patch interpreter preserves existing newline conventions. Unsupported newline
    // conversions must be rejected rather than recording a diff for bytes we did not write.
    if apply_unified_patch(old, &out).map_err(|_| FrameMutationError::InvalidProposal)? != new {
        return Err(FrameMutationError::InvalidProposal);
    }
    Ok(out.into_bytes())
}

fn read_source_bounded(path: &Path) -> Result<Vec<u8>, FrameMutationError> {
    let file = fs::File::open(path).map_err(|_| FrameMutationError::Workspace)?;
    let metadata = file.metadata().map_err(|_| FrameMutationError::Workspace)?;
    if !metadata.is_file() {
        return Err(FrameMutationError::Path);
    }
    let limit = u64::try_from(MAX_SOURCE_BYTES).map_err(|_| FrameMutationError::Limit)?;
    if metadata.len() > limit {
        return Err(FrameMutationError::Limit);
    }
    let mut bytes = Vec::new();
    file.take(limit.checked_add(1).ok_or(FrameMutationError::Limit)?)
        .read_to_end(&mut bytes)
        .map_err(|_| FrameMutationError::Workspace)?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(FrameMutationError::Limit);
    }
    Ok(bytes)
}

fn absent_base() -> Cid {
    content_id(b"polaris.frame-mutation.absent.v1\0")
}

fn unsafe_observations(
    parsed: &polaris_code_intel::Parsed,
    analysis: &polaris_code_intel::SafetyAnalysis,
) -> std::collections::BTreeMap<(String, Vec<u8>), u32> {
    let mut observations = std::collections::BTreeMap::new();
    for observation in &analysis.unsafe_constructs {
        if let Some(source) = observation.span.slice(parsed.source()) {
            let count = observations
                .entry((observation.node_kind.clone(), source.to_vec()))
                .or_insert(0_u32);
            *count = count.saturating_add(1);
        }
    }
    observations
}

fn apply_owned(
    state: Arc<MutationState>,
    owner: InspectionOwner,
    guard: LeaseGuard,
    runtime: tokio::runtime::Handle,
    prepared: PreparedFrameMutation,
    token: Token,
    human: Option<HumanApproval>,
) -> Result<FrameMutationReceipt, FrameMutationError> {
    owner.ensure_open().map_err(|_| FrameMutationError::Owner)?;
    if state.cancellation.is_cancelled()
        || !state.binding.matches_token(&token)
        || content_id(token.to_bytes().as_bytes()) != prepared.token_cid
    {
        return Err(FrameMutationError::Binding);
    }
    let current = read_current(owner.workspace_root(), &prepared.path)?;
    if match current.as_ref() {
        Some(bytes) => content_id(bytes),
        None => absent_base(),
    } != prepared.lease_base
    {
        return Err(FrameMutationError::StaleBase);
    }
    let ticket = state
        .leases
        .check_write(&guard, prepared.lease_base)
        .map_err(|_| FrameMutationError::Lease)?;
    let verifier = HybridMutationVerifier::new(
        state.issuer,
        state.config,
        ArcSafety(Arc::clone(&state.safety)),
        SystemClock,
        KernelEvidence::new(Arc::clone(state.kernel.context())),
    );
    let request = MutationRequest {
        command: &prepared.command,
        target: &prepared.path,
        old_source: &prepared.old,
        new_source: &prepared.new,
        diff: &prepared.diff_bytes,
        intent: &state.intent,
    };
    let decision = runtime
        .block_on(async {
            match human {
                Some(approval) => verifier.verify_with_human_approval(request, approval).await,
                None => verifier.verify(request).await,
            }
        })
        .map_err(|_| FrameMutationError::Verifier)?;
    match decision {
        VerificationDecision::Rejected(rejection) => Err(FrameMutationError::Rejected(rejection)),
        // An approved proposal never escalates again; anything else would be a verifier defect.
        VerificationDecision::NeedsApproval { .. } if human.is_some() => {
            Err(FrameMutationError::Verifier)
        }
        VerificationDecision::NeedsApproval { reason } => {
            let diff = persist_bytes(owner.cas(), &prepared.diff_bytes)?;
            let report = persist_json(
                owner.cas(),
                &NeedsApprovalReport {
                    schema: 1,
                    frame: state.binding.frame_id(),
                    command: prepared.command.id(),
                    path: &prepared.path,
                    diff,
                    reason: format!("{reason:?}"),
                },
            )?;
            let _writer = state
                .kernel
                .context()
                .lock_writer()
                .map_err(|_| FrameMutationError::Owner)?;
            state
                .kernel
                .context()
                .publish(
                    owner.session(),
                    Event::DiffNeedsApproval {
                        frame: state.binding.frame_id(),
                        diff,
                        report,
                    },
                )
                .map_err(|_| FrameMutationError::Owner)?;
            Err(FrameMutationError::NeedsApproval { report, reason })
        }
        VerificationDecision::Approved(approved) => {
            owner.ensure_open().map_err(|_| FrameMutationError::Owner)?;
            if state.cancellation.is_cancelled() {
                return Err(FrameMutationError::Binding);
            }
            let resource =
                ResourcePath::new(&prepared.path).map_err(|_| FrameMutationError::Path)?;
            let facts = Facts {
                resource: &resource,
                symbol: prepared.symbol.as_deref(),
                churn_lines: prepared.churn_lines,
                touches_unsafe: prepared.touches_unsafe,
                diff_blake3: Some(prepared.mutation_binding),
                right: Right::ProposeDiff,
            };
            if authorize(&token, &facts, &state.issuer, &SystemClock)
                .map_err(|_| FrameMutationError::Authorization)?
                != Authorization::Allowed
            {
                return Err(FrameMutationError::Authorization);
            }
            let actual = read_current(owner.workspace_root(), &prepared.path)?;
            if match actual.as_ref() {
                Some(bytes) => content_id(bytes),
                None => absent_base(),
            } != prepared.lease_base
            {
                return Err(FrameMutationError::StaleBase);
            }
            state
                .leases
                .validate_write_ticket(ticket, prepared.lease_base)
                .map_err(|_| FrameMutationError::Lease)?;
            let path =
                resolve_for_commit(owner.workspace_root(), &prepared.path, actual.is_some())?;
            atomic_commit(&path, &prepared.new, actual.is_some())?;
            let committed = read_source_bounded(&path).map_err(|_| {
                FrameMutationError::AppliedButUnrecorded {
                    path: prepared.path.clone(),
                    actual: content_id(&prepared.new),
                }
            })?;
            let actual_cid = content_id(&committed);
            if committed != prepared.new {
                return Err(FrameMutationError::AppliedButUnrecorded {
                    path: prepared.path.clone(),
                    actual: actual_cid,
                });
            }
            let diff = content_id(&prepared.diff_bytes);
            let receipt_record = StoredReceipt {
                schema: 2,
                match_level: prepared.match_level,
                command: approved.command_id(),
                frame: approved.agent(),
                path: prepared.path.clone(),
                base: prepared.base,
                new_cid: actual_cid,
                diff,
                approval_evidence: approved.evidence(),
            };
            if persist_bytes(owner.cas(), &prepared.diff_bytes).is_err() {
                return Err(FrameMutationError::AppliedButUnrecorded {
                    path: prepared.path,
                    actual: actual_cid,
                });
            }
            let receipt_cid = match persist_json(owner.cas(), &receipt_record) {
                Ok(cid) => cid,
                Err(_) => {
                    return Err(FrameMutationError::AppliedButUnrecorded {
                        path: prepared.path,
                        actual: actual_cid,
                    });
                }
            };
            let _writer = state.kernel.context().lock_writer().map_err(|_| {
                FrameMutationError::AppliedButUnrecorded {
                    path: prepared.path.clone(),
                    actual: actual_cid,
                }
            })?;
            if state
                .kernel
                .context()
                .publish(
                    owner.session(),
                    Event::DiffApproved {
                        frame: state.binding.frame_id(),
                        diff,
                    },
                )
                .is_err()
            {
                return Err(FrameMutationError::AppliedButUnrecorded {
                    path: prepared.path,
                    actual: actual_cid,
                });
            }
            Ok(FrameMutationReceipt {
                command: approved.command_id(),
                frame: approved.agent(),
                path: prepared.path,
                base: prepared.base,
                new_cid: actual_cid,
                diff,
                approval_evidence: approved.evidence(),
                receipt_cid,
            })
        }
    }
}

#[derive(Debug)]
struct ArcSafety(Arc<dyn SafetyBoundary>);
impl MutationSafety for ArcSafety {
    async fn assess(
        &self,
        request: &SafetyRequest<'_>,
    ) -> Result<
        polaris_system_one::verifier::SafetyAssessment,
        polaris_system_one::verifier::VerifierError,
    > {
        self.0.assess_owned(request).await
    }
}

#[derive(Serialize)]
struct NeedsApprovalReport<'a> {
    schema: u32,
    frame: FrameId,
    command: CommandId,
    path: &'a str,
    diff: Cid,
    reason: String,
}

#[derive(Serialize)]
struct StoredReceipt {
    schema: u32,
    command: CommandId,
    frame: FrameId,
    path: String,
    base: Option<Cid>,
    new_cid: Cid,
    diff: Cid,
    approval_evidence: Cid,
    /// How the edit target was located (schema 2); absent for writes and patches.
    #[serde(skip_serializing_if = "Option::is_none")]
    match_level: Option<MatchLevel>,
}

fn persist_json(
    cas: &crate::storage::cas::Cas,
    value: &impl Serialize,
) -> Result<Cid, FrameMutationError> {
    let bytes = serde_json::to_vec(value).map_err(|_| FrameMutationError::Persistence)?;
    let cid = cas
        .put(&bytes)
        .map_err(|_| FrameMutationError::Persistence)?;
    let stored = cas.get(&cid).map_err(|_| FrameMutationError::Persistence)?;
    if stored.as_ref() != bytes.as_slice() || content_id(&stored) != cid {
        return Err(FrameMutationError::Persistence);
    }
    Ok(cid)
}

fn persist_bytes(cas: &crate::storage::cas::Cas, bytes: &[u8]) -> Result<Cid, FrameMutationError> {
    let cid = cas
        .put(bytes)
        .map_err(|_| FrameMutationError::Persistence)?;
    let stored = cas.get(&cid).map_err(|_| FrameMutationError::Persistence)?;
    if stored.as_ref() != bytes || content_id(&stored) != cid {
        return Err(FrameMutationError::Persistence);
    }
    Ok(cid)
}

fn read_current(root: &Path, target: &str) -> Result<Option<Vec<u8>>, FrameMutationError> {
    match resolve_in_workspace(root, target) {
        Ok(path) => Ok(Some(read_source_bounded(&path)?)),
        Err(_) => Ok(None),
    }
}

fn resolve_for_commit(
    root: &Path,
    target: &str,
    existing: bool,
) -> Result<PathBuf, FrameMutationError> {
    if existing {
        return resolve_in_workspace(root, target).map_err(|_| FrameMutationError::Path);
    }
    let relative = Path::new(target);
    let parent = relative.parent().ok_or(FrameMutationError::Path)?;
    let parent = if parent.as_os_str().is_empty() {
        "."
    } else {
        parent.to_str().ok_or(FrameMutationError::Path)?
    };
    let parent_path = resolve_parent(root, parent)?;
    Ok(parent_path.join(relative.file_name().ok_or(FrameMutationError::Path)?))
}

fn resolve_parent(root: &Path, parent: &str) -> Result<PathBuf, FrameMutationError> {
    if parent == "." {
        return fs::canonicalize(root).map_err(|_| FrameMutationError::Path);
    }
    let resolved = resolve_in_workspace(root, parent).map_err(|_| FrameMutationError::Path)?;
    if !resolved.is_dir() {
        return Err(FrameMutationError::Path);
    }
    Ok(resolved)
}

fn atomic_commit(path: &Path, bytes: &[u8], replacing: bool) -> Result<(), FrameMutationError> {
    let parent = path.parent().ok_or(FrameMutationError::Path)?;
    if path.file_name().is_none() {
        return Err(FrameMutationError::Path);
    }
    let temp = parent.join(format!(
        ".polaris-mutation-{}.tmp",
        CommandId::new_v7().as_uuid().simple()
    ));
    let permissions = if replacing {
        Some(
            fs::metadata(path)
                .map_err(|_| FrameMutationError::Workspace)?
                .permissions(),
        )
    } else {
        None
    };
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| FrameMutationError::Workspace)?;
    let staging = file.write_all(bytes).and_then(|()| file.sync_all());
    if staging.is_err() {
        drop(file);
        let _removed = fs::remove_file(&temp);
        return Err(FrameMutationError::Workspace);
    }
    if let Some(mode) = permissions
        && (file.set_permissions(mode).is_err() || file.sync_all().is_err())
    {
        drop(file);
        let _removed = fs::remove_file(&temp);
        return Err(FrameMutationError::Workspace);
    }
    drop(file);
    let commit = if replacing {
        fs::rename(&temp, path)
    } else {
        fs::hard_link(&temp, path)
    };
    if commit.is_err() {
        let _removed = fs::remove_file(&temp);
        return Err(FrameMutationError::Workspace);
    }
    if !replacing {
        let _removed = fs::remove_file(&temp);
    }
    Ok(())
}

/// Sanitized mutation failures; no raw token, source text or secret path is included.
#[derive(Debug, thiserror::Error)]
pub enum FrameMutationError {
    /// Sealed frame, token, cancellation or service identity mismatch.
    #[error("frame mutation binding rejected")]
    Binding,
    /// Capability lacked the actual ProposeDiff right for observed facts.
    #[error("frame mutation capability denied")]
    Authorization,
    /// Workspace target violates path policy or containment.
    #[error("frame mutation target rejected")]
    Path,
    /// Request exceeded a hard byte limit.
    #[error("frame mutation exceeds its limit")]
    Limit,
    /// Candidate could not be produced from the exact base.
    #[error("frame mutation proposal is invalid")]
    InvalidProposal,
    /// `edit_file` could not locate `old_text`; the text explains how to fix the call.
    #[error("{0}")]
    EditTarget(String),
    /// Expected base did not match the actual workspace state.
    #[error("frame mutation base is stale")]
    StaleBase,
    /// Workspace operation failed.
    #[error("frame mutation workspace operation failed")]
    Workspace,
    /// Kernel inspection or event owner failed.
    #[error("frame mutation owner unavailable")]
    Owner,
    /// Another writer holds the lease or the lease ticket expired/revoked.
    #[error("frame mutation lease rejected")]
    Lease,
    /// Real parser, metrics or safety analysis failed closed.
    #[error("frame mutation analysis failed")]
    Analysis,
    /// Hybrid verifier infrastructure failed closed.
    #[error("hybrid mutation verifier failed")]
    Verifier,
    /// Actual verifier denied the proposal.
    #[error("hybrid mutation rejected: {0:?}")]
    Rejected(Rejection),
    /// Human review is required; no file mutation occurred.
    #[error("mutation requires explicit human review (report {report})")]
    NeedsApproval {
        /// Durable CAS report; this is not an execution approval.
        report: Cid,
        /// Actual hybrid-verifier reason for requesting review.
        reason: ApprovalReason,
    },
    /// File bytes changed but durable receipt/event completion failed; digest identifies actual bytes.
    #[error("mutation applied at {path}; durable completion is unrecorded ({actual})")]
    AppliedButUnrecorded {
        /// Normalized target whose write completed.
        path: String,
        /// Digest read back from the committed file, or written bytes if read-back failed.
        actual: Cid,
    },
    /// CAS report/receipt could not be durably read back.
    #[error("mutation evidence persistence failed")]
    Persistence,
}

impl From<KernelError> for FrameMutationError {
    fn from(_: KernelError) -> Self {
        Self::Owner
    }
}
