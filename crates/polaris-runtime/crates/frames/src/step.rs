//! One bounded model step with cancellation, tool authorization, and independent verification.
use crate::{Active, AgentContextFrame, FrameError, FrameView};
use crate::{FrameJournal, FrameStepRecord};
use polaris_sdk::{
    error::HasKind,
    ids::{AgentRole, Cid, FastFingerprint, FrameId, TokenCount},
    registry::{
        ChatChunk, ChatChunkSource, ChatMessage, ChatModel, ChatRequest, ChatToolCall, Effects,
        FinishReason, FunctionalOperator, ToolSpec, Usage,
    },
};
use polaris_security_capability::Token;
use polaris_system_one_watchdog::{
    FrameSnapshot, HealthVerdict, NegativeConstraint, ProposedStep, StepIndex,
    StepOutcome as WatchdogStepOutcome, StepRecord, ToolCallFacts,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    io::{self, Write},
};
use tokio_util::sync::CancellationToken;

/// Immutable input to the assembly operator.
#[derive(Debug, Clone)]
pub struct AssembleRequest {
    /// Secret-free frame state.
    pub frame: FrameView,
    /// The caller's current task text.
    pub goal: String,
    /// Exact prior model/tool transcript, in protocol order. The assembler places this after its
    /// current policy and stable system prefix before returning the next prompt.
    pub prior_messages: Vec<ChatMessage>,
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
    /// Actual tokenizer context budget supplied by the assembler's current model binding.
    pub context_token_budget: TokenCount,
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

/// One streamed provider tool call as preserved in the durable assistant response.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CapturedToolCall {
    /// Provider ordering index.
    pub index: u32,
    /// Call identifier if its delta has arrived.
    pub id: Option<String>,
    /// Tool name if its delta has arrived.
    pub name: Option<String>,
    /// Bounded argument JSON accumulated so far.
    pub arguments: String,
}

/// Bounded response state captured from the provider stream for audit and recovery.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AssistantResponseCapture {
    /// Model-visible text accumulated so far.
    pub text: String,
    /// Separate provider reasoning channel, retained as private transcript data.
    pub reasoning: String,
    /// Tool call deltas ordered by provider index; entries may be partial.
    pub tool_calls: Vec<CapturedToolCall>,
    /// Provider terminal reason, when a finish chunk arrived.
    pub finish_reason: Option<FinishReason>,
    /// Provider usage, when reported once before or after the finish chunk.
    pub usage: Option<Usage>,
    /// Whether a valid finish chunk was followed by bounded, clean end-of-stream.
    pub complete: bool,
}

/// A tool result returned by the real tool runner. `content` is model-visible data, not proof.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolResultRef {
    /// CAS reference to the persisted result, created by the tool runner.
    pub cid: Cid,
    /// CAS reference to the exact bounded text supplied to the model.
    pub content_cid: Cid,
    /// Bounded model-visible result text.
    pub content: String,
}

/// A tool result linked to the call identifier issued by the model. The frame creates this
/// binding from the streamed assistant call; tool backends cannot choose or replace the ID.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

    /// Return trusted, metadata-only advisory facts from this runner's actual tool binding.
    /// Implementations must not derive effects or authority from model-supplied JSON. The
    /// default deliberately withholds facts for runners that cannot provide them.
    fn advisory_facts(&self, _call: &ToolCall) -> Option<ToolCallFacts> {
        None
    }

    /// Authorize an untrusted model call. No side effect may happen in this method.
    fn authorize(
        &self,
        token: &Token,
        call: ToolCall,
    ) -> impl Future<Output = Result<Self::Authorized, Self::Error>> + Send;
    /// Authorize using the actual driver-reserved origin. Read-only adapters may use ordinary
    /// authorization; context-sensitive adapters validate the sealed binding before admission.
    fn authorize_at_step(
        &self,
        token: &Token,
        _origin: &crate::FrameStepBinding,
        call: ToolCall,
    ) -> impl Future<Output = Result<Self::Authorized, Self::Error>> + Send {
        self.authorize(token, call)
    }

    /// Execute only the permit returned by `authorize`.
    /// Implementations must stop collecting model-visible content at `max_result_bytes` and
    /// return a typed error when the result exceeds that bound. The frame checks returned content
    /// again before it is verified or replayed.
    fn execute(
        &self,
        permit: Self::Authorized,
        frame: FrameId,
        max_result_bytes: usize,
    ) -> impl Future<Output = Result<ToolResultRef, Self::Error>> + Send;
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
    fn verify(
        &self,
        frame: &FrameView,
        output: &StepTool,
    ) -> impl Future<Output = Result<Verification, Self::Error>> + Send;

    /// Observe a fully validated proposal before any tool authorization or execution.
    /// Returned findings are advisory; the runtime always proceeds through the ordinary
    /// role, capability, lease, and verifier checks. Default implementations disable watchdogs.
    fn inspect_proposal(
        &self,
        _snapshot: &FrameSnapshot,
        _step: &ProposedStep,
    ) -> impl Future<Output = Option<WatchdogObservation>> + Send {
        async { None }
    }
}

