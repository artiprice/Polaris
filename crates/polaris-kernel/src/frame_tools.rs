//! Frame-bound readonly tool dispatch over the kernel's actual workspace, CAS and history.
//!
//! This is an allocating inspection boundary, outside the event bus hot path. Capabilities use
//! lexical workspace identities; builtin containment checks assume a stable workspace. Cancelling
//! a future signals its blocking owner but cannot preempt an operating-system read. The owner
//! retains shutdown guards until it exits. Cancellation during a CAS write may leave an orphan.
use crate::{KernelHandle, runtime::InspectionOwner, storage::cas::content_id};
use polaris_runtime::frames::{
    FramePromptBinding, FrameToolBinding, ToolCall, ToolResultRef, ToolRunner, role_allows_tool,
};
use polaris_sdk::{
    ids::{Cid, FrameId, ShortRef},
    registry::{Effects, ToolError, ToolSpec},
};
use polaris_security::capability::{
    Authorization, Facts, PublicKeyBytes, ResourcePath, Right, SystemClock, Token,
    authorize_with_revocations,
};
use polaris_system_one::watchdog::{ToolCallFacts, ToolId};
use polaris_tools::builtin::{
    AccessResource, ToolAccessPolicy, WorkspacePath, validate_readonly_arguments,
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fmt,
    io::{self, Write},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

const MAX_ARGUMENT_BYTES: usize = 65_536;
const MAX_RESULT_BYTES: usize = 2 * 1024 * 1024;
const MAX_REFERENCES: usize = 4096;
const MAX_REVOCATIONS: usize = 4096;

#[derive(Debug)]
struct AuthorityState {
    binding: FrameToolBinding,
    issuer: PublicKeyBytes,
    references: Mutex<BTreeSet<Cid>>,
    revoked: Mutex<BTreeSet<Vec<u8>>>,
    cancellation: CancellationToken,
    _cancellation_owner: RunnerCancellation,
}

/// Owns the link from external/session cancellation into a private runner child.
pub(crate) struct RunnerCancellation {
    token: CancellationToken,
    watcher: tokio::task::JoinHandle<()>,
}
impl fmt::Debug for RunnerCancellation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RunnerCancellation(<private>)")
    }
}
impl RunnerCancellation {
    pub(crate) fn new(
        kernel: &KernelHandle,
        binding: FrameToolBinding,
        external: CancellationToken,
    ) -> Result<Self, ToolError> {
        let canonical = kernel
            .frame_cancellations()
            .bind_blocking(binding)
            .map_err(|_| denied("actual frame registry rejected binding"))?
            .cancellation_token();
        let token = canonical.child_token();
        if external.is_cancelled() {
            token.cancel();
        }
        let signal = token.clone();
        let watcher=tokio::runtime::Handle::try_current().map_err(|_|owner_error())?.spawn(async move {
            tokio::select! { ()=external.cancelled()=>signal.cancel(), ()=signal.cancelled()=>{} }
        });
        Ok(Self { token, watcher })
    }
    pub(crate) fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}
impl Drop for RunnerCancellation {
    fn drop(&mut self) {
        self.token.cancel();
        self.watcher.abort();
    }
}

/// Production readonly adapter for [`ToolRunner`], pinned to one active frame and kernel root.
///
/// Construct this in trusted composition code with the actual session authority public key.
/// The sealed binding pins local identity; Biscuit tokens retain their bearer semantics.
#[derive(Debug)]
pub struct KernelToolRunner {
    kernel: KernelHandle,
    state: Arc<AuthorityState>,
}

/// Cold reference admission without retaining an external kernel handle or widening authority.
#[derive(Debug, Clone)]
pub(crate) struct FrameReferencePublisher(Arc<AuthorityState>);

impl FrameReferencePublisher {
    pub(crate) fn expose(&self, token: &Token, cid: Cid) -> Result<bool, ToolError> {
        let resource = resource_path(&format!("cas/{}", cid.to_hex()))?;
        match self.0.authorize(token, &resource, Right::Read) {
            Ok(()) => {}
            Err(ToolError::Forbidden(_)) => return Ok(false),
            Err(error) => return Err(error),
        }
        self.0.add_reference(cid)?;
        Ok(true)
    }
}

