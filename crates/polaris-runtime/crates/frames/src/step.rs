//! One bounded model step with cancellation, tool authorization, and independent verification.
use std::{collections::{BTreeMap, BTreeSet}, future::Future};
use polaris_sdk::{contracts::{ChatChunk, ChatMessage, ChatRequest, FinishReason, Usage}, error::HasKind, ids::{AgentRole, Cid, FastFingerprint, FrameId, TokenCount}, registry::{ChatChunkSource, ChatModel, ChatToolCall, Effects, FunctionalOperator, ToolSpec}};
use polaris_security_capability::Token;
use tokio_util::sync::CancellationToken;
use crate::{Active, AgentContextFrame, FrameError, FrameView};

/// Immutable input to the assembly operator.
#[derive(Debug, Clone)]
pub struct AssembleRequest {
    /// Secret-free frame state.
    pub frame: FrameView,
    /// The caller's current task text.
    pub goal: String,
}

/// Prompt components assembled by the caller's real context and directive pipeline.
#[derive(Debug, Clone)]
pub struct FramePrompt {
    /// Ordered conversation; system/policy content is supplied by the assembler.
    pub messages: Vec<ChatMessage>,
    /// Tools enabled by the assembler's role policy.
    pub tools: Vec<ToolSpec>,
    /// Upper bound from the assembler's actual tokenizer, not a byte heuristic.
    pub input_token_upper_bound: TokenCount,
    /// Requested output cap; frame quota clamps this value.
    pub max_output_tokens: Option<TokenCount>,
}

/// A complete model-proposed tool call. It is untrusted until the injected authorizer approves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    /// Provider call identifier.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// JSON argument bytes as text.
    pub arguments: String,
}

/// A tool result returned by the real tool runner. `content` is model-visible data, not proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResultRef {
    /// CAS reference to the persisted result, created by the tool runner.
    pub cid: Cid,
    /// Bounded model-visible result text.
    pub content: String,
}

/// A tool result linked to the call identifier issued by the model. The frame creates this
/// binding from the streamed assistant call; tool backends cannot choose or replace the ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutedToolResult {
    /// Exact provider call ID that must be used when replaying this result.
    pub tool_call_id: String,
    /// Result returned by the authorized backend.
    pub result: ToolResultRef,
}

/// Only the authorizer can produce its implementation-specific permit, and execution consumes that
/// permit rather than a raw model call. Implementations must call the capability API and role policy
/// before constructing the permit.
pub trait ToolRunner: Send + Sync {
    /// Opaque authorized invocation type.
    type Authorized: Send;
    /// Tool backend error.
    type Error: HasKind + Send;

    /// Authorize an untrusted model call. No side effect may happen in this method.
    fn authorize(&self, token: &Token, call: ToolCall) -> impl Future<Output = Result<Self::Authorized, Self::Error>> + Send;
    /// Execute only the permit returned by `authorize`.
    fn execute(&self, permit: Self::Authorized, frame: FrameId) -> impl Future<Output = Result<ToolResultRef, Self::Error>> + Send;
}

/// Output presented to the independent step verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepTool {
    /// Untrusted private reasoning channel, isolated from user-visible text and authorization facts.
    pub reasoning: String,
    /// Provider terminal reason presented to independent verification.
    pub finish_reason: FinishReason,
    /// Untrusted model output text.
    pub text: String,
    /// Results from tool calls that passed authorization and execution.
    pub tool_results: Vec<ExecutedToolResult>,
}

/// Independent verification decision. Only supplied verified diff CIDs are added to history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    /// The step is acceptable; fingerprint comes from a real source/snapshot provider.
    Accepted {
        /// Source fingerprint supplied by trusted analysis.
        fingerprint: FastFingerprint,
        /// Content identifiers the verifier independently accepted.
        verified_diffs: Vec<Cid>,
    },
    /// The agent claims completion; this is not a terminal proof or frame transition.
    ClaimedDone,
    /// Verification rejected the step.
    Rejected,
}