/// Hashed step record and advisory watchdog result produced before tool admission.
#[derive(Debug, Clone)]
pub struct WatchdogObservation {
    /// Bounded hashes only; thought and tool arguments are not retained.
    pub record: StepRecord,
    /// Advisory status that never grants or denies capability authority.
    pub verdict: HealthVerdict,
}

/// Step result. ClaimedDone deliberately leaves the frame active and creates no proof.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// Prior assistant/tool transcript accumulated by `run_frame`.
    pub prior_messages: Vec<ChatMessage>,
    /// Captures the exact request before provider dispatch so interrupted attempts can be journaled.
    pub request_capture: &'a mut Option<ChatRequest>,
    /// Captures successfully executed and CID-validated results for interrupted-attempt journals.
    pub executed_tool_results_capture: &'a mut Vec<ExecutedToolResult>,
    /// Captures bounded provider response data, including partial streams, for the transcript.
    pub assistant_response_capture: &'a mut Option<AssistantResponseCapture>,
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

/// One completed step with the exact request sent to the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepExecution {
    /// Exact assembled request passed to the chat backend.
    pub request: ChatRequest,
    /// Verified result of that request.
    pub outcome: StepOutcome,
}

/// Assemble, reserve prompt/output quota, stream, validate finish and usage, authorize tools,
/// execute permits, and independently verify one step. Full token reservations remain charged even
/// if provider usage is absent or below the reserved upper bound; cancellation never refunds them.
pub async fn run_step<A, M, T, V>(
    input: StepInput<'_>,
    assemble: &A,
    chat: &M,
    tools: &T,
    verify: &V,
) -> Result<StepExecution, FrameError>
where
    A: FunctionalOperator<In = AssembleRequest, Out = FramePrompt>,
    M: ChatModel,
    T: ToolRunner,
    V: StepVerifier,
{
    let StepInput {
        frame,
        goal,
        prior_messages,
        request_capture,
        executed_tool_results_capture,
        assistant_response_capture,
        cancellation,
        max_goal_bytes,
        max_prompt_bytes,
        max_output_bytes,
        max_tool_calls,
        max_tool_argument_bytes,
        max_tool_result_bytes,
        max_stream_chunks,
    } = input;
    check_cancel(cancellation)?;
    if goal.len() > max_goal_bytes {
        return Err(FrameError::OutputLimit);
    }
    let step = frame.begin_step()?;
    let prompt = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(FrameError::Cancelled),
        result = assemble.apply(AssembleRequest { frame: frame.view(), goal: goal.to_owned(), prior_messages }) => result.map_err(|error| FrameError::Backend(error.kind(), "assembly"))?,
    };
    check_cancel(cancellation)?;
    let model = frame.model.clone().ok_or(FrameError::Accounting)?;
    let mut tool_effects = BTreeMap::new();
    let mut exposed_tools = Vec::new();
    for spec in prompt.tools {
        let name = spec.name.to_string();
        if tool_effects.contains_key(&name) {
            return Err(FrameError::RoleToolDenied);
        }
        if role_allows_tool(frame.role, spec.effects) {
            tool_effects.insert(name, spec.effects);
            exposed_tools.push(spec);
        }
    }
    validate_prompt_size(&prompt.messages, &exposed_tools, max_prompt_bytes)?;
    let output_limit =
        frame.reserve_model_tokens(prompt.input_token_upper_bound, prompt.max_output_tokens)?;
    let request = ChatRequest {
        model,
        messages: prompt.messages,
        tools: exposed_tools,
        max_output_tokens: Some(output_limit),
        effort: frame.effort,
    };
    *request_capture = Some(request.clone());
    let durable_request = request.clone();
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
    let stream_result: Result<(), FrameError> = async {
        loop {
            let next = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(FrameError::Cancelled),
                chunk = stream.next_chunk() => chunk,
            };
            let Some(chunk) = next else {
                break;
            };
            stream_chunks = stream_chunks
                .checked_add(1)
                .ok_or(FrameError::OutputLimit)?;
            if stream_chunks > max_stream_chunks {
                return Err(FrameError::OutputLimit);
            }
            let chunk = chunk.map_err(FrameError::Model)?;
            if finish_reason.is_some() && !matches!(chunk, ChatChunk::Usage(_)) {
                return Err(FrameError::InvalidStream);
            }
            match chunk {
                ChatChunk::Text(part) => {
                    let new_len = text
                        .len()
                        .checked_add(reasoning.len())
                        .and_then(|len| len.checked_add(part.len()))
                        .ok_or(FrameError::OutputLimit)?;
                    if new_len > max_output_bytes {
                        return Err(FrameError::OutputLimit);
                    }
                    text.push_str(&part);
                }
                ChatChunk::ToolCallDelta {
                    index,
                    id,
                    name,
                    arguments_fragment,
                } => {
                    if usize::try_from(index).map_err(|_| FrameError::InvalidToolCall)?
                        >= max_tool_calls
                    {
                        return Err(FrameError::OutputLimit);
                    }
                    let call = calls.entry(index).or_default();
                    append_optional(&mut call.id, id, 256)?;
                    append_optional(&mut call.name, name, 256)?;
                    let argument_len = call
                        .arguments
                        .len()
                        .checked_add(arguments_fragment.len())
                        .ok_or(FrameError::OutputLimit)?;
                    if argument_len > max_tool_argument_bytes {
                        return Err(FrameError::OutputLimit);
                    }
                    call.arguments.push_str(&arguments_fragment);
                }
                ChatChunk::Reasoning(part) => {
                    let new_len = text
                        .len()
                        .checked_add(reasoning.len())
                        .and_then(|len| len.checked_add(part.len()))
                        .ok_or(FrameError::OutputLimit)?;
                    if new_len > max_output_bytes {
                        return Err(FrameError::OutputLimit);
                    }
                    reasoning.push_str(&part);
                }
                ChatChunk::Usage(report) => {
                    if usage.replace(report).is_some() {
                        return Err(FrameError::InvalidUsage);
                    }
                }
                ChatChunk::Finished(reason) => {
                    if finish_reason.replace(reason).is_some() {
                        return Err(FrameError::InvalidStream);
                    }
                }
            }
        }
        Ok(())
    }
    .await;
    let stream_complete = stream_result.is_ok() && finish_reason.is_some();
    *assistant_response_capture = Some(AssistantResponseCapture {
        text: text.clone(),
        reasoning: reasoning.clone(),
        tool_calls: calls
            .iter()
            .map(|(index, call)| CapturedToolCall {
                index: *index,
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            })
            .collect(),
        finish_reason,
        usage,
        complete: stream_complete,
    });
    stream_result?;
    let finish_reason = finish_reason.ok_or(FrameError::IncompleteStream)?;
    if let Some(report) = usage {
        frame.validate_reported_usage(prompt.input_token_upper_bound, output_limit, report)?;
    }
    if !matches!(finish_reason, FinishReason::Stop | FinishReason::ToolCalls) {
        return Ok(StepExecution {
            request: durable_request,
            outcome: StepOutcome::Aborted,
        });
    }
    if (finish_reason == FinishReason::ToolCalls) != !calls.is_empty() {
        return Err(FrameError::InvalidToolCall);
    }

    let mut validated_calls = Vec::with_capacity(calls.len());
    let mut replay_calls = Vec::with_capacity(calls.len());
    let mut seen_call_ids = BTreeSet::new();
    for (_, call) in calls {
        let id = call.id.ok_or(FrameError::InvalidToolCall)?;
        let name = call.name.ok_or(FrameError::InvalidToolCall)?;
        if id.is_empty() || name.is_empty() || !seen_call_ids.insert(id.clone()) {
            return Err(FrameError::InvalidToolCall);
        }
        let tool_call = ToolCall {
            id: id.clone(),
            name: name.clone(),
            arguments: call.arguments,
        };
        replay_calls.push(ChatToolCall {
            id,
            name,
            arguments_json: tool_call.arguments.clone(),
        });
        let effects = tool_effects
            .get(&tool_call.name)
            .ok_or(FrameError::RoleToolDenied)?;
        if !role_allows_tool(frame.role, *effects) {
            return Err(FrameError::RoleToolDenied);
        }
        validated_calls.push(tool_call);
    }

    let advisory_facts = if validated_calls.is_empty() {
        Some(Vec::new())
    } else {
        validated_calls
            .iter()
            .map(|call| tools.advisory_facts(call))
            .collect::<Option<Vec<_>>>()
    };
    let advisory_has_write = advisory_facts
        .as_ref()
        .is_some_and(|facts| facts.iter().any(|facts| facts.writes_workspace));
    let mut watchdog_observation = if let Some(tool_calls) = advisory_facts {
        let view = frame.view();
        let snapshot = FrameSnapshot {
            frame: view.id,
            current_step: step.get(),
            role: view.role,
            root_goal: goal.to_owned(),
            recent: view.watchdog_recent,
            negative_constraints: view.negative_constraints,
            context_tokens: prompt.input_token_upper_bound.get(),
            context_budget: prompt.context_token_budget.get(),
        };
        let proposal = ProposedStep {
            thought: text.clone(),
            tool_calls,
        };
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(FrameError::Cancelled),
            observation = verify.inspect_proposal(&snapshot, &proposal) => observation,
        }
    } else {
        None
    };
    if let Some(observation) = watchdog_observation.as_ref() {
        frame.record_watchdog_step(observation.record.clone())?;
    }

    // Authorize the complete call set before executing any member. This prevents a malformed or
    // disallowed later call from causing partial side effects from an earlier call.
    let mut permits = Vec::with_capacity(validated_calls.len());
    for call in validated_calls {
        check_cancel(cancellation)?;
        let call_id = call.id.clone();
        let origin = frame.step_binding(max_tool_result_bytes)?;
        let permit = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(FrameError::Cancelled),
            result = tools.authorize_at_step(&frame.token, &origin, call) => result.map_err(|error| FrameError::Backend(error.kind(), "tool authorization"))?,
        };
        permits.push((call_id, permit));
    }

    let mut results = Vec::with_capacity(permits.len());
    for (call_id, permit) in permits {
        check_cancel(cancellation)?;
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(FrameError::Cancelled),
            result = tools.execute(permit, frame.id, max_tool_result_bytes) => result.map_err(|error| FrameError::Backend(error.kind(), "tool execution"))?,
        };
        if result.content.len() > max_tool_result_bytes {
            return Err(FrameError::OutputLimit);
        }
        let content_digest = blake3::hash(result.content.as_bytes());
        if content_digest.as_bytes() != result.content_cid.as_bytes() {
            return Err(FrameError::ToolResultReferenceMismatch);
        }
        executed_tool_results_capture.push(ExecutedToolResult {
            tool_call_id: call_id.clone(),
            result: result.clone(),
        });
        results.push(ExecutedToolResult {
            tool_call_id: call_id,
            result,
        });
    }
    check_cancel(cancellation)?;
    let output = StepTool {
        finish_reason,
        reasoning,
        text,
        tool_results: results.clone(),
    };
    let verification_frame = frame.view();
    let decision = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(FrameError::Cancelled),
        result = verify.verify(&verification_frame, &output) => result.map_err(|error| FrameError::Backend(error.kind(), "step verification"))?,
    };
    check_cancel(cancellation)?;
    if let Some(observation) = watchdog_observation.as_mut() {
        let next_outcome = match &decision {
            Verification::Accepted { .. } | Verification::ClaimedDone => {
                WatchdogStepOutcome::Completed
            }
            Verification::Rejected if advisory_has_write => WatchdogStepOutcome::RejectedPatch {
                hypothesis: observation.record.fingerprint,
            },
            Verification::Rejected => WatchdogStepOutcome::Failed,
        };
        observation.record.outcome = next_outcome;
        frame.record_watchdog_step(observation.record.clone())?;
        if matches!(next_outcome, WatchdogStepOutcome::RejectedPatch { .. }) {
            frame.record_negative_constraint(NegativeConstraint {
                hypothesis: observation.record.fingerprint,
                rejected_at: StepIndex::new(step.get()),
            })?;
        }
    }
    match decision {
        Verification::Accepted {
            fingerprint,
            verified_diffs,
        } => {
            frame.complete_step(step, fingerprint, verified_diffs);
            if results.is_empty() {
                Ok(StepExecution {
                    request: durable_request,
                    outcome: StepOutcome::Continue { text: output.text },
                })
            } else {
                let reasoning = if output.reasoning.is_empty() {
                    None
                } else {
                    Some(output.reasoning)
                };
                let assistant_message =
                    ChatMessage::assistant_calls(output.text, replay_calls, reasoning);
                Ok(StepExecution {
                    request: durable_request,
                    outcome: StepOutcome::ToolResults {
                        assistant_message,
                        results,
                    },
                })
            }
        }
        Verification::ClaimedDone => Ok(StepExecution {
            request: durable_request,
            outcome: StepOutcome::ClaimedDone { claim: output.text },
        }),
        Verification::Rejected => Ok(StepExecution {
            request: durable_request,
            outcome: StepOutcome::Aborted,
        }),
    }
}

