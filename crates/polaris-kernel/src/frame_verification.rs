//! Frame step and completion verification against the active workspace, CAS and event log.
//!
//! The cheap frame fingerprint is derived from a paranoid manifest scan. Full CIDs are retained
//! in the returned opaque evidence; the fingerprint is only the runtime's compact hint.

use crate::{KernelError, KernelHandle, runtime::InspectionOwner};
use polaris_runtime::frames::{FrameToolBinding, FrameView, StepTool, StepVerifier, Verification};
use polaris_sdk::{
    contracts::{Event, EventRecord},
    error::{ErrorKind, HasKind},
    ids::{Cid, FastFingerprint, FrameId, SessionId},
    registry::ToolError,
};
use polaris_storage::sync::{self, EntryKind, HashMode, IgnoreRules, Manifest};
use serde::Deserialize;
use serde_json::Value;
use std::{fmt, path::Path};

const MAX_RESULT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_PROOF_BYTES: u64 = 1024 * 1024;
const MAX_CALL_BYTES: u64 = 4 * 1024 * 1024 + 4096;
const MAX_ENTRIES: usize = 100_000;

/// Actual workspace/CAS evidence for one accepted frame step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedFrameStep {
    frame: FrameId,
    session: SessionId,
    source: Cid,
    fingerprint: FastFingerprint,
    verified_diffs: Vec<Cid>,
}

impl VerifiedFrameStep {
    /// Bound frame id.
    #[must_use]
    pub const fn frame(&self) -> FrameId {
        self.frame
    }
    /// Bound session id.
    #[must_use]
    pub const fn session(&self) -> SessionId {
        self.session
    }
    /// Full paranoid manifest CID.
    #[must_use]
    pub const fn source_cid(&self) -> Cid {
        self.source
    }
    /// Compact runtime fingerprint derived from the full source CID.
    #[must_use]
    pub const fn fingerprint(&self) -> FastFingerprint {
        self.fingerprint
    }
    /// Independently accepted mutation diff CIDs.
    #[must_use]
    pub fn verified_diffs(&self) -> &[Cid] {
        &self.verified_diffs
    }
}

/// Opaque proof gate result for the exact frame, session, and current workspace tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedFrameCompletion {
    frame: FrameId,
    session: SessionId,
    source: Cid,
    report: Cid,
}

impl VerifiedFrameCompletion {
    /// Bound frame id.
    #[must_use]
    pub const fn frame(&self) -> FrameId {
        self.frame
    }
    /// Bound session id.
    #[must_use]
    pub const fn session(&self) -> SessionId {
        self.session
    }
    /// Full current paranoid manifest CID.
    #[must_use]
    pub const fn source_cid(&self) -> Cid {
        self.source
    }
    /// Durable proof report CID from the actual passed event.
    #[must_use]
    pub const fn report_cid(&self) -> Cid {
        self.report
    }
}

/// Production verifier bound to the actual kernel session and sealed frame.
pub struct KernelFrameVerifier {
    kernel: KernelHandle,
    binding: FrameToolBinding,
    cancellation: tokio_util::sync::CancellationToken,
}

impl fmt::Debug for KernelFrameVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KernelFrameVerifier(<redacted>)")
    }
}

impl KernelFrameVerifier {
    /// Bind the verifier to a frame from this kernel's actual root.
    /// # Errors
    /// Rejects a frame binding rooted in another kernel.
    pub fn new(
        kernel: KernelHandle,
        binding: FrameToolBinding,
    ) -> Result<Self, FrameVerificationError> {
        if binding.root_frame_id() != kernel.frame() {
            return Err(FrameVerificationError::Binding);
        }
        let cancellation = kernel
            .frame_cancellations()
            .bind_blocking(binding.clone())
            .map_err(|_| FrameVerificationError::Binding)?
            .cancellation_token();
        Ok(Self {
            kernel,
            binding,
            cancellation,
        })
    }