/// Single-use authorization witness. Its call, token, frame and issuing runner are private.
pub struct AuthorizedFrameTool {
    state: Arc<AuthorityState>,
    token: Token,
    name: String,
    arguments: Value,
    admission: ResourcePath,
    right: Right,
    reference: Option<Cid>,
    /// A model-correctable rejection (malformed arguments, unknown tool). Executing it performs
    /// no tool work and returns the reason to the model so it can retry; policy denials never
    /// take this path.
    rejection: Option<ToolError>,
}

/// Errors the model can fix by retrying with different arguments.
fn model_correctable(error: &ToolError) -> bool {
    matches!(error, ToolError::InvalidInput(_) | ToolError::NotFound(_))
}

impl fmt::Debug for AuthorizedFrameTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AuthorizedFrameTool(<redacted>)")
    }
}

pub(crate) mod sealed {
    pub trait FrameToolAccess {}
}

/// Kernel-owned prompt catalog and identity port for the exact runner that executes a frame.
/// Implementations are sealed to this composition crate; tool effects remain role checked.
pub trait FrameToolAccess: ToolRunner<Error = ToolError> + sealed::FrameToolAccess {
    /// Validate actual kernel/frame/token identity before prompt assembly or dispatch.
    /// # Errors
    /// Foreign identity, revoked authority, or cancelled frame.
    fn validate_prompt_binding(
        &self,
        kernel: &KernelHandle,
        binding: &FramePromptBinding,
        token: &Token,
    ) -> Result<(), ToolError>;
    /// Admit an actual archive reference without bypassing its separate Read capability.
    /// # Errors
    /// Denied, cancelled, or bounded-reference admission failure.
    fn expose_prompt_reference(&self, token: &Token, cid: Cid) -> Result<(), ToolError>;
    /// Return the implemented specification for this exact runner.
    fn tool_specification(&self, name: &str) -> Option<ToolSpec>;
    /// The cancellation signal owned by this frame runner.
    fn cancellation_token(&self) -> CancellationToken;
}
impl sealed::FrameToolAccess for KernelToolRunner {}
impl FrameToolAccess for KernelToolRunner {
    fn validate_prompt_binding(
        &self,
        kernel: &KernelHandle,
        binding: &FramePromptBinding,
        token: &Token,
    ) -> Result<(), ToolError> {
        Self::validate_prompt_binding(self, kernel, binding, token)
    }
    fn expose_prompt_reference(&self, token: &Token, cid: Cid) -> Result<(), ToolError> {
        Self::expose_prompt_reference(self, token, cid)
    }
    fn tool_specification(&self, name: &str) -> Option<ToolSpec> {
        self.kernel.tools().get(name).cloned()
    }
    fn cancellation_token(&self) -> CancellationToken {
        self.state.cancellation.clone()
    }
}

impl KernelToolRunner {
    pub(crate) fn reference_publisher(&self) -> FrameReferencePublisher {
        FrameReferencePublisher(Arc::clone(&self.state))
    }
    pub(crate) fn validate_prompt_binding(
        &self,
        kernel: &KernelHandle,
        binding: &FramePromptBinding,
        token: &Token,
    ) -> Result<(), ToolError> {
        if !Arc::ptr_eq(self.kernel.context(), kernel.context())
            || self.state.binding.frame_id() != binding.frame_id()
            || self.state.binding.root_frame_id() != binding.root_frame_id()
            || self.state.binding.role() != binding.role()
            || !self.state.binding.matches_token(token)
        {
            return Err(denied("prompt and tool identities differ"));
        }
        self.state.check_cancel()
    }

    pub(crate) fn expose_prompt_reference(&self, token: &Token, cid: Cid) -> Result<(), ToolError> {
        let resource = resource_path(&format!("cas/{}", cid.to_hex()))?;
        self.state.authorize(token, &resource, Right::Read)?;
        self.state.add_reference(cid)
    }

    /// Bind the real kernel and a sealed active-frame witness to an explicit capability issuer.
    /// This cold constructor performs durable frame registration; asynchronous hosts should call it on a blocking owner.
    ///
    /// # Errors
    /// Rejects a binding whose root frame is not this kernel's root frame.
    pub fn new(
        kernel: KernelHandle,
        binding: FrameToolBinding,
        issuer: PublicKeyBytes,
        cancellation: CancellationToken,
    ) -> Result<Self, ToolError> {
        if binding.root_frame_id() != kernel.frame() {
            return Err(denied("frame belongs to a different kernel root"));
        }
        let cancellation_owner = RunnerCancellation::new(&kernel, binding.clone(), cancellation)?;
        let cancellation = cancellation_owner.token();
        Ok(Self {
            kernel,
            state: Arc::new(AuthorityState {
                binding,
                issuer,
                references: Mutex::new(BTreeSet::new()),
                revoked: Mutex::new(BTreeSet::new()),
                cancellation,
                _cancellation_owner: cancellation_owner,
            }),
        })
    }