fn validate_prompt_size(
    messages: &[ChatMessage],
    tools: &[ToolSpec],
    maximum_bytes: usize,
) -> Result<(), FrameError> {
    let mut counter = BoundedByteCounter {
        bytes: 0,
        maximum_bytes,
        exceeded: false,
    };
    let serialization = serde_json::to_writer(&mut counter, &(messages, tools));
    if counter.exceeded {
        return Err(FrameError::OutputLimit);
    }
    serialization.map_err(|_| FrameError::Accounting)
}

struct BoundedByteCounter {
    bytes: usize,
    maximum_bytes: usize,
    exceeded: bool,
}

impl Write for BoundedByteCounter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let Some(next_bytes) = self.bytes.checked_add(buffer.len()) else {
            self.exceeded = true;
            return Err(io::Error::other("serialized prompt size overflow"));
        };
        if next_bytes > self.maximum_bytes {
            self.exceeded = true;
            return Err(io::Error::other(
                "serialized prompt exceeds configured bound",
            ));
        }
        self.bytes = next_bytes;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Hard bounds and byte limits used by every attempt in a frame run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRunLimits {
    /// Maximum model/tool/verifier attempts in this invocation.
    pub maximum_attempts: u32,
    /// Maximum retries after verifier rejection.
    pub maximum_retries: u32,
    /// Maximum tool-bearing turns before stopping.
    pub maximum_tool_rounds: u32,
    /// Maximum caller-supplied goal bytes.
    pub max_goal_bytes: usize,
    /// Maximum assembled messages and tool-schema bytes per attempt.
    pub max_prompt_bytes: usize,
    /// Maximum accumulated assistant and reasoning bytes per attempt.
    pub max_output_bytes: usize,
    /// Maximum number of streamed tool calls per attempt.
    pub max_tool_calls: usize,
    /// Maximum argument bytes per tool call.
    pub max_tool_argument_bytes: usize,
    /// Maximum returned tool text bytes.
    pub max_tool_result_bytes: usize,
    /// Maximum chunks per model stream, including empty/usage chunks.
    pub max_stream_chunks: usize,
}