    /// Independently check a model step against actual result CAS and same-frame journal events.
    /// # Errors
    /// Returns an error on foreign identity, unavailable stores, malformed evidence, or mismatch.
    pub async fn verify_step(
        &self,
        frame: &FrameView,
        output: &StepTool,
    ) -> Result<VerifiedFrameStep, FrameVerificationError> {
        self.check_frame(frame)?;
        let size = output.tool_results.iter().try_fold(
            output
                .text
                .len()
                .checked_add(output.reasoning.len())
                .ok_or(FrameVerificationError::Evidence)?,
            |total, result| {
                total
                    .checked_add(result.result.content.len())
                    .and_then(|n| n.checked_add(result.tool_call_id.len()))
                    .ok_or(FrameVerificationError::Evidence)
            },
        )?;
        if size > 8 * 1024 * 1024 || output.tool_results.len() > 64 {
            return Err(FrameVerificationError::Evidence);
        }
        let owner = self
            .kernel
            .inspection_owner()
            .await
            .map_err(FrameVerificationError::Admission)?;
        let expected = self.binding.frame_id();
        let output = output.clone();
        let task = tokio::task::spawn_blocking(move || verify_step_owned(owner, expected, &output));
        let result = task
            .await
            .map_err(|_| FrameVerificationError::OwnerFailed)?;
        self.check_frame(frame)?;
        result
    }

    /// Validate an actual passed proof event against the current exact workspace tree.
    /// This method returns evidence only; it never changes the frame's terminal state.
    /// # Errors
    /// Rejects missing, stale, malformed, foreign-frame, or unlinked proof records.
    pub async fn validate_completion(
        &self,
        frame: &FrameView,
    ) -> Result<VerifiedFrameCompletion, FrameVerificationError> {
        self.check_frame(frame)?;
        let owner = self
            .kernel
            .inspection_owner()
            .await
            .map_err(FrameVerificationError::Admission)?;
        let expected = self.binding.frame_id();
        let task = tokio::task::spawn_blocking(move || validate_completion_owned(owner, expected));
        let result = task
            .await
            .map_err(|_| FrameVerificationError::OwnerFailed)?;
        self.check_frame(frame)?;
        result
    }

    fn check_frame(&self, frame: &FrameView) -> Result<(), FrameVerificationError> {
        if self.cancellation.is_cancelled()
            || frame.id != self.binding.frame_id()
            || frame.parent != self.binding.parent_frame_id()
            || frame.role != self.binding.role()
        {
            return Err(FrameVerificationError::Binding);
        }
        Ok(())
    }
}

impl StepVerifier for KernelFrameVerifier {
    type Error = FrameVerificationError;
    async fn verify(
        &self,
        frame: &FrameView,
        output: &StepTool,
    ) -> Result<Verification, Self::Error> {
        if output.finish_reason == polaris_sdk::registry::FinishReason::ToolCalls
            && output.tool_results.is_empty()
        {
            return Err(FrameVerificationError::Evidence);
        }
        let evidence = self.verify_step(frame, output).await?;
        Ok(Verification::Accepted {
            fingerprint: evidence.fingerprint,
            verified_diffs: evidence.verified_diffs,
        })
    }
}

