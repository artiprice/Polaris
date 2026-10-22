//! Prompt assembly, budget accounting, and the context virtualization facade.

use std::{
    collections::{HashSet, VecDeque},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use bytes::Bytes;
use polaris_sdk::{
    ids::ShortRef,
    registry::{ChatMessage, ChatRole, ToolSpec},
};
use polaris_storage_cas::Cas;

use crate::{
    ContextError,
    fold::{ByteRange, FoldConfig, FoldRequest, FoldState, Folded, expand},
    prefix::{PrefixHash, StablePrefix},
    scratchpad::{EpistemicScratchpad, ScratchpadConfig},
};

/// A token count with an explicit accuracy label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenEstimate {
    /// Estimated or measured token count.
    pub tokens: u64,
    /// True when this count is not an exact measurement of the provider's full chat template.
    pub is_estimate: bool,
}

/// Tokenizer seam bound by the composition root for the selected model.
pub trait TokenEstimator: Send + Sync {
    /// Count tokens for text without mutating it.
    fn estimate(&self, text: &str) -> Result<TokenEstimate, ContextError>;

    /// Count a complete structured chat message, including metadata and framing allowances.
    ///
    /// The default implementation counts each text field separately so tool arguments, call
    /// identity, and replayed reasoning cannot disappear from accounting. It adds fixed costs
    /// for role/framing fields and tool-call structure. Because provider chat templates differ,
    /// this result is always labeled estimated even if [`Self::estimate`] uses an exact text
    /// tokenizer.
    fn estimate_message(&self, message: &ChatMessage) -> Result<TokenEstimate, ContextError> {
        let role = match message.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
            ChatRole::Tool => "tool",
        };
        let mut tokens = self.estimate("polaris-chat-message-v1")?.tokens;
        let mut is_estimate = true;
        add_text_estimate(self, &mut tokens, &mut is_estimate, role)?;
        add_text_estimate(self, &mut tokens, &mut is_estimate, &message.content)?;
        if let Some(reasoning) = message.reasoning.as_deref() {
            add_text_estimate(self, &mut tokens, &mut is_estimate, reasoning)?;
            add_estimate(self, &mut tokens, &mut is_estimate, "reasoning field")?;
        }
        if let Some(call_id) = message.tool_call_id.as_deref() {
            add_text_estimate(self, &mut tokens, &mut is_estimate, call_id)?;
            add_estimate(self, &mut tokens, &mut is_estimate, "tool result id field")?;
        }
        for call in &message.tool_calls {
            add_text_estimate(self, &mut tokens, &mut is_estimate, &call.id)?;
            add_text_estimate(self, &mut tokens, &mut is_estimate, &call.name)?;
            add_text_estimate(self, &mut tokens, &mut is_estimate, &call.arguments_json)?;
            add_estimate(self, &mut tokens, &mut is_estimate, "tool call structure")?;
        }
        Ok(TokenEstimate {
            tokens,
            is_estimate,
        })
    }
}

fn add_text_estimate<K: TokenEstimator + ?Sized>(
    estimator: &K,
    total: &mut u64,
    is_estimate: &mut bool,
    text: &str,
) -> Result<(), ContextError> {
    let estimate = estimator.estimate(text)?;
    *total = total
        .checked_add(estimate.tokens)
        .ok_or(ContextError::EstimateOverflow)?;
    *is_estimate |= estimate.is_estimate;
    Ok(())
}

fn add_estimate<K: TokenEstimator + ?Sized>(
    estimator: &K,
    total: &mut u64,
    is_estimate: &mut bool,
    text: &str,
) -> Result<(), ContextError> {
    add_text_estimate(estimator, total, is_estimate, text)
}

/// Explicitly approximate byte-count/4 estimator for environments without a target tokenizer.
#[derive(Debug, Clone, Copy, Default)]
pub struct ByteHeuristic;

impl TokenEstimator for ByteHeuristic {
    fn estimate(&self, text: &str) -> Result<TokenEstimate, ContextError> {
        let bytes = u64::try_from(text.len()).map_err(|_| ContextError::EstimateOverflow)?;
        let tokens = bytes.checked_add(3).ok_or(ContextError::EstimateOverflow)? / 4;
        Ok(TokenEstimate {
            tokens,
            is_estimate: true,
        })
    }
}

