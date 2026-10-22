//! Typestate transitions retain exact proof bindings and remove raw artifact references.
use crate::{DataLabError, OptIn};
use core::{fmt, marker::PhantomData};
use polaris_sdk::{
    contracts::Timestamp,
    ids::{Cid, FrameId, SessionId},
    registry::Redactor,
};
use polaris_security_automata::{Category, DfaScanner, EntropyCfg, ScanBuf, high_entropy_spans};
use polaris_storage_cas::{Cas, content_id};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Collected data which has not yet passed a proof gate.
#[derive(Debug)]
pub struct Draft;
/// Exact source and runner artifacts have matching proof-of-done evidence.
#[derive(Debug)]
pub struct Verified;
/// All text and training artifacts have been masked and independently rescanned.
#[derive(Debug)]
pub struct Masked;

/// Exported message role. Tool call/result identities remain explicit in both formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// Stable task/system instructions.
    System,
    /// User request.
    User,
    /// Model output.
    Assistant,
    /// Tool result.
    Tool,
}

/// Input text or an actual CAS reference. Debug never prints input bytes.
pub enum RawContent {
    /// Inline sensitive text, zeroized when dropped.
    Inline(Zeroizing<String>),
    /// Existing content in the caller's CAS; never a fabricated placeholder reference.
    Blob(Cid),
}

impl RawContent {
    /// Wrap text in zeroizing storage.
    #[must_use]
    pub fn text(value: String) -> Self {
        Self::Inline(Zeroizing::new(value))
    }

    fn resolve(&self, cas: &Cas, limit: usize) -> Result<Zeroizing<Vec<u8>>, DataLabError> {
        let bytes = match self {
            Self::Inline(text) => {
                if text.len() > limit {
                    return Err(DataLabError::Limit);
                }
                text.as_bytes().to_vec()
            }
            Self::Blob(cid) => {
                let bytes = cas.get(cid)?;
                if content_id(&bytes) != *cid {
                    return Err(DataLabError::DigestMismatch);
                }
                bytes.to_vec()
            }
        };
        let bytes = Zeroizing::new(bytes);
        if bytes.len() > limit {
            return Err(DataLabError::Limit);
        }
        Ok(bytes)
    }

    pub(crate) fn masked_text(&self) -> Result<&str, DataLabError> {
        match self {
            Self::Inline(text) => Ok(text.as_str()),
            Self::Blob(_) => Err(DataLabError::Schema("unresolved masked message")),
        }
    }
}

impl fmt::Debug for RawContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inline(text) => f
                .debug_struct("Inline")
                .field("bytes", &text.len())
                .finish(),
            Self::Blob(cid) => f.debug_tuple("Blob").field(cid).finish(),
        }
    }
}

/// Model-issued tool call, preserved as structured data after masking.
#[derive(Debug)]
pub struct ToolCallInput {
    /// Call identity used by a matching tool message.
    pub id: String,
    /// Actual tool name, not a simulated invocation.
    pub name: String,
    /// JSON arguments; valid JSON is required before and after masking.
    pub arguments: RawContent,
}

/// One collected message.
#[derive(Debug)]
pub struct MessageInput {
    /// Message role.
    pub role: MessageRole,
    /// Message body or content reference.
    pub content: RawContent,
    /// Tool calls emitted by an assistant message.
    pub tool_calls: Vec<ToolCallInput>,
    /// Call id for a tool-result message.
    pub tool_call_id: Option<String>,
}

/// Source and licensing metadata declared by the session owner.
#[derive(Debug)]
pub struct Provenance {
    /// Repository identity; may be omitted for a standalone session.
    pub repository: Option<String>,
    /// Recorded source license, when known.
    pub license: Option<String>,
    /// Whether the owner explicitly excluded this source from training material.
    pub excluded: bool,
    /// Actual model id used by the execution owner.
    pub model: String,
    /// Relevant producer/tool version descriptions.
    pub tool_versions: Vec<String>,
}

/// Raw inputs bound to one immutable workspace and execution artifacts.
#[derive(Debug)]
pub struct TrajectoryInput {
    /// Consent and event session.
    pub session: SessionId,
    /// Frame that produced the solution.
    pub frame: FrameId,
    /// Immutable workspace snapshot evaluated by the runner.
    pub workspace_snapshot: Cid,
    /// Exact proposed diff stored in CAS.
    pub diff: Cid,
    /// Real test execution record in CAS.
    pub test_run: Cid,
    /// Time of collection, supplied by the execution clock.
    pub collected_at: Timestamp,
    /// Actual conversation to export after masking.
    pub messages: Vec<MessageInput>,
    /// Source provenance and declared use restrictions.
    pub provenance: Provenance,
}