    /// Admit a context/archive CID supplied by trusted composition code for this frame.
    /// This does not bypass the token's separate `Read` check on `cas/<full CID>`.
    ///
    /// # Errors
    /// Fails closed on cancellation, poisoned state or the 4096-reference bound.
    pub fn allow_reference(&self, cid: Cid) -> Result<(), ToolError> {
        self.state.check_cancel()?;
        self.state.add_reference(cid)
    }

    /// Revoke this token chain locally for this runner, including outstanding permits.
    /// Revocation is checked at admission, execution and every builtin resource boundary.
    /// It is in-memory policy, not a durable or global revocation service.
    ///
    /// # Errors
    /// Fails closed on poisoned state or the bounded revocation ledger capacity.
    pub fn revoke_token(&self, token: &Token) -> Result<(), ToolError> {
        let ids = token.revocation_identifiers();
        let mut revoked = self.state.revoked.lock().map_err(|_| owner_error())?;
        let additions = ids.iter().filter(|id| !revoked.contains(*id)).count();
        let total = revoked
            .len()
            .checked_add(additions)
            .ok_or_else(owner_error)?;
        if total > MAX_REVOCATIONS {
            return Err(denied("revocation ledger full"));
        }
        revoked.extend(ids);
        Ok(())
    }
}

impl AuthorityState {
    fn check_cancel(&self) -> Result<(), ToolError> {
        if self.cancellation.is_cancelled() {
            return Err(denied("frame cancelled"));
        }
        Ok(())
    }
    fn add_reference(&self, cid: Cid) -> Result<(), ToolError> {
        let mut refs = self.references.lock().map_err(|_| owner_error())?;
        if !refs.contains(&cid) && refs.len() >= MAX_REFERENCES {
            return Err(denied("frame reference ledger full"));
        }
        let _inserted = refs.insert(cid);
        Ok(())
    }
    fn check_reference(&self, cid: Cid) -> Result<(), ToolError> {
        if !self
            .references
            .lock()
            .map_err(|_| owner_error())?
            .contains(&cid)
        {
            return Err(denied("content reference is not visible to this frame"));
        }
        Ok(())
    }
    fn authorize(
        &self,
        token: &Token,
        resource: &ResourcePath,
        right: Right,
    ) -> Result<(), ToolError> {
        self.check_cancel()?;
        // Keep revocation linearized with the check: a completed revoke cannot be omitted by
        // an authorization that is still evaluating an older snapshot.
        let revocation_guard = self.revoked.lock().map_err(|_| owner_error())?;
        let revoked: Vec<Vec<u8>> = revocation_guard.iter().cloned().collect();
        let facts = Facts {
            resource,
            symbol: None,
            churn_lines: 0,
            touches_unsafe: false,
            diff_blake3: None,
            right,
        };
        match authorize_with_revocations(token, &facts, &self.issuer, &SystemClock, &revoked) {
            Ok(Authorization::Allowed) => Ok(()),
            Ok(Authorization::Denied { .. }) | Err(_) => Err(denied("capability denied")),
        }
    }
}

struct InvocationPolicy<'a> {
    state: &'a AuthorityState,
    token: &'a Token,
    operation: &'a CancellationToken,
}

impl InvocationPolicy<'_> {
    fn check(&self) -> Result<(), ToolError> {
        self.state.check_cancel()?;
        if self.operation.is_cancelled() {
            return Err(denied("tool invocation cancelled"));
        }
        Ok(())
    }
}

impl ToolAccessPolicy for InvocationPolicy<'_> {
    fn authorize(&self, resource: AccessResource<'_>) -> Result<(), ToolError> {
        self.check()?;
        let resource = match resource {
            AccessResource::WorkspaceRoot => resource_path("__polaris_workspace__")?,
            AccessResource::WorkspacePath(path) => resource_path(path.as_str())?,
        };
        self.state.authorize(self.token, &resource, Right::Read)
    }
}

struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl ToolRunner for KernelToolRunner {
    type Authorized = AuthorizedFrameTool;
    type Error = ToolError;

    fn advisory_facts(&self, call: &ToolCall) -> Option<ToolCallFacts> {
        let spec = self.kernel.tools().get(&call.name)?;
        advisory_facts(&call.name, frame_tool_effects(&call.name, spec.effects))
    }

    async fn authorize(
        &self,
        token: &Token,
        call: ToolCall,
    ) -> Result<Self::Authorized, ToolError> {
        self.state.check_cancel()?;
        if !self.state.binding.matches_token(token) {
            return Err(denied("token does not match active frame binding"));
        }
        let name: String = call.name.chars().take(128).collect();
        match self.authorize_call(token, call).await {
            Err(error) if model_correctable(&error) => Ok(AuthorizedFrameTool {
                state: Arc::clone(&self.state),
                token: token.clone(),
                name,
                arguments: Value::Null,
                admission: resource_path("__polaris_workspace__")?,
                right: Right::Read,
                reference: None,
                rejection: Some(error),
            }),
            other => other,
        }
    }

    async fn execute(
        &self,
        permit: Self::Authorized,
        frame: FrameId,
        max_result_bytes: usize,
    ) -> Result<ToolResultRef, ToolError> {
        if !Arc::ptr_eq(&self.state, &permit.state) || frame != self.state.binding.frame_id() {
            return Err(denied("permit frame or runner mismatch"));
        }
        if max_result_bytes == 0 {
            return Err(invalid("result bound must be positive"));
        }
        self.state.check_cancel()?;
        let owner = tokio::select! {
            biased;
            () = self.state.cancellation.cancelled() => return Err(denied("frame cancelled")),
            owner = self.kernel.inspection_owner() => owner?,
        };
        let cap = max_result_bytes.min(MAX_RESULT_BYTES);
        let operation = self.state.cancellation.child_token();
        let _cancel = CancelOnDrop(operation.clone());
        tokio::task::spawn_blocking(move || execute_owned(owner, permit, frame, cap, &operation))
            .await
            .map_err(|_| owner_error())?
    }
}

impl KernelToolRunner {
    /// Full admission of one call; the trait method turns correctable errors into rejections.
    async fn authorize_call(
        &self,
        token: &Token,
        call: ToolCall,
    ) -> Result<AuthorizedFrameTool, ToolError> {
        if call.arguments.len() > MAX_ARGUMENT_BYTES || call.name.len() > 128 || call.id.len() > 256
        {
            return Err(invalid("tool call exceeds input bound"));
        }
        let mut arguments: Value = serde_json::from_str(&call.arguments)
            .map_err(|_| invalid("tool arguments must be JSON"))?;
        validate_readonly_arguments(&call.name, &arguments)?;
        let spec = self
            .kernel
            .tools()
            .get(&call.name)
            .ok_or_else(|| invalid("unknown tool"))?;
        if !role_allows_tool(
            self.state.binding.role(),
            frame_tool_effects(&call.name, spec.effects),
        ) {
            return Err(denied("frame role denies tool"));
        }
        let owner = tokio::select! {
            biased;
            () = self.state.cancellation.cancelled() => return Err(denied("frame cancelled")),
            owner = self.kernel.inspection_owner() => owner?,
        };
        let state = Arc::clone(&self.state);
        let token = token.clone();
        let operation = self.state.cancellation.child_token();
        let _cancel = CancelOnDrop(operation.clone());
        tokio::task::spawn_blocking(move || {
            owner.ensure_open()?;
            if operation.is_cancelled() {
                return Err(denied("tool admission cancelled"));
            }
            let (admission, right, reference) =
                admission(&owner, &state, &token, &call.name, &mut arguments)?;
            state.authorize(&token, &admission, right)?;
            Ok(AuthorizedFrameTool {
                state,
                token,
                name: call.name,
                arguments,
                admission,
                right,
                reference,
                rejection: None,
            })
        })
        .await
        .map_err(|_| owner_error())?
    }
}

pub(crate) fn advisory_facts(name: &str, effects: Effects) -> Option<ToolCallFacts> {
    Some(ToolCallFacts {
        tool: ToolId::new(name).ok()?,
        workspace_path: None,
        writes_workspace: effects.writes_workspace,
        network: effects.network,
        executes: effects.executes,
    })
}

