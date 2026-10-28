//! Frame-bound model tools over actual kernel services. This is an allocating cold boundary.
//! Authorization is read-only; accepted executions have bounded owners that survive caller drop.
use crate::{
    AuthorizedFrameTool, FrameMutationConfig, FrameMutationError, FrameMutationRequest,
    FrameMutationService, FrameToolAccess, HumanRequestOutcome, KernelHandle, KernelToolRunner,
    frame_tools::{RunnerCancellation, sealed, serialize_bounded},
    runtime::FrameOperationOwner,
    storage::cas::content_id,
};
use polaris_runtime::frames::{
    FramePromptBinding, FrameStepBinding, FrameToolBinding, ToolCall, ToolResultRef, ToolRunner,
    role_allows_tool,
};
use polaris_sdk::{
    contracts::Event,
    human::{HumanQuestion, HumanRequestId, HumanRequestPayload},
    ids::{Cid, CommandId, FrameId, SessionId},
    registry::{Effects, ToolError, ToolSpec},
};
use polaris_security::capability::{
    Authorization, Facts, PublicKeyBytes, ResourcePath, Right, SystemClock, Token, authorize,
};
use polaris_system_one::verifier::MutationSafety;
use polaris_tools::builtin::{WorkspacePath, is_secret_path, resolve_in_workspace};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fmt, fs::File, io::Read, sync::Arc, time::Instant};
use tokio_util::sync::CancellationToken;
const MAX_ARGS: usize = 4 * 1024 * 1024;
const MAX_RESULT: usize = 2 * 1024 * 1024;
const MAX_FILE: usize = 4 * 1024 * 1024;

struct AgentAuthority {
    binding: FrameToolBinding,
    issuer: PublicKeyBytes,
    cancel: CancellationToken,
    session: SessionId,
    references: crate::frame_tools::FrameReferencePublisher,
    _cancellation_owner: RunnerCancellation,
}
impl fmt::Debug for AgentAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AgentAuthority(<redacted>)")
    }
}
impl AgentAuthority {
    fn check(&self, token: &Token) -> Result<(), ToolError> {
        if self.cancel.is_cancelled() || !self.binding.matches_token(token) {
            return Err(denied("frame token or cancellation mismatch"));
        }
        Ok(())
    }
    fn authorize(&self, token: &Token, resource: &str, right: Right) -> Result<(), ToolError> {
        self.check(token)?;
        let path = ResourcePath::new(resource).map_err(|_| invalid("invalid tool resource"))?;
        let facts = Facts {
            resource: &path,
            symbol: None,
            churn_lines: 0,
            touches_unsafe: false,
            diff_blake3: None,
            right,
        };
        if authorize(token, &facts, &self.issuer, &SystemClock)
            .map_err(|_| denied("tool capability denied"))?
            != Authorization::Allowed
        {
            return Err(denied("tool capability denied"));
        }
        Ok(())
    }
}