/// Mutable frame state and run-level policy supplied to the bounded production driver.
#[derive(Debug)]
pub struct FrameRunInput<'a> {
    /// Active frame that owns capability, step and token quota ledgers.
    pub frame: &'a mut AgentContextFrame<Active>,
    /// Goal text supplied to every attempt in the run.
    pub goal: &'a str,
    /// Cancellation shared with all backend calls.
    pub cancellation: &'a Cancellation,
    /// Hard attempt, tool-loop and byte bounds.
    pub limits: FrameRunLimits,
}

/// Terminal reason for a bounded multi-step frame run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRunStop {
    /// The model returned an ordinary response without tool calls.
    ModelTurnComplete,
    /// The independent verifier observed a claim; proof-of-done remains external.
    ClaimedDone,
    /// The verifier rejected the final permitted attempt.
    Rejected,
    /// The configured retry allowance ended after rejection.
    RetryLimit,
    /// The configured total attempt allowance ended after a tool response.
    AttemptLimit,
    /// The configured tool-round allowance ended after a tool response.
    ToolRoundLimit,
}

/// Result and accounting for a completed bounded frame run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRunOutcome {
    /// Last verified step outcome.
    pub last_step: StepOutcome,
    /// Why no further model attempt was made.
    pub stop: FrameRunStop,
    /// Attempts reserved by the frame during this invocation.
    pub attempts: u32,
    /// Verifier rejections retried within the configured bound.
    pub retries: u32,
    /// Tool-bearing turns executed within the configured bound.
    pub tool_rounds: u32,
}