fn admission(
    owner: &InspectionOwner,
    state: &AuthorityState,
    token: &Token,
    name: &str,
    arguments: &mut Value,
) -> Result<(ResourcePath, Right, Option<Cid>), ToolError> {
    match name {
        "read_file" | "skeleton" | "list_dir" | "grep" => {
            let resource = match arguments.get("path") {
                Some(Value::String(path)) => {
                    let path = WorkspacePath::parse(path)
                        .map_err(|_| invalid("invalid workspace path"))?;
                    resource_path(path.as_str())?
                }
                None | Some(Value::Null) if matches!(name, "list_dir" | "grep") => {
                    resource_path("__polaris_workspace__")?
                }
                _ => return Err(invalid("workspace path is required")),
            };
            Ok((resource, Right::Read, None))
        }
        "glob" | "repo_map" => Ok((resource_path("__polaris_workspace__")?, Right::Read, None)),
        "polaris_recall" => Ok((
            resource_path(&format!("session/{}/history", owner.session()))?,
            Right::Read,
            None,
        )),
        "polaris_expand" => {
            let reference = arguments
                .get("reference")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("content reference required"))?;
            let cid = if reference.len() == 64 {
                Cid::from_hex(reference).map_err(|_| invalid("invalid content id"))?
            } else {
                let short: ShortRef = reference
                    .parse()
                    .map_err(|_| invalid("invalid short reference"))?;
                // First identify an already-visible CID without touching CAS metadata. Only
                // after capability authorization may the store check global shortref ambiguity.
                let visible = {
                    let references = state.references.lock().map_err(|_| owner_error())?;
                    let mut matches = references.iter().filter(|cid| ShortRef::of(cid) == short);
                    let candidate = matches
                        .next()
                        .copied()
                        .ok_or_else(|| denied("content reference unavailable"))?;
                    if matches.next().is_some() {
                        return Err(denied("content reference unavailable"));
                    }
                    candidate
                };
                state.authorize(
                    token,
                    &resource_path(&format!("cas/{}", visible.to_hex()))?,
                    Right::Read,
                )?;
                let resolved = owner
                    .cas()
                    .resolve(&short)
                    .map_err(|_| denied("content reference unavailable"))?;
                if resolved != visible {
                    return Err(denied("content reference unavailable"));
                }
                resolved
            };
            state.check_reference(cid)?;
            let object = arguments
                .as_object_mut()
                .ok_or_else(|| invalid("arguments must be an object"))?;
            let _old = object.insert("reference".to_owned(), Value::String(cid.to_hex()));
            Ok((
                resource_path(&format!("cas/{}", cid.to_hex()))?,
                Right::Read,
                Some(cid),
            ))
        }
        "json_query" | "regex_query" | "diff_apply" | "classify_command" => Ok((
            resource_path(&format!("tools/{name}"))?,
            Right::ExecuteTool,
            None,
        )),
        _ => Err(denied("tool is not in readonly frame profile")),
    }
}

fn execute_owned(
    owner: InspectionOwner,
    permit: AuthorizedFrameTool,
    frame: FrameId,
    cap: usize,
    operation: &CancellationToken,
) -> Result<ToolResultRef, ToolError> {
    owner.ensure_open()?;
    let policy = InvocationPolicy {
        state: &permit.state,
        token: &permit.token,
        operation,
    };
    policy.check()?;
    if let Some(rejection) = permit.rejection.as_ref() {
        return correctable_result(&owner, &permit, rejection, cap);
    }
    permit
        .state
        .authorize(&permit.token, &permit.admission, permit.right)?;
    if let Some(cid) = permit.reference {
        permit.state.check_reference(cid)?;
    }
    let cap_u64 = u64::try_from(cap).map_err(|_| invalid("result bound exceeds platform"))?;
    let output =
        match owner.invoke_authorized(&permit.name, &permit.arguments, frame, cap_u64, &policy) {
            Ok(output) => output,
            Err(error) if model_correctable(&error) => {
                return correctable_result(&owner, &permit, &error, cap);
            }
            Err(error) => return Err(error),
        };
    policy.check()?;
    // Preserve both the value and its truncation metadata. The envelope itself must fit.
    let value_bytes = serialize_bounded(&output.value, cap)?;
    if u64::try_from(value_bytes.len()).map_err(|_| owner_error())? != output.bytes {
        return Err(owner_error());
    }
    let envelope = serialize_bounded(&output, cap)?;
    policy.check()?;
    owner.ensure_open()?;
    // Recheck expiry/revocation after a potentially slow filesystem read and before publication.
    permit
        .state
        .authorize(&permit.token, &permit.admission, permit.right)?;
    let expected = content_id(&envelope);
    let cid = owner.cas().put(&envelope).map_err(|_| owner_error())?;
    let stored = owner
        .cas()
        .get_bounded(&cid, cap_u64)
        .map_err(|_| owner_error())?;
    if cid != expected || content_id(&stored) != expected || stored.as_ref() != envelope.as_slice()
    {
        return Err(owner_error());
    }
    policy.check()?;
    permit
        .state
        .authorize(&permit.token, &permit.admission, permit.right)?;
    permit.state.add_reference(cid)?;
    let content = String::from_utf8(envelope).map_err(|_| owner_error())?;
    Ok(ToolResultRef {
        cid,
        content_cid: cid,
        content,
    })
}