/// Kernel model-tool composition using real read, mutation and durable human-request services.
/// Supply a production semantic assessor in trusted host composition; leases come from the kernel.
/// Tool calls never issue human-answer or execution-approval credentials.
pub struct KernelAgentToolRunner<S> {
    kernel: KernelHandle,
    readonly: KernelToolRunner,
    mutations: Option<Arc<FrameMutationService<S>>>,
    state: Arc<AgentAuthority>,
}
impl<S> fmt::Debug for KernelAgentToolRunner<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KernelAgentToolRunner(<redacted>)")
    }
}
impl<S: MutationSafety + 'static> KernelAgentToolRunner<S> {
    /// Pin the actual kernel/frame/issuer, kernel-owned leases, semantic assessor and cancellation owner.
    /// # Errors
    /// A foreign binding, invalid policy or unavailable underlying service fails closed.
    pub fn new(
        kernel: KernelHandle,
        binding: FrameToolBinding,
        issuer: PublicKeyBytes,
        safety: Arc<S>,
        mut policy: FrameMutationConfig,
    ) -> Result<Self, ToolError> {
        let cancellation_owner =
            RunnerCancellation::new(&kernel, binding.clone(), policy.cancellation.clone())?;
        let cancellation = cancellation_owner.token();
        policy.cancellation = cancellation.clone();
        let readonly = KernelToolRunner::new(
            kernel.clone(),
            binding.clone(),
            issuer,
            cancellation.clone(),
        )?;
        let mutations = if role_allows_tool(binding.role(), write_effects()) {
            Some(Arc::new(
                FrameMutationService::new(kernel.clone(), binding.clone(), issuer, safety, policy)
                    .map_err(mutation_error)?,
            ))
        } else {
            None
        };
        let session = kernel.session();
        let references = readonly.reference_publisher();
        Ok(Self {
            kernel,
            readonly,
            mutations,
            state: Arc::new(AgentAuthority {
                binding,
                issuer,
                cancel: cancellation,
                session,
                references,
                _cancellation_owner: cancellation_owner,
            }),
        })
    }
    /// Cancel only this runner and its accepted calls. It never cancels a parent or sibling token.
    pub fn cancel(&self) {
        self.state.cancel.cancel();
    }

    /// Register an actual trusted scratchpad/archive CID for this frame's authorized retrieval.
    /// Registration does not grant CAS read rights or bypass per-call capability checks.
    /// # Errors
    /// Cancelled runner or exhausted/poisoned bounded reference storage.
    pub fn allow_reference(&self, cid: Cid) -> Result<(), ToolError> {
        self.readonly.allow_reference(cid)
    }

    async fn admit(
        &self,
        token: &Token,
        call: ToolCall,
        origin: Option<&FrameStepBinding>,
    ) -> Result<AgentFramePermit, ToolError> {
        self.state.check(token)?;
        if call.id.len() > 256
            || call.id.is_empty()
            || call.name.len() > 128
            || call.arguments.len() > MAX_ARGS
        {
            return Err(invalid("tool call exceeds input bound"));
        }
        let step = if let Some(origin) = origin {
            let binding = origin.frame_binding();
            if origin.step() == 0
                || binding.frame_id() != self.state.binding.frame_id()
                || binding.root_frame_id() != self.state.binding.root_frame_id()
                || binding.role() != self.state.binding.role()
                || !binding.matches_token(token)
            {
                return Err(denied("tool origin differs from actual frame"));
            }
            Some(origin.step())
        } else {
            None
        };
        let result_limit = origin.map(FrameStepBinding::max_result_bytes);
        let admission_cap = match result_limit {
            Some(limit) => limit.min(MAX_RESULT),
            None => MAX_RESULT,
        };
        if admission_cap < 512 {
            return Err(denied("driver result budget is too small"));
        }
        let spec = self
            .tool_specification(&call.name)
            .ok_or_else(|| invalid("unknown tool"))?;
        if !role_allows_tool(self.state.binding.role(), spec.effects) {
            return Err(denied("frame role denies tool"));
        }
        let name = call.name.clone();
        let id = call.id.clone();
        let arguments: Value = serde_json::from_str(&call.arguments)
            .map_err(|_| invalid("tool arguments must be bounded JSON"))?;
        let stored = StoredCall {
            version: 1,
            frame: self.state.binding.frame_id(),
            step,
            id,
            name: name.clone(),
            arguments: arguments.clone(),
        };
        let kind = match name.as_str() {
            "math" => {
                self.state
                    .authorize(token, "tools/math", Right::ExecuteTool)?;
                PermitKind::Math(decode(arguments)?)
            }
            "write_file" => {
                let input: WriteArgs = decode(arguments)?;
                let service = self
                    .mutations
                    .as_ref()
                    .ok_or_else(|| denied("frame cannot mutate"))?;
                let result_bound = result_bound(&input.path)?;
                if result_bound > admission_cap {
                    return Err(denied("mutation receipt exceeds driver result budget"));
                }
                PermitKind::Mutation {
                    prepared: Box::new(
                        service
                            .prepare_with_id(
                                token,
                                match input.command_id {
                                    Some(id) => id,
                                    None => CommandId::new_v7(),
                                },
                                FrameMutationRequest::WriteFile {
                                    path: input.path,
                                    contents: input.contents,
                                    expected_base: input.expected_base,
                                },
                            )
                            .await
                            .map_err(mutation_error)?,
                    ),
                    result_bound,
                }
            }
            "edit_file" => {
                let input: EditArgs = decode(arguments)?;
                let service = self
                    .mutations
                    .as_ref()
                    .ok_or_else(|| denied("frame cannot mutate"))?;
                let result_bound = result_bound(&input.path)?;
                if result_bound > admission_cap {
                    return Err(denied("mutation receipt exceeds driver result budget"));
                }
                PermitKind::Mutation {
                    prepared: Box::new(
                        service
                            .prepare_with_id(
                                token,
                                match input.command_id {
                                    Some(id) => id,
                                    None => CommandId::new_v7(),
                                },
                                FrameMutationRequest::EditFile {
                                    path: input.path,
                                    old_text: input.old_text,
                                    new_text: input.new_text,
                                    replace_all: input.replace_all,
                                    expected_base: input.expected_base,
                                },
                            )
                            .await
                            .map_err(mutation_error)?,
                    ),
                    result_bound,
                }
            }
            "apply_patch" => {
                let input: PatchArgs = decode(arguments)?;
                let service = self
                    .mutations
                    .as_ref()
                    .ok_or_else(|| denied("frame cannot mutate"))?;
                let result_bound = result_bound(&input.path)?;
                if result_bound > admission_cap {
                    return Err(denied("mutation receipt exceeds driver result budget"));
                }
                PermitKind::Mutation {
                    prepared: Box::new(
                        service
                            .prepare_with_id(
                                token,
                                match input.command_id {
                                    Some(id) => id,
                                    None => CommandId::new_v7(),
                                },
                                FrameMutationRequest::ApplyPatch {
                                    path: input.path,
                                    patch: input.patch,
                                    expected_base: input.expected_base,
                                },
                            )
                            .await
                            .map_err(mutation_error)?,
                    ),
                    result_bound,
                }
            }
            "file_revision" => {
                let input: PathArgs = decode(arguments)?;
                let path = WorkspacePath::parse(&input.path)
                    .map_err(|_| invalid("invalid workspace path"))?;
                if is_secret_path(&path) {
                    return Err(denied("workspace path denied"));
                }
                self.state.authorize(token, path.as_str(), Right::Read)?;
                PermitKind::Revision(path.as_str().to_owned())
            }
            "run_tests" => {
                self.state
                    .authorize(token, "tools/run_tests", Right::ExecuteTool)?;
                let request: crate::ExecutionRequest = decode(arguments)?;
                self.kernel
                    .validate_frame_test_request(self.state.binding.frame_id(), &request)
                    .map_err(|_| denied("frame sandbox execution profile or request denied"))?;
                if admission_cap < 2048 {
                    return Err(denied("test receipt exceeds result budget"));
                }
                PermitKind::Tests(request)
            }
            "ask_user" => {
                let step =
                    step.ok_or_else(|| invalid("ask_user requires actual runtime step context"))?;
                self.state
                    .authorize(token, "tools/ask_user", Right::ExecuteTool)?;
                let input: AskArgs = decode(arguments)?;
                let _validated = HumanRequestPayload::new(
                    HumanRequestId::new_v7(),
                    self.state.session,
                    self.state.binding.frame_id(),
                    step,
                    input.questions.clone(),
                )
                .map_err(|_| invalid("invalid human question batch"))?;
                PermitKind::Ask {
                    questions: input.questions,
                    step,
                }
            }
            _ => PermitKind::Read(Box::new(self.readonly.authorize(token, call).await?)),
        };
        Ok(AgentFramePermit {
            state: Arc::clone(&self.state),
            token: token.clone(),
            call: stored,
            kind,
            result_limit,
        })
    }
}