/// Run bounded model/tool/verifier turns until an ordinary response, completion claim, rejection,
/// retry limit, attempt limit, or tool-round limit. Tool results retain their durable CAS ids and
/// are replayed under the exact call ids supplied by the model. Every attempt goes through
/// `run_step`, so steps and full token reservations remain charged on failure and cancellation.
pub async fn run_frame<A, M, T, V, J>(
    input: FrameRunInput<'_>,
    assemble: &A,
    chat: &M,
    tools: &T,
    verify: &V,
    journal: &J,
) -> Result<FrameRunOutcome, FrameError>
where
    A: FunctionalOperator<In = AssembleRequest, Out = FramePrompt>,
    M: ChatModel,
    T: ToolRunner,
    V: StepVerifier,
    J: FrameJournal,
{
    let FrameRunInput {
        frame,
        goal,
        cancellation,
        limits,
    } = input;
    if limits.maximum_attempts == 0 {
        return Err(FrameError::StepLimit);
    }
    let mut prior_messages = Vec::new();
    let mut attempts = 0_u32;
    let mut retries = 0_u32;
    let mut tool_rounds = 0_u32;
    loop {
        if attempts >= limits.maximum_attempts {
            return Err(FrameError::StepLimit);
        }
        let starting_step = frame.steps();
        let mut request_capture = None;
        let mut executed_tool_results_capture = Vec::new();
        let mut assistant_response_capture = None;
        let execution = run_step(
            StepInput {
                frame,
                goal,
                prior_messages: prior_messages.clone(),
                request_capture: &mut request_capture,
                executed_tool_results_capture: &mut executed_tool_results_capture,
                assistant_response_capture: &mut assistant_response_capture,
                cancellation,
                max_goal_bytes: limits.max_goal_bytes,
                max_prompt_bytes: limits.max_prompt_bytes,
                max_output_bytes: limits.max_output_bytes,
                max_tool_calls: if limits.maximum_tool_rounds == 0 {
                    0
                } else {
                    limits.max_tool_calls
                },
                max_tool_argument_bytes: limits.max_tool_argument_bytes,
                max_tool_result_bytes: limits.max_tool_result_bytes,
                max_stream_chunks: limits.max_stream_chunks,
            },
            assemble,
            chat,
            tools,
            verify,
        )
        .await;
        let execution = match execution {
            Ok(execution) => execution,
            Err(error) => {
                if frame.steps() > starting_step {
                    let outcome = if matches!(&error, FrameError::Cancelled) {
                        polaris_sdk::contracts::FrameStepOutcome::Cancelled
                    } else {
                        polaris_sdk::contracts::FrameStepOutcome::Failed
                    };
                    let record = FrameStepRecord {
                        schema: crate::FRAME_STEP_RECORD_SCHEMA_VERSION,
                        frame: frame.view().id,
                        step: frame.steps(),
                        goal: goal.to_owned(),
                        request: request_capture,
                        assistant_response: assistant_response_capture,
                        result: None,
                        executed_tool_results: executed_tool_results_capture,
                        watchdog_memory: crate::WatchdogFrameMemory::from_view(&frame.view()),
                        outcome,
                    };
                    let _transcript = journal.record(&record).await.map_err(|journal_error| {
                        FrameError::Backend(journal_error.kind(), "frame journal")
                    })?;
                }
                return Err(error);
            }
        };
        attempts = attempts.checked_add(1).ok_or(FrameError::Accounting)?;
        let outcome = execution.outcome;
        let journal_outcome = match &outcome {
            StepOutcome::Continue { .. } | StepOutcome::ToolResults { .. } => {
                polaris_sdk::contracts::FrameStepOutcome::Accepted
            }
            StepOutcome::ClaimedDone { .. } => {
                polaris_sdk::contracts::FrameStepOutcome::ClaimedDone
            }
            StepOutcome::Aborted => polaris_sdk::contracts::FrameStepOutcome::Rejected,
        };
        let record = FrameStepRecord {
            schema: crate::FRAME_STEP_RECORD_SCHEMA_VERSION,
            frame: frame.view().id,
            step: frame.steps(),
            goal: goal.to_owned(),
            request: Some(execution.request),
            assistant_response: assistant_response_capture,
            result: Some(outcome.clone()),
            executed_tool_results: if matches!(&outcome, StepOutcome::ToolResults { .. }) {
                Vec::new()
            } else {
                executed_tool_results_capture
            },
            watchdog_memory: crate::WatchdogFrameMemory::from_view(&frame.view()),
            outcome: journal_outcome,
        };
        let _transcript = journal
            .record(&record)
            .await
            .map_err(|journal_error| FrameError::Backend(journal_error.kind(), "frame journal"))?;
        let stop = match &outcome {
            StepOutcome::Continue { text } => {
                prior_messages.push(ChatMessage::text(
                    polaris_sdk::registry::ChatRole::Assistant,
                    text.clone(),
                ));
                return Ok(FrameRunOutcome {
                    last_step: outcome,
                    stop: FrameRunStop::ModelTurnComplete,
                    attempts,
                    retries,
                    tool_rounds,
                });
            }
            StepOutcome::ToolResults {
                assistant_message,
                results,
            } => {
                prior_messages.push(assistant_message.clone());
                for entry in results {
                    prior_messages.push(ChatMessage::tool_result(
                        entry.tool_call_id.clone(),
                        entry.result.content.clone(),
                    ));
                }
                tool_rounds = tool_rounds.checked_add(1).ok_or(FrameError::Accounting)?;
                if tool_rounds >= limits.maximum_tool_rounds {
                    FrameRunStop::ToolRoundLimit
                } else if attempts >= limits.maximum_attempts {
                    FrameRunStop::AttemptLimit
                } else {
                    continue;
                }
            }
            StepOutcome::ClaimedDone { .. } => FrameRunStop::ClaimedDone,
            StepOutcome::Aborted => {
                if retries < limits.maximum_retries && attempts < limits.maximum_attempts {
                    retries = retries.checked_add(1).ok_or(FrameError::Accounting)?;
                    continue;
                }
                if retries >= limits.maximum_retries {
                    FrameRunStop::RetryLimit
                } else {
                    FrameRunStop::Rejected
                }
            }
        };
        return Ok(FrameRunOutcome {
            last_step: outcome,
            stop,
            attempts,
            retries,
            tool_rounds,
        });
    }
}