/// Per-step verification seam; adapters must not turn missing analysis into acceptance.
pub trait StepVerifier: Send + Sync {
    /// Verifier failure.
    type Error: HasKind + Send;
    /// Assess generated text and tool outcomes independently.
    fn verify(&self, frame: &FrameView, output: &StepTool) -> impl Future<Output = Result<Verification, Self::Error>> + Send;
}

/// Step result. ClaimedDone deliberately leaves the frame active and creates no proof.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    /// The model returned an ordinary turn without tools.
    Continue {
        /// Accepted model text for the caller to append to context.
        text: String,
    },
    /// Authorized tools ran; caller may append results and request another step.
    ToolResults {
        /// Exact assistant turn, including ordered calls and isolated reasoning metadata.
        assistant_message: ChatMessage,
        /// Authorized results in call-index order, each linked to its original call ID.
        results: Vec<ExecutedToolResult>,
    },
    /// The verifier observed a claim, but completion is not established.
    ClaimedDone {
        /// Unverified completion claim from the model.
        claim: String,
    },
    /// Verification rejected the step or a non-actionable finish reason arrived.
    Aborted,
}

/// Cooperative cancellation signal. Cancellation is selected against every backend await.
pub type Cancellation = CancellationToken;

/// Bounded step input. Limits are checked at each boundary before model or tool invocation.
pub struct StepInput<'a> {
    /// Active frame that receives the resulting verified timeline entry.
    pub frame: &'a mut AgentContextFrame<Active>,
    /// User/agent goal for the assembly operator.
    pub goal: &'a str,
    /// Cancellation shared by the frame and stream owner.
    pub cancellation: &'a Cancellation,
    /// Maximum caller-supplied goal bytes.
    pub max_goal_bytes: usize,
    /// Maximum assembled messages and tool-schema bytes.
    pub max_prompt_bytes: usize,
    /// Maximum accumulated assistant and reasoning bytes.
    pub max_output_bytes: usize,
    /// Maximum number of tool calls in one model response.
    pub max_tool_calls: usize,
    /// Maximum argument bytes per tool call.
    pub max_tool_argument_bytes: usize,
    /// Maximum returned tool text bytes.
    pub max_tool_result_bytes: usize,
    /// Maximum model stream chunks, including empty/usage chunks.
    pub max_stream_chunks: usize,
}