/// Context policy for folding and total prompt budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextConfig {
    /// Tool-output folding policy.
    pub fold: FoldConfig,
    /// Scratchpad text overflow policy.
    pub scratchpad: ScratchpadConfig,
    /// Maximum total input tokens including prefix, scratchpad, active scope, and window.
    pub max_context_tokens: u64,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            fold: FoldConfig::default(),
            scratchpad: ScratchpadConfig::default(),
            max_context_tokens: 32_000,
        }
    }
}

/// All immutable and mutable prompt parts before provider dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssembledPrompt {
    /// Stable cached prefix.
    pub prefix: StablePrefix,
    /// Append-only epistemic state, mounted after the stable prefix.
    pub scratchpad: String,
    /// Active-scope text, mounted after scratchpad and before conversation history.
    pub active_scope: String,
    /// Oldest messages may be omitted to meet the context budget; remaining order is unchanged.
    pub window: Vec<ChatMessage>,
    /// Measured or estimated token bookkeeping.
    pub stats: AssemblyStats,
}

/// Per-assembly budget facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssemblyStats {
    /// Stable prefix tokens.
    pub prefix_tokens: u64,
    /// Scratchpad tokens.
    pub scratchpad_tokens: u64,
    /// Active-scope tokens.
    pub active_scope_tokens: u64,
    /// Retained conversation-window tokens.
    pub window_tokens: u64,
    /// Total estimated or measured input tokens.
    pub total_tokens: u64,
    /// Number of oldest conversation messages omitted to fit the total cap.
    pub omitted_messages: u64,
    /// True unless a whole-provider chat-template measurement is available. This API currently
    /// has no such measurement, so assembled prompt totals remain estimates.
    pub is_estimate: bool,
}

/// Snapshot backing `/context-map` with honest token-estimate state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextMapReport {
    /// Current assembled input token count.
    pub total_tokens: u64,
    /// Configured total context cap.
    pub window_cap: u64,
    /// Stable prefix count.
    pub prefix_tokens: u64,
    /// Scratchpad count.
    pub scratchpad_tokens: u64,
    /// Active-scope plus conversation-window count.
    pub active_scope_tokens: u64,
    /// Approximate tokens saved across completed fold calls.
    pub folded_saved_tokens: u64,
    /// True when totals are not measured using the provider's full chat template.
    pub is_estimate: bool,
}

/// Request data needed to assemble one prompt.
#[derive(Debug, Clone)]
pub struct AssembleRequest<'a> {
    /// System instructions included in the stable prefix.
    pub system_prompt: &'a str,
    /// Full or fixed-profile tool list; it is sorted by name for stable serialization.
    pub tools: &'a [ToolSpec],
    /// Version string for the repo map.
    pub repo_map_version: &'a str,
    /// Repo map bytes as UTF-8 text, generated by the caller.
    pub repo_map: &'a str,
    /// Epistemic state, rendered after the prefix.
    pub scratchpad: &'a EpistemicScratchpad,
    /// Active scope text.
    pub active_scope: &'a str,
    /// Conversation history, oldest first.
    pub window: &'a [ChatMessage],
}

/// Per-process prefix stability sample; this is a local signal, not provider-cache evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefixStability {
    /// Number of assemblies observed.
    pub requests: u64,
    /// Number whose hash equals the immediately previous assembly.
    pub consecutive_matches: u64,
}

/// CAS-backed context folding and prompt assembly.
///
/// This crate does not sanitize secrets or establish path containment. Its caller must pass data
/// through the configured privacy/scope pipeline before exposing it to an external model. Raw
/// outputs are stored byte-exactly in CAS, including any secrets present in the input.
pub struct ContextVirtualizer<C, K> {
    cas: C,
    estimator: K,
    config: ContextConfig,
    folds: FoldState,
    folded_saved_tokens: AtomicU64,
    prefix_previous: Mutex<Option<PrefixHash>>,
    prefix_stability: Mutex<PrefixStability>,
}

impl<K: TokenEstimator> ContextVirtualizer<Cas, K> {
    /// Construct using the concrete synchronous CAS backend and caller-selected token estimator.
    #[must_use]
    pub fn new(cas: Cas, estimator: K, config: ContextConfig) -> Self {
        Self {
            cas,
            estimator,
            config,
            folds: FoldState::default(),
            folded_saved_tokens: AtomicU64::new(0),
            prefix_previous: Mutex::new(None),
            prefix_stability: Mutex::new(PrefixStability {
                requests: 0,
                consecutive_matches: 0,
            }),
        }
    }