/// Single-use authorization produced only by the bound runner. Tokens and arguments are opaque.
pub struct AgentFramePermit {
    state: Arc<AgentAuthority>,
    token: Token,
    call: StoredCall,
    kind: PermitKind,
    result_limit: Option<usize>,
}
impl fmt::Debug for AgentFramePermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AgentFramePermit(<redacted>)")
    }
}
enum PermitKind {
    Math(polaris_tools::math::MathInput),
    Tests(crate::ExecutionRequest),
    Read(Box<AuthorizedFrameTool>),
    Mutation {
        prepared: Box<crate::PreparedFrameMutation>,
        result_bound: usize,
    },
    Ask {
        questions: Vec<HumanQuestion>,
        step: u32,
    },
    Revision(String),
}
#[derive(Serialize)]
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
struct WriteArgs {
    path: String,
    contents: String,
    #[serde(deserialize_with = "required_base")]
    expected_base: Option<Cid>,
    #[serde(default)]
    command_id: Option<CommandId>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditArgs {
    path: String,
    old_text: String,
    new_text: String,
    #[serde(default)]
    replace_all: bool,
    expected_base: Cid,
    #[serde(default)]
    command_id: Option<CommandId>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchArgs {
    path: String,
    patch: String,
    expected_base: Cid,
    #[serde(default)]
    command_id: Option<CommandId>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathArgs {
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AskArgs {
    questions: Vec<HumanQuestion>,
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ToolError> {
    serde_json::from_value(value).map_err(|_| invalid("invalid tool argument shape"))
}
fn required_base<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Option<Cid>, D::Error> {
    Option::<Cid>::deserialize(input)
}
fn write_effects() -> Effects {
    Effects {
        writes_workspace: true,
        ..Effects::READ_ONLY
    }
}
fn result_bound(path: &str) -> Result<usize, ToolError> {
    serde_json::to_vec(path)
        .map_err(|_| invalid("invalid result path"))?
        .len()
        .checked_add(2048)
        .ok_or_else(|| invalid("result bound overflow"))
}

impl<S: MutationSafety + 'static> ToolRunner for KernelAgentToolRunner<S> {
    type Authorized = AgentFramePermit;
    type Error = ToolError;
    fn advisory_facts(
        &self,
        call: &ToolCall,
    ) -> Option<polaris_system_one::watchdog::ToolCallFacts> {
        let spec = FrameToolAccess::tool_specification(self, &call.name)?;
        crate::frame_tools::advisory_facts(
            &call.name,
            crate::frame_tools::frame_tool_effects(&call.name, spec.effects),
        )
    }
    async fn authorize(
        &self,
        token: &Token,
        call: ToolCall,
    ) -> Result<Self::Authorized, Self::Error> {
        self.admit(token, call, None).await
    }
    async fn authorize_at_step(
        &self,
        token: &Token,
        origin: &FrameStepBinding,
        call: ToolCall,
    ) -> Result<Self::Authorized, Self::Error> {
        self.admit(token, call, Some(origin)).await
    }
    async fn execute(
        &self,
        permit: AgentFramePermit,
        frame: FrameId,
        max_result_bytes: usize,
    ) -> Result<ToolResultRef, ToolError> {
        if !Arc::ptr_eq(&self.state, &permit.state) || frame != self.state.binding.frame_id() {
            return Err(denied("permit belongs to another runner or frame"));
        }
        self.state.check(&permit.token)?;
        if permit
            .result_limit
            .is_some_and(|limit| limit != max_result_bytes)
        {
            return Err(denied("result budget differs from authorized step"));
        }
        let cap = max_result_bytes.min(MAX_RESULT);
        if cap < 512 {
            return Err(invalid("tool result budget is too small"));
        }
        if let PermitKind::Read(read) = permit.kind {
            return self.readonly.execute(*read, frame, cap).await;
        }
        if let PermitKind::Mutation { result_bound, .. } = &permit.kind
            && cap < *result_bound
        {
            return Err(invalid("mutation receipt exceeds result budget"));
        }
        let owner = self.kernel.frame_operation_owner().await?;
        let service = Arc::clone(self.kernel.human_requests());
        let mutations = self.mutations.as_ref().map(Arc::clone);
        let state = Arc::clone(&self.state);
        let cancel = self.state.cancel.child_token();
        let _drop = CancelOnDrop(cancel.clone());
        let task = tokio::spawn(async move {
            execute_owned(owner, state, permit, mutations, service, cancel, cap).await
        });
        task.await.map_err(|_| owner_error())?
    }
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
struct Output {
    value: Value,
    ok: bool,
}

async fn execute_owned<S: MutationSafety + 'static>(
    owner: FrameOperationOwner,
    state: Arc<AgentAuthority>,
    permit: AgentFramePermit,
    mutations: Option<Arc<FrameMutationService<S>>>,
    human: Arc<crate::HumanRequestService>,
    cancel: CancellationToken,
    cap: usize,
) -> Result<ToolResultRef, ToolError> {
    let AgentFramePermit {
        token, call, kind, ..
    } = permit;
    state.check(&token)?;
    if cancel.is_cancelled() {
        return Err(denied("tool invocation cancelled"));
    }
    let frame = state.binding.frame_id();
    let began = Instant::now();
    let context = Arc::clone(owner.context());
    let session = owner.session();
    let call_cid = tokio::task::spawn_blocking(move || {
        let bytes = serialize_bounded(&call, MAX_ARGS.saturating_add(4096))?;
        let cid = put_verified(&context, &bytes)?;
        let _writer = context.lock_writer().map_err(|_| owner_error())?;
        let _event = context
            .publish(session, Event::ToolExecutionStarted { frame, call: cid })
            .map_err(|_| owner_error())?;
        Ok::<_, ToolError>(cid)
    })
    .await
    .map_err(|_| owner_error())??;
    // A wait can outlive the capability even while the separately authorized host answer is valid.
    // Mutation final authorization is performed by its exact-diff service under the write lease.
    let completion_authority = match &kind {
        PermitKind::Math(_) => Some(("tools/math".to_owned(), Right::ExecuteTool)),
        PermitKind::Ask { .. } => Some(("tools/ask_user".to_owned(), Right::ExecuteTool)),
        PermitKind::Tests(_) => Some(("tools/run_tests".to_owned(), Right::ExecuteTool)),
        PermitKind::Revision(path) => Some((path.clone(), Right::Read)),
        PermitKind::Mutation { .. } | PermitKind::Read(_) => None,
    };
    let output = match kind {
        PermitKind::Math(input) => {
            let operation_cancel = cancel.clone();
            let math_state = Arc::clone(&state);
            let math_token = token.clone();
            tokio::task::spawn_blocking(move || {
                if operation_cancel.is_cancelled() {
                    return Err(denied("math operation cancelled"));
                }
                math_state.authorize(&math_token, "tools/math", Right::ExecuteTool)?;
                let value = polaris_tools::math::MathTool
                    .evaluate(input)
                    .map_err(|_| invalid("math input or computation rejected"))?;
                if operation_cancel.is_cancelled() {
                    return Err(denied("math operation cancelled"));
                }
                Ok::<_, ToolError>(Output {
                    value: serde_json::to_value(value).map_err(|_| owner_error())?,
                    ok: true,
                })
            })
            .await
            .map_err(|_| owner_error())?
        }
        PermitKind::Tests(request) => run_approved_tests(&owner, request, &cancel).await,
        PermitKind::Mutation { prepared, .. } => match mutations {
            Some(service) => match service.apply(*prepared, &token).await {
                Ok(receipt) => Ok(Output {
                    ok: true,
                    value: json!({"status":"applied","path":receipt.path(),"base":receipt.base(),"revision":receipt.new_cid(),"diff":receipt.diff(),"approval":receipt.approval_evidence(),"receipt":receipt.cid()}),
                }),
                Err(FrameMutationError::NeedsApproval { report, .. }) => Ok(Output {
                    ok: false,
                    value: json!({"status":"needs_approval","report":report}),
                }),
                Err(error) => Err(mutation_error(error)),
            },
            None => Err(denied("frame cannot mutate")),
        },
        PermitKind::Ask { questions, step } => {
            let request = human
                .ask(&state.binding, &token, &state.issuer, step, questions)
                .await
                .map_err(|_| invalid("human request admission failed"));
            match request {
                Err(error) => Err(error),
                Ok(ticket) => {
                    let shutdown = owner.shutdown_token();
                    let waiting = human.wait(ticket.request_id, cancel.clone());
                    tokio::pin!(waiting);
                    let outcome = tokio::select! {
                        result=&mut waiting=>result,
                        ()=shutdown.cancelled()=>{cancel.cancel();waiting.await},
                    };
                    match outcome {
                        Ok(HumanRequestOutcome::Answered { answer, .. }) => {
                            let context = Arc::clone(owner.context());
                            let references = state.references.clone();
                            let answer_token = token.clone();
                            tokio::task::spawn_blocking(move|| {
                                let bytes=context.cas.get_bounded(&answer,u64::try_from(polaris_sdk::human::MAX_HUMAN_ANSWER_BYTES).map_err(|_|owner_error())?).map_err(|_|owner_error())?;
                                let payload:Value=serde_json::from_slice(&bytes).map_err(|_|owner_error())?;
                                let expand_available=references.expose(&answer_token,answer)?;
                                let full=json!({"status":"answered","request_id":ticket.request_id,"answer_cid":answer,"answer":payload});
                                let value=if serialize_bounded(&full,cap).is_ok() { full } else { json!({"status":"answered","request_id":ticket.request_id,"answer_cid":answer,"truncated":true,"expand_available":expand_available}) };
                                Ok::<_,ToolError>(Output { value,ok:true })
                            }).await.map_err(|_|owner_error())?
                        }
                        Ok(HumanRequestOutcome::Interrupted { .. }) => {
                            Err(denied("human request interrupted"))
                        }
                        Ok(HumanRequestOutcome::Pending) => Err(owner_error()),
                        Err(_) => Err(owner_error()),
                    }
                }
            }
        }
        PermitKind::Revision(path) => {
            let context = Arc::clone(owner.context());
            let state = Arc::clone(&state);
            let token = token.clone();
            let root = owner.workspace_root().to_owned();
            tokio::task::spawn_blocking(move || {
                context.ensure_open().map_err(|_| owner_error())?;
                state.authorize(&token, &path, Right::Read)?;
                let resolved = resolve_in_workspace(&root, &path)
                    .map_err(|_| denied("workspace path denied"))?;
                let file = File::open(resolved).map_err(|_| owner_error())?;
                let metadata = file.metadata().map_err(|_| owner_error())?;
                let limit = u64::try_from(MAX_FILE).map_err(|_| owner_error())?;
                if !metadata.is_file() || metadata.len() > limit {
                    return Err(invalid("revision source exceeds file bound"));
                }
                let mut bytes = Vec::new();
                file.take(limit.saturating_add(1))
                    .read_to_end(&mut bytes)
                    .map_err(|_| owner_error())?;
                if bytes.len() > MAX_FILE {
                    return Err(invalid("revision source exceeds file bound"));
                }
                state.authorize(&token, &path, Right::Read)?;
                Ok::<_, ToolError>(Output {
                    ok: true,
                    value: json!({"path":path,"revision":content_id(&bytes),"bytes":bytes.len()}),
                })
            })
            .await
            .map_err(|_| owner_error())?
        }
        PermitKind::Read(_) => Err(invalid("readonly permit was not delegated")),
    };
    let context = Arc::clone(owner.context());
    tokio::task::spawn_blocking(move || {
        let output=output.and_then(|result| {
            state.check(&token)?;
            if cancel.is_cancelled() {return Err(denied("tool invocation cancelled"));}
            if let Some((resource,right))=&completion_authority {
                state.authorize(&token,resource,*right)?;
            }
            Ok(result)
        });
        let ok = matches!(&output,Ok(result) if result.ok);
        let result = match output {
            Ok(result) => (|| {
                let bytes = serialize_bounded(&result.value, cap)?;
                let content_cid = put_verified(&context, &bytes)?;
                let evidence = serialize_bounded(&json!({"version":1,"session":session,"frame":frame,"call":call_cid,"content":content_cid}), 1024)?;
                let cid = put_verified(&context, &evidence)?;
                Ok(ToolResultRef {
                    cid,
                    content_cid,
                    content: String::from_utf8(bytes).map_err(|_| owner_error())?,
                })
            })(),
            Err(error) => Err(error),
        };
        let elapsed = began.elapsed();
        let latency_us = elapsed
            .as_secs()
            .saturating_mul(1_000_000)
            .saturating_add(u64::from(elapsed.subsec_micros()));
        let _writer = context.lock_writer().map_err(|_| owner_error())?;
        if let Ok(value)=&result {
            let _recorded=context.publish(session,Event::FrameToolResultRecorded {frame,call:call_cid,result:value.cid}).map_err(|_|owner_error())?;
        }
        let finished = context
            .publish(
                session,
                Event::ToolExecutionCompleted {
                    frame,
                    call: call_cid,
                    ok: ok && result.is_ok(),
                    latency_us,
                },
            )
            .map_err(|_| owner_error());
        let _owner = owner;
        match result {
            Err(error) => Err(error),
            Ok(value) => {
                let _event = finished?;
                Ok(value)
            }
        }
    })
    .await
    .map_err(|_| owner_error())?
}
fn put_verified(context: &crate::KernelCtx, bytes: &[u8]) -> Result<Cid, ToolError> {
    let cid = context.cas.put(bytes).map_err(|_| owner_error())?;
    if cid != content_id(bytes)
        || context
            .cas
            .get_bounded(&cid, u64::try_from(bytes.len()).map_err(|_| owner_error())?)
            .map_err(|_| owner_error())?
            .as_ref()
            != bytes
    {
        return Err(owner_error());
    }
    Ok(cid)
}
impl<S: MutationSafety + 'static> sealed::FrameToolAccess for KernelAgentToolRunner<S> {}
impl<S: MutationSafety + 'static> FrameToolAccess for KernelAgentToolRunner<S> {
    fn validate_prompt_binding(
        &self,
        kernel: &KernelHandle,
        binding: &FramePromptBinding,
        token: &Token,
    ) -> Result<(), ToolError> {
        self.state.check(token)?;
        self.readonly
            .validate_prompt_binding(kernel, binding, token)
    }
    fn expose_prompt_reference(&self, token: &Token, cid: Cid) -> Result<(), ToolError> {
        self.state.check(token)?;
        self.readonly.expose_prompt_reference(token, cid)
    }
    fn cancellation_token(&self) -> CancellationToken {
        self.state.cancel.clone()
    }
    fn tool_specification(&self, name: &str) -> Option<ToolSpec> {
        if name == "math" {
            return Some(polaris_tools::math::MathTool.tool_spec());
        }
        let (description, schema, effects) = match name {
            "run_tests" => (
                "Prepare exact sandbox tests and an independent falsifier. Wait for explicit host approval of the review digest and return actual proof results.",
                r#"{"type":"object","additionalProperties":false,"properties":{"argv":{"type":"array","minItems":1,"items":{"type":"string"}},"independent_argv":{"type":"array","minItems":1,"items":{"type":"string"}},"timeout_secs":{"type":"integer","minimum":1,"maximum":900},"min_tests":{"type":"integer","minimum":1}},"required":["argv","independent_argv","timeout_secs","min_tests"]}"#,
                Effects {
                    executes: true,
                    ..Effects::READ_ONLY
                },
            ),
            "write_file" => (
                "Replace an exact revision or create an absent UTF-8 file. Obtain existing revisions with file_revision.",
                r#"{"type":"object","additionalProperties":false,"properties":{"command_id":{"type":"string","format":"uuid"},"path":{"type":"string"},"contents":{"type":"string"},"expected_base":{"type":["string","null"]}},"required":["path","contents","expected_base"]}"#,
                write_effects(),
            ),
            "edit_file" => (
                "Replace exact text in one current file revision. Obtain its revision with file_revision.",
                r#"{"type":"object","additionalProperties":false,"properties":{"command_id":{"type":"string","format":"uuid"},"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"},"replace_all":{"type":"boolean"},"expected_base":{"type":"string"}},"required":["path","old_text","new_text","expected_base"]}"#,
                write_effects(),
            ),
            "apply_patch" => (
                "Apply a single-file unified patch against an exact current revision.",
                r#"{"type":"object","additionalProperties":false,"properties":{"command_id":{"type":"string","format":"uuid"},"path":{"type":"string"},"patch":{"type":"string"},"expected_base":{"type":"string"}},"required":["path","patch","expected_base"]}"#,
                write_effects(),
            ),
            "file_revision" => (
                "Read the current content revision of a bounded regular workspace file.",
                r#"{"type":"object","additionalProperties":false,"properties":{"path":{"type":"string"}},"required":["path"]}"#,
                Effects::READ_ONLY,
            ),
            // Effects has no human/persistent-store bit. READ_ONLY is the conservative policy
            // floor also used for CAS/history; the separate ExecuteTool capability is required.
            "ask_user" => (
                "Ask a complete question batch and wait for the host's atomic answer. This never grants execution approval.",
                ASK_SCHEMA,
                Effects::READ_ONLY,
            ),
            _ => return self.readonly.tool_specification(name),
        };
        Some(ToolSpec {
            name: name.into(),
            description: description.into(),
            input_schema_json: schema.into(),
            effects,
        })
    }
}
const ASK_SCHEMA: &str = r#"{"type":"object","additionalProperties":false,"properties":{"questions":{"type":"array","minItems":1,"maxItems":8,"items":{"type":"object","additionalProperties":false,"properties":{"id":{"type":"string"},"prompt":{"type":"string"},"required":{"type":"boolean"},"kind":{"oneOf":[{"type":"object","additionalProperties":false,"properties":{"type":{"const":"free_text"}},"required":["type"]},{"type":"object","additionalProperties":false,"properties":{"type":{"enum":["single_choice","multi_choice"]},"options":{"type":"array","minItems":1,"maxItems":16,"items":{"type":"object","additionalProperties":false,"properties":{"id":{"type":"string"},"label":{"type":"string"}},"required":["id","label"]}}},"required":["type","options"]}]}},"required":["id","prompt","required","kind"]}}},"required":["questions"]}"#;
fn denied(message: &str) -> ToolError {
    ToolError::Forbidden(message.into())
}
fn invalid(message: &str) -> ToolError {
    ToolError::InvalidInput(message.into())
}
fn owner_error() -> ToolError {
    ToolError::Io("frame tool owner failed".into())
}
fn mutation_error(error: FrameMutationError) -> ToolError {
    match error {
        FrameMutationError::Authorization
        | FrameMutationError::Binding
        | FrameMutationError::Path => denied("mutation authorization or binding rejected"),
        FrameMutationError::AppliedButUnrecorded { path, actual } => ToolError::Io(format!(
            "mutation applied at {path} ({actual}); completion unrecorded; rescan required"
        )),
        FrameMutationError::StaleBase => {
            invalid("mutation base is stale; read the current revision")
        }
        FrameMutationError::Limit | FrameMutationError::InvalidProposal => {
            invalid("mutation proposal exceeds bounds or is invalid")
        }
        FrameMutationError::Owner
        | FrameMutationError::Workspace
        | FrameMutationError::Lease
        | FrameMutationError::Analysis
        | FrameMutationError::Verifier
        | FrameMutationError::Rejected(_)
        | FrameMutationError::NeedsApproval { .. }
        | FrameMutationError::Persistence => owner_error(),
    }
}

async fn run_approved_tests(
    owner: &FrameOperationOwner,
    request: crate::ExecutionRequest,
    cancel: &CancellationToken,
) -> Result<Output, ToolError> {
    let mut events = owner.context().subscribe();
    let review = owner
        .prepare_frame_tests(request)
        .await
        .map_err(|_| owner_error())?;
    let context = Arc::clone(owner.context());
    let session = owner.session();
    let shutdown = owner.shutdown_token();
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(1800));
    tokio::pin!(deadline);
    loop {
        let ctx = Arc::clone(&context);
        let review_for_read = review.clone();
        let frame =
            serde_json::from_value(review.details.get("frame").ok_or_else(owner_error)?.clone())
                .map_err(|_| owner_error())?;
        let terminal = tokio::task::spawn_blocking(move || {
            read_test_receipt(&ctx, session, frame, &review_for_read)
        })
        .await
        .map_err(|_| owner_error())??;
        if let Some((passed, report)) = terminal {
            return Ok(Output {
                ok: passed,
                value: json!({"status":if passed {"proof_passed"} else {"proof_blocked"},"review":review.digest,"tree":review.tree,"report":report}),
            });
        }
        // Unrelated frame traffic must not trigger another whole-session replay. A lag still
        // forces durable recovery; relevant events are only hints until the exact CAS/log lookup.
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(denied("test review wait interrupted")),
                () = shutdown.cancelled() => return Err(denied("kernel shutdown interrupted test review")),
                () = &mut deadline => return Err(denied("test review wait deadline exceeded")),
                result = events.next() => match result {
                    Ok(record) if record.session()==session => {
                        if matches!(record.event(),
                            Event::ToolExecutionStarted {frame:actual,call}
                            | Event::ToolExecutionCompleted {frame:actual,call,..}
                            if *actual==frame && *call==review.digest)
                            || matches!(record.event(),
                                Event::ProofOfDonePassed {frame:actual,..}
                                | Event::ProofOfDoneBlocked {frame:actual,..} if *actual==frame)
                        {break;}
                    },
                    Ok(_) => {},
                    Err(crate::bus::BusError::Lagged { .. }) => break,
                    Err(_) => return Err(owner_error()),
                },
            }
        }
    }
}