#[derive(Default)]
struct CallAssembly {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

fn append_optional(
    target: &mut Option<String>,
    part: Option<String>,
    maximum: usize,
) -> Result<(), FrameError> {
    if let Some(part) = part {
        match target {
            Some(existing) => {
                let len = existing
                    .len()
                    .checked_add(part.len())
                    .ok_or(FrameError::OutputLimit)?;
                if len > maximum {
                    return Err(FrameError::OutputLimit);
                }
                existing.push_str(&part);
            }
            None => {
                if part.len() > maximum {
                    return Err(FrameError::OutputLimit);
                }
                *target = Some(part);
            }
        }
    }
    Ok(())
}

/// Whether the trusted role policy permits a tool with these declared effects.
#[must_use]
pub fn role_allows_tool(role: AgentRole, effects: Effects) -> bool {
    match role {
        AgentRole::Verifier | AgentRole::Watchdog => {
            !effects.writes_workspace && !effects.network && !effects.executes
        }
        AgentRole::Researcher => !effects.writes_workspace && !effects.executes,
        AgentRole::Hacker => effects == Effects::PURE,
        AgentRole::Orchestrator | AgentRole::SubAgent => true,
    }
}

fn check_cancel(cancellation: &Cancellation) -> Result<(), FrameError> {
    if cancellation.is_cancelled() {
        Err(FrameError::Cancelled)
    } else {
        Ok(())
    }
}