    /// Store/fold exact tool output. CAS is synchronous and may block on disk I/O.
    pub fn fold(&self, request: FoldRequest<'_>) -> Result<Folded, ContextError> {
        let folded = self.folds.fold(&self.cas, self.config.fold, request)?;
        let original = String::from_utf8_lossy(request.output);
        let before = self.estimator.estimate(&original)?.tokens;
        let after = self.estimator.estimate(&folded.view)?.tokens;
        let _updated =
            self.folded_saved_tokens
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                    Some(current.saturating_add(before.saturating_sub(after)))
                });
        Ok(folded)
    }

    /// Expand a short reference, validating ambiguity through the CAS resolver.
    pub fn expand(
        &self,
        reference: ShortRef,
        range: Option<ByteRange>,
    ) -> Result<Bytes, ContextError> {
        expand(&self.cas, reference, range)
    }

    /// Append an epistemic fact using this virtualizer's configured CAS overflow threshold.
    pub fn append_scratchpad(
        &self,
        scratchpad: &mut EpistemicScratchpad,
        turn: u32,
        kind: crate::EntryKind,
        text: &str,
    ) -> Result<(), ContextError> {
        let _entry = scratchpad.append(turn, kind, text, &self.cas, self.config.scratchpad)?;
        Ok(())
    }

    /// Assemble a prompt without modifying the stable prefix or scratchpad.
    pub fn assemble(&self, request: &AssembleRequest<'_>) -> Result<AssembledPrompt, ContextError> {
        let prefix = StablePrefix::new(
            request.system_prompt,
            request.tools,
            request.repo_map_version,
            request.repo_map,
        )?;
        self.observe_prefix(prefix.hash)?;
        let scratchpad = request.scratchpad.render();
        let active_scope = request.active_scope.to_owned();
        let prefix_estimate = self.estimator.estimate(&prefix.bytes)?;
        let scratchpad_estimate = self.estimator.estimate(&scratchpad)?;
        let active_estimate = self.estimator.estimate(&active_scope)?;
        let immutable = prefix_estimate
            .tokens
            .checked_add(scratchpad_estimate.tokens)
            .and_then(|sum| sum.checked_add(active_estimate.tokens))
            .ok_or(ContextError::EstimateOverflow)?;
        if immutable > self.config.max_context_tokens {
            return Err(ContextError::Budget);
        }

        let groups = validated_window_groups(request.window, &self.estimator)?;
        let mut retained_groups: VecDeque<WindowGroup> = groups.into();
        let mut window_tokens = retained_groups.iter().try_fold(0_u64, |sum, group| {
            sum.checked_add(group.tokens)
                .ok_or(ContextError::EstimateOverflow)
        })?;
        let mut omitted = 0_u64;
        while immutable
            .checked_add(window_tokens)
            .ok_or(ContextError::EstimateOverflow)?
            > self.config.max_context_tokens
        {
            let Some(removed_group) = retained_groups.pop_front() else {
                return Err(ContextError::Budget);
            };
            window_tokens = window_tokens
                .checked_sub(removed_group.tokens)
                .ok_or(ContextError::EstimateOverflow)?;
            let removed_messages = removed_group
                .end
                .checked_sub(removed_group.start)
                .ok_or(ContextError::EstimateOverflow)?;
            omitted = omitted
                .checked_add(
                    u64::try_from(removed_messages).map_err(|_| ContextError::EstimateOverflow)?,
                )
                .ok_or(ContextError::EstimateOverflow)?;
        }
        // Text-tokenizer precision does not account for provider roles, tool-call framing, or the
        // provider-specific chat template. Keep the public total honest until that seam exists.
        let is_estimate = true;
        let total_tokens = immutable
            .checked_add(window_tokens)
            .ok_or(ContextError::EstimateOverflow)?;
        let stats = AssemblyStats {
            prefix_tokens: prefix_estimate.tokens,
            scratchpad_tokens: scratchpad_estimate.tokens,
            active_scope_tokens: active_estimate.tokens,
            window_tokens,
            total_tokens,
            omitted_messages: omitted,
            is_estimate,
        };
        let retained_start = retained_groups
            .front()
            .map_or(request.window.len(), |group| group.start);
        let retained = request
            .window
            .get(retained_start..)
            .ok_or(ContextError::ToolHistory)?;
        Ok(AssembledPrompt {
            prefix,
            scratchpad,
            active_scope,
            window: retained.to_vec(),
            stats,
        })
    }

    /// Current prefix stability sample.
    pub fn prefix_stability(&self) -> Result<PrefixStability, ContextError> {
        self.prefix_stability
            .lock()
            .map(|state| *state)
            .map_err(|_| ContextError::StatePoisoned)
    }

    /// Build the context-map accounting view for the latest assembly.
    #[must_use]
    pub fn context_map(&self, prompt: &AssembledPrompt) -> ContextMapReport {
        ContextMapReport {
            total_tokens: prompt.stats.total_tokens,
            window_cap: self.config.max_context_tokens,
            prefix_tokens: prompt.stats.prefix_tokens,
            scratchpad_tokens: prompt.stats.scratchpad_tokens,
            active_scope_tokens: prompt
                .stats
                .active_scope_tokens
                .saturating_add(prompt.stats.window_tokens),
            folded_saved_tokens: self.folded_saved_tokens.load(Ordering::Relaxed),
            is_estimate: prompt.stats.is_estimate,
        }
    }

    fn observe_prefix(&self, hash: PrefixHash) -> Result<(), ContextError> {
        let mut previous = self
            .prefix_previous
            .lock()
            .map_err(|_| ContextError::StatePoisoned)?;
        let mut stats = self
            .prefix_stability
            .lock()
            .map_err(|_| ContextError::StatePoisoned)?;
        stats.requests = stats
            .requests
            .checked_add(1)
            .ok_or(ContextError::EstimateOverflow)?;
        if previous.as_ref().is_some_and(|old| *old == hash) {
            stats.consecutive_matches = stats
                .consecutive_matches
                .checked_add(1)
                .ok_or(ContextError::EstimateOverflow)?;
        }
        *previous = Some(hash);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct WindowGroup {
    start: usize,
    end: usize,
    tokens: u64,
}

fn validated_window_groups<K: TokenEstimator>(
    messages: &[ChatMessage],
    estimator: &K,
) -> Result<Vec<WindowGroup>, ContextError> {
    let mut groups = Vec::new();
    let mut seen_call_ids = HashSet::new();
    let mut index = 0_usize;
    while index < messages.len() {
        let message = messages.get(index).ok_or(ContextError::ToolHistory)?;
        if message.role == ChatRole::Tool || message.tool_call_id.is_some() {
            return Err(ContextError::ToolHistory);
        }
        if message.role != ChatRole::Assistant && !message.tool_calls.is_empty() {
            return Err(ContextError::ToolHistory);
        }
        let start = index;
        let mut group_tokens = estimator.estimate_message(message)?.tokens;
        if !message.tool_calls.is_empty() {
            let mut pending = HashSet::new();
            for call in &message.tool_calls {
                if call.id.trim().is_empty()
                    || call.name.trim().is_empty()
                    || !seen_call_ids.insert(call.id.clone())
                    || !pending.insert(call.id.clone())
                {
                    return Err(ContextError::ToolHistory);
                }
            }
            index = index.checked_add(1).ok_or(ContextError::EstimateOverflow)?;
            while !pending.is_empty() {
                let result = messages.get(index).ok_or(ContextError::ToolHistory)?;
                if result.role != ChatRole::Tool || !result.tool_calls.is_empty() {
                    return Err(ContextError::ToolHistory);
                }
                let result_id = result
                    .tool_call_id
                    .as_deref()
                    .filter(|id| !id.trim().is_empty())
                    .ok_or(ContextError::ToolHistory)?;
                if !pending.remove(result_id) {
                    return Err(ContextError::ToolHistory);
                }
                group_tokens = group_tokens
                    .checked_add(estimator.estimate_message(result)?.tokens)
                    .ok_or(ContextError::EstimateOverflow)?;
                index = index.checked_add(1).ok_or(ContextError::EstimateOverflow)?;
            }
        } else {
            index = index.checked_add(1).ok_or(ContextError::EstimateOverflow)?;
        }
        groups.push(WindowGroup {
            start,
            end: index,
            tokens: group_tokens,
        });
    }
    Ok(groups)
}