fn read_test_receipt(
    ctx: &crate::KernelCtx,
    session: SessionId,
    frame: FrameId,
    review: &crate::ExecutionReview,
) -> Result<Option<(bool, Cid)>, ToolError> {
    let mut started = false;
    let mut proof = None;
    for (index, record) in ctx
        .bus
        .store()
        .replay_iter(polaris_sdk::ids::Seq::new(u64::MAX))
        .map_err(|_| owner_error())?
        .enumerate()
    {
        if index >= 1_000_000 {
            return Err(denied("test receipt replay limit exceeded"));
        }
        let record = record.map_err(|_| owner_error())?;
        if record.session() != session {
            continue;
        }
        match record.event() {
            Event::ToolExecutionStarted {
                frame: actual,
                call,
            } if *actual == frame && *call == review.digest => {
                if started {
                    return Err(owner_error());
                }
                started = true;
            }
            Event::ProofOfDonePassed {
                frame: actual,
                report,
            }
            | Event::ProofOfDoneBlocked {
                frame: actual,
                report,
            } if started && *actual == frame => {
                let passed = matches!(record.event(), Event::ProofOfDonePassed { .. });
                let bytes = ctx
                    .cas
                    .get_bounded(report, 1024 * 1024)
                    .map_err(|_| owner_error())?;
                if content_id(&bytes) != *report {
                    return Err(owner_error());
                }
                let value: Value = serde_json::from_slice(&bytes).map_err(|_| owner_error())?;
                if value.get("review") != Some(&json!(review.digest)) {
                    return Err(owner_error());
                }
                if value.get("schema") == Some(&json!(2)) {
                    if value.get("session") != Some(&json!(session))
                        || value.get("frame") != Some(&json!(frame))
                        || value.get("tree") != Some(&json!(review.tree))
                        || value.get("passed") != Some(&json!(passed))
                    {
                        return Err(owner_error());
                    }
                } else if passed
                    || !matches!(
                        value.get("stage").and_then(Value::as_str),
                        Some("sandbox_execution" | "sandbox_cleanup")
                    )
                {
                    return Err(owner_error());
                }
                proof = Some((passed, *report));
            }
            Event::ToolExecutionCompleted {
                frame: actual,
                call,
                ok,
                ..
            } if started && *actual == frame && *call == review.digest => {
                let (passed, report) = proof.ok_or_else(owner_error)?;
                if passed != *ok {
                    return Err(owner_error());
                }
                return Ok(Some((passed, report)));
            }
            _ => {}
        }
    }
    Ok(None)
}

#[cfg(test)]
#[path = "agent_tools_receipt_tests.rs"]
mod receipt_tests;