/// Processing limits, not measured performance or model-token claims.
#[derive(Debug, Clone, Copy)]
pub struct TrajectoryLimits {
    /// Maximum messages per trajectory.
    pub max_messages: usize,
    /// Maximum decoded bytes in one message or training artifact.
    pub max_content_bytes: usize,
    /// Maximum total decoded text and artifact bytes.
    pub max_total_bytes: usize,
    /// Maximum tool calls across the trajectory.
    pub max_tool_calls: usize,
    /// Require an explicitly recorded source license before export.
    pub require_license: bool,
}

impl Default for TrajectoryLimits {
    fn default() -> Self {
        Self {
            max_messages: 4096,
            max_content_bytes: 4 * 1024 * 1024,
            max_total_bytes: 32 * 1024 * 1024,
            max_tool_calls: 8192,
            require_license: true,
        }
    }
}

/// Proof outcome from the trusted kernel proof-record adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofStatus {
    /// Independent scoped checks passed.
    Passed,
    /// At least one required check failed.
    Failed,
    /// A required check or artifact was absent/inconclusive.
    Unverified,
}

/// Identities resolved from an actual stored proof record by the composition-root adapter.
#[derive(Debug, Clone, Copy)]
pub struct ProofFacts {
    /// Session bound to the proof event.
    pub session: SessionId,
    /// Producing frame.
    pub frame: FrameId,
    /// Evaluated workspace snapshot.
    pub workspace_snapshot: Cid,
    /// Diff approved and executed by that proof.
    pub diff: Cid,
    /// Execution record whose results the proof evaluated.
    pub test_run: Cid,
    /// Scoped proof status.
    pub status: ProofStatus,
}

/// Trusted adapter for the current core proof schema. It must validate the full proof record,
/// including independent falsifier/mutation evidence, and bind its event identities.
pub trait ProofResolver {
    /// Resolve already digest-checked proof bytes. A model assertion is never a proof record.
    fn resolve(&self, proof: &[u8]) -> Result<ProofFacts, DataLabError>;
}

/// Private payload prevents bypassing the proof/masking transitions through public struct fields.
pub struct Trajectory<S> {
    pub(crate) input: TrajectoryInput,
    pub(crate) proof: Option<Cid>,
    pub(crate) limits: TrajectoryLimits,
    pub(crate) consent: Cid,
    state: PhantomData<S>,
}

impl<S> fmt::Debug for Trajectory<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Trajectory")
            .field("session", &self.input.session)
            .field("frame", &self.input.frame)
            .field("messages", &self.input.messages.len())
            .field("proof", &self.proof)
            .finish_non_exhaustive()
    }
}

impl Trajectory<Draft> {
    /// Collect only with currently enabled, persisted session consent.
    ///
    /// # Errors
    /// Invalid bounds, restricted provenance, malformed tool sequence or absent consent.
    pub fn collect(
        input: TrajectoryInput,
        optin: &OptIn,
        limits: TrajectoryLimits,
    ) -> Result<Self, DataLabError> {
        let permit = optin.permit(input.session)?;
        if limits.max_messages == 0
            || limits.max_content_bytes == 0
            || limits.max_total_bytes == 0
            || input.messages.is_empty()
            || input.messages.len() > limits.max_messages
            || input.provenance.tool_versions.len() > 128
            || input.provenance.model.len() > 1024
            || input
                .provenance
                .repository
                .as_ref()
                .is_some_and(|value| value.len() > 4096)
            || input
                .provenance
                .license
                .as_ref()
                .is_some_and(|value| value.len() > 4096)
            || input
                .provenance
                .tool_versions
                .iter()
                .any(|value| value.len() > 4096)
        {
            return Err(DataLabError::Limit);
        }
        if input.provenance.excluded
            || (limits.require_license
                && input
                    .provenance
                    .license
                    .as_deref()
                    .is_none_or(|license| license.trim().is_empty()))
        {
            return Err(DataLabError::ProvenanceExcluded);
        }
        validate_messages(&input.messages, limits.max_tool_calls)?;
        Ok(Self {
            input,
            proof: None,
            limits,
            consent: permit.record(),
            state: PhantomData,
        })
    }