/// Assemble, reserve prompt/output quota, stream, validate finish and usage, authorize tools,
/// execute permits, and independently verify one step. Full token reservations remain charged even
/// if provider usage is absent or below the reserved upper bound; cancellation never refunds them.
pub async fn run_step<A, M, T, V>(input: StepInput<'_>, assemble: &A, chat: &M, tools: &T, verify: &V) -> Result<StepOutcome, FrameError>
where
    A: FunctionalOperator<In = AssembleRequest, Out = FramePrompt>,
    M: ChatModel,
    T: ToolRunner,
    V: StepVerifier,
{
    let StepInput { frame, goal, cancellation, max_goal_bytes, max_prompt_bytes, max_output_bytes, max_tool_calls, max_tool_argument_bytes, max_tool_result_bytes, max_stream_chunks } = input;
    check_cancel(cancellation)?;
    if goal.len() > max_goal_bytes { return Err(FrameError::OutputLimit); }
    let step = frame.begin_step()?;
    let prompt = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(FrameError::Cancelled),
        result = assemble.apply(AssembleRequest { frame: frame.view(), goal: goal.to_owned() }) => result.map_err(|error| FrameError::Backend(error.kind(), "assembly"))?,
    };
    check_cancel(cancellation)?;
    let model = frame.model.clone().ok_or(FrameError::Accounting)?;
    let mut tool_effects = BTreeMap::new();
    let mut exposed_tools = Vec::new();
    for spec in prompt.tools {
        let name = spec.name.to_string();
        if tool_effects.contains_key(&name) { return Err(FrameError::RoleToolDenied); }
        if role_allows_tool(frame.role, spec.effects) {
            tool_effects.insert(name, spec.effects);
            exposed_tools.push(spec);
        }
    }
    let prompt_bytes = prompt.messages.iter().try_fold(0_usize, |total, message| total.checked_add(message.content.len()).ok_or(FrameError::OutputLimit))?;
    let prompt_bytes = exposed_tools.iter().try_fold(prompt_bytes, |total, spec| total.checked_add(spec.name.len()).and_then(|value| value.checked_add(spec.description.len())).and_then(|value| value.checked_add(spec.input_schema_json.len())).ok_or(FrameError::OutputLimit))?;
    if prompt_bytes > max_prompt_bytes { return Err(FrameError::OutputLimit); }
    let output_limit = frame.reserve_model_tokens(prompt.input_token_upper_bound, prompt.max_output_tokens)?;
    let request = ChatRequest { model, messages: prompt.messages, tools: exposed_tools, max_output_tokens: Some(output_limit), effort: frame.effort };
    let mut stream = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(FrameError::Cancelled),
        result = chat.stream(request) => result.map_err(FrameError::Model)?,
    };
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut calls: BTreeMap<u32, CallAssembly> = BTreeMap::new();
    let mut finish_reason: Option<FinishReason> = None;
    let mut usage = None;
    let mut stream_chunks = 0_usize;
    loop {
        let next = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(FrameError::Cancelled),
            chunk = stream.next_chunk() => chunk,
        };
        let Some(chunk) = next else { break; };
        stream_chunks = stream_chunks.checked_add(1).ok_or(FrameError::OutputLimit)?;
        if stream_chunks > max_stream_chunks { return Err(FrameError::OutputLimit); }
        let chunk = chunk.map_err(FrameError::Model)?;
        if usage.is_some() && !matches!(chunk, ChatChunk::Finished(_)) { return Err(FrameError::InvalidUsage); }
        match chunk {
            ChatChunk::Text(part) => {
                let new_len = text.len().checked_add(reasoning.len()).and_then(|len| len.checked_add(part.len())).ok_or(FrameError::OutputLimit)?;
                if new_len > max_output_bytes { return Err(FrameError::OutputLimit); }
                text.push_str(&part);
            }
            ChatChunk::ToolCallDelta { index, id, name, arguments_fragment } => {
                if usize::try_from(index).map_err(|_| FrameError::InvalidToolCall)? >= max_tool_calls { return Err(FrameError::OutputLimit); }
                let call = calls.entry(index).or_default();
                append_optional(&mut call.id, id, 256)?;
                append_optional(&mut call.name, name, 256)?;
                let argument_len = call.arguments.len().checked_add(arguments_fragment.len()).ok_or(FrameError::OutputLimit)?;
                if argument_len > max_tool_argument_bytes { return Err(FrameError::OutputLimit); }
                call.arguments.push_str(&arguments_fragment);
            }
            ChatChunk::Reasoning(part) => {
                let new_len = text.len().checked_add(reasoning.len()).and_then(|len| len.checked_add(part.len())).ok_or(FrameError::OutputLimit)?;
                if new_len > max_output_bytes { return Err(FrameError::OutputLimit); }
                reasoning.push_str(&part);
            }
            ChatChunk::Usage(report) => {
                if usage.replace(report).is_some() { return Err(FrameError::InvalidUsage); }
            }
            ChatChunk::Finished(reason) => { finish_reason = Some(reason); break; }
        }
    }
    let finish_reason = finish_reason.ok_or(FrameError::IncompleteStream)?;
    if let Some(report) = usage { frame.validate_reported_usage(prompt.input_token_upper_bound, output_limit, report)?; }
    if !matches!(finish_reason, FinishReason::Stop | FinishReason::ToolCalls) { return Ok(StepOutcome::Aborted); }

    let mut validated_calls = Vec::with_capacity(calls.len());
    let mut replay_calls = Vec::with_capacity(calls.len());
    let mut seen_call_ids = BTreeSet::new();
    for (_, call) in calls {
        let id = call.id.ok_or(FrameError::InvalidToolCall)?;
        let name = call.name.ok_or(FrameError::InvalidToolCall)?;
        if id.is_empty() || name.is_empty() || !seen_call_ids.insert(id.clone()) { return Err(FrameError::InvalidToolCall); }
        let tool_call = ToolCall { id: id.clone(), name: name.clone(), arguments: call.arguments };
        replay_calls.push(ChatToolCall { id, name, arguments_json: tool_call.arguments.clone() });
        let effects = tool_effects.get(&tool_call.name).ok_or(FrameError::RoleToolDenied)?;
        if !role_allows_tool(frame.role, *effects) { return Err(FrameError::RoleToolDenied); }
        validated_calls.push(tool_call);
    }

    // Authorize the complete call set before executing any member. This prevents a malformed or
    // disallowed later call from causing partial side effects from an earlier call.
    let mut permits = Vec::with_capacity(validated_calls.len());
    for call in validated_calls {
        check_cancel(cancellation)?;
        let call_id = call.id.clone();
        let permit = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(FrameError::Cancelled),
            result = tools.authorize(&frame.token, call) => result.map_err(|error| FrameError::Backend(error.kind(), "tool authorization"))?,
        };
        permits.push((call_id, permit));
    }

    let mut results = Vec::with_capacity(permits.len());
    for (call_id, permit) in permits {
        check_cancel(cancellation)?;
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(FrameError::Cancelled),
            result = tools.execute(permit, frame.id) => result.map_err(|error| FrameError::Backend(error.kind(), "tool execution"))?,
        };
        if result.content.len() > max_tool_result_bytes { return Err(FrameError::OutputLimit); }
        results.push(ExecutedToolResult { tool_call_id: call_id, result });
    }
    check_cancel(cancellation)?;
    let output = StepTool { finish_reason, reasoning, text, tool_results: results.clone() };
    let decision = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(FrameError::Cancelled),
        result = verify.verify(&frame.view(), &output) => result.map_err(|error| FrameError::Backend(error.kind(), "step verification"))?,
    };
    check_cancel(cancellation)?;
    match decision {
        Verification::Accepted { fingerprint, verified_diffs } => {
            frame.complete_step(step, fingerprint, verified_diffs);
            if results.is_empty() {
                Ok(StepOutcome::Continue { text: output.text })
            } else {
                let reasoning = if output.reasoning.is_empty() { None } else { Some(output.reasoning) };
                let assistant_message = ChatMessage::assistant_calls(output.text, replay_calls, reasoning);
                Ok(StepOutcome::ToolResults { assistant_message, results })
            }
        }
        Verification::ClaimedDone => Ok(StepOutcome::ClaimedDone { claim: output.text }),
        Verification::Rejected => Ok(StepOutcome::Aborted),
    }
}

#[derive(Default)]
struct CallAssembly { id: Option<String>, name: Option<String>, arguments: String }

fn append_optional(target: &mut Option<String>, part: Option<String>, maximum: usize) -> Result<(), FrameError> {
    if let Some(part) = part {
        match target {
            Some(existing) => {
                let len = existing.len().checked_add(part.len()).ok_or(FrameError::OutputLimit)?;
                if len > maximum { return Err(FrameError::OutputLimit); }
                existing.push_str(&part);
            }
            None => {
                if part.len() > maximum { return Err(FrameError::OutputLimit); }
                *target = Some(part);
            }
        }
    }
    Ok(())
}

fn role_allows_tool(role: AgentRole, effects: Effects) -> bool {
    match role {
        AgentRole::Verifier | AgentRole::Watchdog => !effects.writes_workspace && !effects.network && !effects.executes,
        AgentRole::Researcher => !effects.writes_workspace && !effects.executes,
        AgentRole::Hacker => effects == Effects::PURE,
        AgentRole::Orchestrator | AgentRole::SubAgent => true,
    }
}

fn check_cancel(cancellation: &Cancellation) -> Result<(), FrameError> {
    if cancellation.is_cancelled() { Err(FrameError::Cancelled) } else { Ok(()) }
}