fn verify_step_owned(
    owner: InspectionOwner,
    frame: FrameId,
    output: &StepTool,
) -> Result<VerifiedFrameStep, FrameVerificationError> {
    owner
        .ensure_open()
        .map_err(FrameVerificationError::Admission)?;
    let manifest = snapshot(owner.workspace_root())?;
    let source = manifest.content_id();
    let events = session_events(&owner)?;
    let mut diffs = Vec::new();
    let mut call_ids = std::collections::BTreeSet::new();
    if output.tool_results.len() > 64 {
        return Err(FrameVerificationError::Evidence);
    }
    for executed in &output.tool_results {
        if executed.tool_call_id.is_empty()
            || executed.tool_call_id.len() > 256
            || !call_ids.insert(&executed.tool_call_id)
            || u64::try_from(executed.result.content.len())
                .map_err(|_| FrameVerificationError::Evidence)?
                > MAX_RESULT_BYTES
        {
            return Err(FrameVerificationError::Evidence);
        }
        let content = read_cas(owner.cas(), executed.result.content_cid, MAX_RESULT_BYTES)?;
        if content.as_slice() != executed.result.content.as_bytes() {
            return Err(FrameVerificationError::Evidence);
        }
        if executed.result.cid == executed.result.content_cid {
            // Legacy read-only runner outputs cannot contribute approval or completion evidence.
            let value: Value =
                serde_json::from_slice(&content).map_err(|_| FrameVerificationError::Evidence)?;
            if value.get("status").is_some() {
                return Err(FrameVerificationError::Evidence);
            }
            continue;
        }
        let envelope: ResultEnvelope = decode_cas(owner.cas(), executed.result.cid, 4096)?;
        if envelope.version != 1
            || envelope.session != owner.session()
            || envelope.frame != frame
            || envelope.content != executed.result.content_cid
        {
            return Err(FrameVerificationError::Evidence);
        }
        let call: StoredCall = decode_cas(owner.cas(), envelope.call, MAX_CALL_BYTES)?;
        if call.version != 1
            || call.frame != frame
            || call.id != executed.tool_call_id
            || call.step == Some(0)
        {
            return Err(FrameVerificationError::Evidence);
        }
        let started=events.iter().find(|r|matches!(r.event(),Event::ToolExecutionStarted {frame:f,call:c} if *f==frame && *c==envelope.call)).ok_or(FrameVerificationError::Evidence)?;
        let recorded=events.iter().find(|r|matches!(r.event(),Event::FrameToolResultRecorded {frame:f,call:c,result} if *f==frame && *c==envelope.call && *result==executed.result.cid)).ok_or(FrameVerificationError::Evidence)?;
        let completed=events.iter().find(|r|matches!(r.event(),Event::ToolExecutionCompleted {frame:f,call:c,..} if *f==frame && *c==envelope.call)).ok_or(FrameVerificationError::Evidence)?;
        if !(started.seq() < recorded.seq() && recorded.seq() < completed.seq()) {
            return Err(FrameVerificationError::Evidence);
        }
        let value: Value =
            serde_json::from_slice(&content).map_err(|_| FrameVerificationError::Evidence)?;
        match call.name.as_str() {
            "write_file" | "edit_file" | "apply_patch" => {
                match value.get("status").and_then(Value::as_str) {
                    Some("applied") => {
                        if !matches!(
                            completed.event(),
                            Event::ToolExecutionCompleted { ok: true, .. }
                        ) {
                            return Err(FrameVerificationError::Evidence);
                        }
                        diffs.push(verify_mutation_result(
                            &owner,
                            &events,
                            frame,
                            &call,
                            &content,
                            &manifest,
                            (started.seq(), recorded.seq()),
                        )?);
                    }
                    Some("needs_approval")
                        if matches!(
                            completed.event(),
                            Event::ToolExecutionCompleted { ok: false, .. }
                        ) =>
                    {
                        let report: Cid = serde_json::from_value(
                            value
                                .get("report")
                                .cloned()
                                .ok_or(FrameVerificationError::Evidence)?,
                        )
                        .map_err(|_| FrameVerificationError::Evidence)?;
                        let _report = read_cas(owner.cas(), report, MAX_RESULT_BYTES)?;
                    }
                    _ => return Err(FrameVerificationError::Evidence),
                }
            }
            "math" => {
                let input: polaris_tools::math::MathInput =
                    serde_json::from_value(call.arguments.clone())
                        .map_err(|_| FrameVerificationError::Evidence)?;
                let expected = polaris_tools::math::MathTool
                    .evaluate(input)
                    .map_err(|_| FrameVerificationError::Evidence)?;
                let expected =
                    serde_json::to_value(expected).map_err(|_| FrameVerificationError::Evidence)?;
                if value != expected
                    || !matches!(
                        completed.event(),
                        Event::ToolExecutionCompleted { ok: true, .. }
                    )
                {
                    return Err(FrameVerificationError::Evidence);
                }
            }
            "ask_user" | "file_revision" | "run_tests" => {}
            _ => return Err(FrameVerificationError::Evidence),
        }
    }
    if snapshot(owner.workspace_root())?.content_id() != source {
        return Err(FrameVerificationError::Workspace);
    }
    diffs.sort();
    diffs.dedup();
    let prefix: [u8; 8] = source
        .as_bytes()
        .get(..8)
        .ok_or(FrameVerificationError::Evidence)?
        .try_into()
        .map_err(|_| FrameVerificationError::Evidence)?;
    Ok(VerifiedFrameStep {
        frame,
        session: owner.session(),
        source,
        fingerprint: FastFingerprint::new(u64::from_le_bytes(prefix)),
        verified_diffs: diffs,
    })
}
fn verify_mutation_result(
    owner: &InspectionOwner,
    events: &[EventRecord],
    frame: FrameId,
    call: &StoredCall,
    bytes: &[u8],
    manifest: &Manifest,
    range: (polaris_sdk::ids::Seq, polaris_sdk::ids::Seq),
) -> Result<Cid, FrameVerificationError> {
    let (started, recorded) = range;
    let result: MutationResult =
        serde_json::from_slice(bytes).map_err(|_| FrameVerificationError::Evidence)?;
    let receipt: StoredReceipt = decode_cas(owner.cas(), result.receipt, 16 * 1024)?;
    if result.status != "applied"
        || receipt.schema != 1
        || receipt.frame != frame
        || receipt.path != result.path
        || receipt.base != result.base
        || receipt.new_cid != result.revision
        || receipt.diff != result.diff
        || receipt.approval_evidence != result.approval
    {
        return Err(FrameVerificationError::Evidence);
    }
    if call.arguments.get("path").and_then(Value::as_str) != Some(receipt.path.as_str()) {
        return Err(FrameVerificationError::Evidence);
    }
    let base: Option<Cid> = serde_json::from_value(
        call.arguments
            .get("expected_base")
            .cloned()
            .ok_or(FrameVerificationError::Evidence)?,
    )
    .map_err(|_| FrameVerificationError::Evidence)?;
    if base != receipt.base {
        return Err(FrameVerificationError::Evidence);
    }
    if let Some(id) = call.arguments.get("command_id") {
        let id: polaris_sdk::ids::CommandId =
            serde_json::from_value(id.clone()).map_err(|_| FrameVerificationError::Evidence)?;
        if id != receipt.command {
            return Err(FrameVerificationError::Evidence);
        }
    }
    let path = sync::RelPath::new(&receipt.path).map_err(|_| FrameVerificationError::Evidence)?;
    if !manifest
        .entries
        .get(&path)
        .is_some_and(|e| e.kind == EntryKind::File && e.hash == receipt.new_cid)
    {
        return Err(FrameVerificationError::Evidence);
    }
    let approval: ApprovalEvidence =
        decode_cas(owner.cas(), receipt.approval_evidence, 128 * 1024)?;
    let old = match receipt.base {
        Some(cid) => cid,
        None => crate::storage::cas::content_id(b""),
    };
    let digest = crate::verifier::mutation_digest(
        receipt.command,
        frame,
        &receipt.path,
        receipt.diff,
        old,
        receipt.new_cid,
    )
    .map_err(|_| FrameVerificationError::Evidence)?;
    if approval.schema != 1
        || approval.command_id != receipt.command
        || approval.agent != frame
        || approval.target != receipt.path
        || approval.diff != receipt.diff
        || approval.old_source != old
        || approval.new_source != receipt.new_cid
        || approval.mutation_binding != digest
        || approval.safety_result != "preflight_and_congruence_allowed"
        || approval.churn_lines > approval.max_churn_lines
        || i64::from(approval.cyclomatic_delta) > i64::from(approval.max_cyclomatic_increase)
        || !approval.tree_edit_distance_exact
        || approval.tree_edit_distance > approval.max_tree_edit_distance
    {
        return Err(FrameVerificationError::Evidence);
    }
    let _diff = read_cas(owner.cas(), receipt.diff, MAX_RESULT_BYTES)?;
    if !events.iter().any(|r|r.seq()>started && r.seq()<recorded && matches!(r.event(),Event::DiffApproved {frame:f,diff} if *f==frame && *diff==receipt.diff)) { return Err(FrameVerificationError::Evidence); }
    Ok(receipt.diff)
}
fn validate_completion_owned(
    owner: InspectionOwner,
    frame: FrameId,
) -> Result<VerifiedFrameCompletion, FrameVerificationError> {
    owner
        .ensure_open()
        .map_err(FrameVerificationError::Admission)?;
    let tree = snapshot(owner.workspace_root())?.content_id();
    let events = session_events(&owner)?;
    // The latest terminal proof for this frame controls acceptance; do not revive an older pass.
    let terminal=events.iter().rev().find(|r|matches!(r.event(),Event::ProofOfDonePassed {frame:f,..}|Event::ProofOfDoneBlocked {frame:f,..} if *f==frame)).ok_or(FrameVerificationError::CompletionUnavailable)?;
    let Event::ProofOfDonePassed { report, .. } = *terminal.event() else {
        return Err(FrameVerificationError::CompletionUnavailable);
    };
    let proof: ProofReport = decode_cas(owner.cas(), report, MAX_PROOF_BYTES)?;
    if proof.schema != 2
        || proof.session != owner.session()
        || proof.frame != frame
        || !proof.passed
        || proof.reason.is_some()
        || proof.tree != tree
    {
        return Err(FrameVerificationError::CompletionUnavailable);
    }
    let independent: IndependentReport =
        decode_cas(owner.cas(), proof.independent_report, MAX_PROOF_BYTES)?;
    let ordinary: OrdinaryExecution =
        decode_cas(owner.cas(), independent.ordinary_execution, MAX_PROOF_BYTES)?;
    if independent.kind != "independent_falsifier_run"
        || independent.exit_code != 0
        || ordinary.exit_code != 0
        || ordinary.command != proof.command
        || ordinary.stdout != proof.stdout
        || ordinary.stderr != proof.stderr
    {
        return Err(FrameVerificationError::Evidence);
    }
    for cid in [
        proof.stdout,
        proof.stderr,
        independent.stdout,
        independent.stderr,
    ] {
        let _bytes = read_cas(owner.cas(), cid, 16 * 1024 * 1024)?;
    }
    let executed=events.iter().find(|r|r.seq()<terminal.seq() && matches!(r.event(),Event::SandboxExecutionCompleted {frame:f,exec,exit_code:0,..} if *f==frame && *exec==independent.ordinary_execution)).ok_or(FrameVerificationError::Evidence)?;
    let start = events
        .iter()
        .rev()
        .find(|r| {
            r.seq() < executed.seq()
                && matches!(r.event(),Event::ToolExecutionStarted {frame:f,call} if *f==frame && *call==proof.review)
        })
        .ok_or(FrameVerificationError::Evidence)?;
    let Event::ToolExecutionStarted { call, .. } = *start.event() else {
        return Err(FrameVerificationError::Evidence);
    };
    let review: Value = decode_cas(owner.cas(), call, MAX_PROOF_BYTES)?;
    if review.get("tree") != Some(&serde_json::json!(tree))
        || review.get("session") != Some(&serde_json::json!(owner.session()))
        || review.get("frame") != Some(&serde_json::json!(frame))
    {
        return Err(FrameVerificationError::Evidence);
    }
    if !events.iter().any(|r|r.seq()>terminal.seq() && matches!(r.event(),Event::ToolExecutionCompleted {frame:f,call:c,ok:true,..} if *f==frame && *c==call)) {return Err(FrameVerificationError::Evidence);}
    if snapshot(owner.workspace_root())?.content_id() != tree {
        return Err(FrameVerificationError::Workspace);
    }
    Ok(VerifiedFrameCompletion {
        frame,
        session: owner.session(),
        source: tree,
        report,
    })
}
fn read_cas(
    cas: &crate::storage::cas::Cas,
    cid: Cid,
    max: u64,
) -> Result<Vec<u8>, FrameVerificationError> {
    let bytes = cas
        .get_bounded(&cid, max)
        .map_err(|_| FrameVerificationError::Evidence)?;
    if crate::storage::cas::content_id(&bytes) != cid {
        return Err(FrameVerificationError::Evidence);
    }
    Ok(bytes.to_vec())
}
fn decode_cas<T: serde::de::DeserializeOwned>(
    cas: &crate::storage::cas::Cas,
    cid: Cid,
    max: u64,
) -> Result<T, FrameVerificationError> {
    serde_json::from_slice(&read_cas(cas, cid, max)?).map_err(|_| FrameVerificationError::Evidence)
}
fn session_events(owner: &InspectionOwner) -> Result<Vec<EventRecord>, FrameVerificationError> {
    let mut events = Vec::new();
    for record in owner
        .context()
        .bus
        .store()
        .replay_iter(polaris_sdk::ids::Seq::new(u64::MAX))
        .map_err(KernelError::from)?
    {
        let record = record.map_err(KernelError::from)?;
        if record.session() == owner.session() {
            if events.len() >= 1_000_000 {
                return Err(FrameVerificationError::Evidence);
            }
            events.push(record);
        }
    }
    Ok(events)
}
fn snapshot(root: &Path) -> Result<Manifest, FrameVerificationError> {
    let rules = IgnoreRules::from_workspace(root).map_err(|_| FrameVerificationError::Workspace)?;
    let scan = sync::scan(root, &rules, None, HashMode::Paranoid)
        .map_err(|_| FrameVerificationError::Workspace)?;
    if !scan.escaped.is_empty()
        || !scan.unrepresentable.is_empty()
        || scan.manifest.entries.len() > MAX_ENTRIES
        || !sync::detect_conflicts(&scan.manifest).is_empty()
    {
        return Err(FrameVerificationError::Workspace);
    }
    Ok(scan.manifest)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultEnvelope {
    version: u8,
    session: SessionId,
    frame: FrameId,
    call: Cid,
    content: Cid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredCall {
    version: u16,
    frame: FrameId,
    step: Option<u32>,
    id: String,
    name: String,
    arguments: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MutationResult {
    status: String,
    path: String,
    base: Option<Cid>,
    revision: Cid,
    diff: Cid,
    approval: Cid,
    receipt: Cid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReceipt {
    schema: u32,
    command: polaris_sdk::ids::CommandId,
    frame: FrameId,
    path: String,
    base: Option<Cid>,
    new_cid: Cid,
    diff: Cid,
    approval_evidence: Cid,
}
// These mandatory identity and threshold fields are validated; additional audit fields stay in CAS.
#[derive(Deserialize)]
struct ApprovalEvidence {
    schema: u16,
    command_id: polaris_sdk::ids::CommandId,
    agent: FrameId,
    target: String,
    diff: Cid,
    old_source: Cid,
    new_source: Cid,
    mutation_binding: Cid,
    churn_lines: u32,
    max_churn_lines: u32,
    cyclomatic_delta: i32,
    max_cyclomatic_increase: u32,
    tree_edit_distance: u32,
    tree_edit_distance_exact: bool,
    max_tree_edit_distance: u32,
    safety_result: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProofReport {
    schema: u16,
    session: SessionId,
    frame: FrameId,
    review: Cid,
    command: polaris_sdk::ids::CommandId,
    tree: Cid,
    stdout: Cid,
    stderr: Cid,
    independent_report: Cid,
    passed: bool,
    reason: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IndependentReport {
    kind: String,
    stdout: Cid,
    stderr: Cid,
    exit_code: i32,
    ordinary_execution: Cid,
}
#[derive(Deserialize)]
struct OrdinaryExecution {
    command: polaris_sdk::ids::CommandId,
    stdout: Cid,
    stderr: Cid,
    exit_code: i32,
}

/// Sanitized fail-closed errors from workspace and durable proof verification.
#[derive(Debug, thiserror::Error)]
pub enum FrameVerificationError {
    /// Binding does not name the kernel's sealed frame.
    #[error("frame verification binding mismatch")]
    Binding,
    /// CAS, receipt, event, call or approval evidence is missing or inconsistent.
    #[error("frame verification evidence is invalid")]
    Evidence,
    /// No valid same-frame proof exists for the current workspace tree.
    #[error("proof of done is unavailable for the current frame and source")]
    CompletionUnavailable,
    /// Workspace scan failed or encountered an unsafe tree.
    #[error("workspace source could not be verified")]
    Workspace,
    /// Kernel event storage failed.
    #[error(transparent)]
    Kernel(#[from] KernelError),
    /// Inspection admission failed.
    #[error(transparent)]
    Admission(#[from] ToolError),
    /// Bounded blocking verifier did not return.
    #[error("frame verification owner failed")]
    OwnerFailed,
}

impl HasKind for FrameVerificationError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::Binding | Self::Evidence | Self::CompletionUnavailable | Self::Workspace => {
                ErrorKind::Policy
            }
            Self::Kernel(error) => error.kind(),
            Self::Admission(error) => error.kind(),
            Self::OwnerFailed => ErrorKind::Fatal,
        }
    }
}