    /// Bind the draft to real passing proof bytes from CAS through a trusted record adapter.
    ///
    /// # Errors
    /// Consent revocation, missing/corrupt proof or any source/execution identity mismatch.
    pub fn verified(
        self,
        cas: &Cas,
        resolver: &impl ProofResolver,
        proof: Cid,
        optin: &OptIn,
    ) -> Result<Trajectory<Verified>, DataLabError> {
        let permit = optin.permit(self.input.session)?;
        let bytes = cas.get(&proof)?;
        if bytes.len() > self.limits.max_content_bytes {
            return Err(DataLabError::Limit);
        }
        if content_id(&bytes) != proof {
            return Err(DataLabError::DigestMismatch);
        }
        let facts = resolver.resolve(&bytes)?;
        if facts.status != ProofStatus::Passed
            || facts.session != self.input.session
            || facts.frame != self.input.frame
            || facts.workspace_snapshot != self.input.workspace_snapshot
            || facts.diff != self.input.diff
            || facts.test_run != self.input.test_run
        {
            return Err(DataLabError::NotVerified);
        }
        Ok(Trajectory {
            input: self.input,
            proof: Some(proof),
            limits: self.limits,
            consent: permit.record(),
            state: PhantomData,
        })
    }
}

/// Independent category-isolated signature plus entropy rescan of every decoded text value.
#[derive(Debug)]
pub struct ResidualGuard {
    secrets: DfaScanner,
    pii: DfaScanner,
    matches: usize,
    entropy: EntropyCfg,
}

impl ResidualGuard {
    /// Compile actual default patterns; heuristic entropy defaults remain uncalibrated.
    ///
    /// # Errors
    /// Rule compilation errors.
    pub fn new() -> Result<Self, DataLabError> {
        Ok(Self {
            secrets: DfaScanner::default_pack_for(&[Category::Secret])?,
            pii: DfaScanner::default_pack_for(&[Category::Pii])?,
            matches: 8192,
            entropy: EntropyCfg::default(),
        })
    }

    /// Reject any residual signature/entropy candidate. No finding echoes the matched value.
    ///
    /// # Errors
    /// Residual content or inability to finish the bounded scan.
    pub fn check(&self, bytes: &[u8]) -> Result<(), DataLabError> {
        for scanner in [&self.secrets, &self.pii] {
            if scanner.any_match(bytes)? {
                return Err(DataLabError::ResidualSecret);
            }
        }
        let mut out = ScanBuf::with_capacity(self.matches);
        let _count = high_entropy_spans(bytes, &self.entropy, &mut out)?;
        if !out.is_empty() {
            return Err(DataLabError::ResidualSecret);
        }
        Ok(())
    }
}

impl Trajectory<Verified> {
    /// Mask all decoded content, tool arguments, metadata and artifact copies before export.
    ///
    /// # Errors
    /// Revoked consent, content bounds, malformed arguments, residual sensitive data or CAS I/O.
    pub fn mask(
        mut self,
        cas: &Cas,
        redactor: &impl Redactor,
        guard: &ResidualGuard,
        optin: &OptIn,
    ) -> Result<Trajectory<Masked>, DataLabError> {
        let permit = optin.permit(self.input.session)?;
        let mut total = 0usize;
        for message in &mut self.input.messages {
            let raw = message
                .content
                .resolve(cas, self.limits.max_content_bytes)?;
            add_total(&mut total, raw.len(), self.limits.max_total_bytes)?;
            message.content = RawContent::text(mask_text(
                &raw,
                redactor,
                guard,
                self.limits.max_content_bytes,
            )?);
            if let Some(id) = &mut message.tool_call_id {
                *id = mask_text(id.as_bytes(), redactor, guard, 128)?;
            }
            for call in &mut message.tool_calls {
                call.id = mask_text(call.id.as_bytes(), redactor, guard, 128)?;
                call.name = mask_text(call.name.as_bytes(), redactor, guard, 128)?;
                let raw = call.arguments.resolve(cas, self.limits.max_content_bytes)?;
                add_total(&mut total, raw.len(), self.limits.max_total_bytes)?;
                // Masking may alter JSON contents, but must not turn tool calls into invalid data.
                let _: serde_json::Value = serde_json::from_slice(&raw)
                    .map_err(|_| DataLabError::Schema("tool arguments JSON"))?;
                let masked = mask_text(&raw, redactor, guard, self.limits.max_content_bytes)?;
                let value: serde_json::Value = serde_json::from_str(&masked)
                    .map_err(|_| DataLabError::Schema("masked tool arguments JSON"))?;
                scan_json_values(&value, guard, 0)?;
                call.arguments = RawContent::text(masked);
            }
        }
        for artifact in [&mut self.input.diff, &mut self.input.test_run] {
            let raw = RawContent::Blob(*artifact).resolve(cas, self.limits.max_content_bytes)?;
            add_total(&mut total, raw.len(), self.limits.max_total_bytes)?;
            let masked = Zeroizing::new(mask_text(
                &raw,
                redactor,
                guard,
                self.limits.max_content_bytes,
            )?);
            *artifact = cas.put(masked.as_bytes())?;
            if *artifact != content_id(masked.as_bytes()) {
                return Err(DataLabError::DigestMismatch);
            }
        }
        for text in [
            &mut self.input.provenance.repository,
            &mut self.input.provenance.license,
        ]
        .into_iter()
        .flatten()
        {
            *text = mask_text(
                text.as_bytes(),
                redactor,
                guard,
                self.limits.max_content_bytes,
            )?;
        }
        self.input.provenance.model = mask_text(
            self.input.provenance.model.as_bytes(),
            redactor,
            guard,
            1024,
        )?;
        for version in &mut self.input.provenance.tool_versions {
            *version = mask_text(version.as_bytes(), redactor, guard, 4096)?;
        }
        validate_messages(&self.input.messages, self.limits.max_tool_calls)?;
        Ok(Trajectory {
            input: self.input,
            proof: self.proof,
            limits: self.limits,
            consent: permit.record(),
            state: PhantomData,
        })
    }
}