/// Publish a model-correctable failure as an ordinary CAS-backed tool result. The message is a
/// static kernel diagnostic (never file content), so the model can fix its arguments and retry.
fn correctable_result(
    owner: &InspectionOwner,
    permit: &AuthorizedFrameTool,
    error: &ToolError,
    cap: usize,
) -> Result<ToolResultRef, ToolError> {
    let envelope = serde_json::to_vec(&serde_json::json!({
        "tool": permit.name,
        "error": error.to_string(),
        "hint": "No tool work was performed. Correct the arguments (workspace-relative paths, existing files, the documented schema) and call the tool again.",
    }))
    .map_err(|_| owner_error())?;
    if envelope.len() > cap {
        return Err(invalid("tool error exceeds the result bound"));
    }
    owner.ensure_open()?;
    let expected = content_id(&envelope);
    let cid = owner.cas().put(&envelope).map_err(|_| owner_error())?;
    if cid != expected {
        return Err(owner_error());
    }
    permit.state.add_reference(cid)?;
    let content = String::from_utf8(envelope).map_err(|_| owner_error())?;
    Ok(ToolResultRef {
        cid,
        content_cid: cid,
        content,
    })
}

struct BoundedWriter {
    bytes: Vec<u8>,
    cap: usize,
}
impl Write for BoundedWriter {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let next = self
            .bytes
            .len()
            .checked_add(input.len())
            .ok_or_else(|| io::Error::other("output bound overflow"))?;
        if next > self.cap {
            return Err(io::Error::other("output bound exceeded"));
        }
        self.bytes.extend_from_slice(input);
        Ok(input.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(crate) fn serialize_bounded(
    value: &impl serde::Serialize,
    cap: usize,
) -> Result<Vec<u8>, ToolError> {
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        cap,
    };
    serde_json::to_writer(&mut writer, value)
        .map_err(|_| invalid("tool result exceeds output bound"))?;
    Ok(writer.bytes)
}
fn resource_path(path: &str) -> Result<ResourcePath, ToolError> {
    ResourcePath::new(path).map_err(|_| invalid("invalid authorization resource"))
}
fn denied(message: &str) -> ToolError {
    ToolError::Forbidden(message.to_owned())
}
fn invalid(message: &str) -> ToolError {
    ToolError::InvalidInput(message.to_owned())
}
fn owner_error() -> ToolError {
    ToolError::Io("frame tool owner failed".to_owned())
}

// CAS and history reads must not inherit the catalog's pure in-memory query classification.
// Effects currently has no persistent-store read bit; READ_ONLY is the conservative role input.
pub(crate) fn frame_tool_effects(name: &str, declared: Effects) -> Effects {
    match name {
        "polaris_expand" | "polaris_recall" => Effects::READ_ONLY,
        _ => declared,
    }
}

#[cfg(test)]
mod tests {
    use super::serialize_bounded;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn bounded_serialization_preserves_exact_json_or_rejects(text in ".{0,1024}", cap in 0_usize..4096) {
            let expected = serde_json::to_vec(&text);
            let result = serialize_bounded(&text, cap);
            match expected {
                Ok(bytes) if bytes.len() <= cap => {
                    prop_assert!(matches!(result, Ok(actual) if actual == bytes));
                }
                Ok(_) => prop_assert!(result.is_err()),
                Err(_) => prop_assert!(result.is_err()),
            }
        }
    }
}