fn mask_text(
    raw: &[u8],
    redactor: &impl Redactor,
    guard: &ResidualGuard,
    limit: usize,
) -> Result<String, DataLabError> {
    if raw.len() > limit {
        return Err(DataLabError::Limit);
    }
    let mut masked = Zeroizing::new(Vec::with_capacity(raw.len()));
    let _stats = redactor.redact_into(raw, &mut masked);
    if masked.len() > limit {
        return Err(DataLabError::Limit);
    }
    guard.check(&masked)?;
    let text = std::str::from_utf8(&masked).map_err(|_| DataLabError::Schema("text encoding"))?;
    Ok(text.to_owned())
}

fn add_total(total: &mut usize, bytes: usize, limit: usize) -> Result<(), DataLabError> {
    *total = total.checked_add(bytes).ok_or(DataLabError::Limit)?;
    if *total > limit {
        return Err(DataLabError::Limit);
    }
    Ok(())
}

fn scan_json_values(
    value: &serde_json::Value,
    guard: &ResidualGuard,
    depth: usize,
) -> Result<(), DataLabError> {
    if depth > 64 {
        return Err(DataLabError::Limit);
    }
    let next = depth.saturating_add(1);
    match value {
        serde_json::Value::String(text) => guard.check(text.as_bytes()),
        serde_json::Value::Array(values) => {
            for value in values {
                scan_json_values(value, guard, next)?;
            }
            Ok(())
        }
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                guard.check(key.as_bytes())?;
                scan_json_values(value, guard, next)?;
            }
            Ok(())
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            Ok(())
        }
    }
}

fn validate_messages(messages: &[MessageInput], max_calls: usize) -> Result<(), DataLabError> {
    use std::collections::BTreeSet;
    let mut calls = BTreeSet::new();
    let mut pending = BTreeSet::new();
    for message in messages {
        if !message.tool_calls.is_empty() && message.role != MessageRole::Assistant {
            return Err(DataLabError::Schema("tool calls on non-assistant message"));
        }
        for call in &message.tool_calls {
            if call.id.is_empty()
                || call.id.len() > 128
                || call.name.is_empty()
                || call.name.len() > 128
                || !call
                    .name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
                || !calls.insert(call.id.as_str())
            {
                return Err(DataLabError::Schema("tool call identity"));
            }
            let _inserted = pending.insert(call.id.as_str());
            if calls.len() > max_calls {
                return Err(DataLabError::Limit);
            }
        }
        match (&message.tool_call_id, message.role) {
            (Some(id), MessageRole::Tool) if pending.remove(id.as_str()) => {}
            (None, MessageRole::Tool) | (Some(_), _) => {
                return Err(DataLabError::Schema("unmatched tool result"));
            }
            (None, MessageRole::System | MessageRole::User | MessageRole::Assistant) => {}
        }
    }
    if !pending.is_empty() {
        return Err(DataLabError::Schema("unresolved tool calls"));
    }
    Ok(())
}
