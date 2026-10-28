//! Tokenizer-independent validation and boundaries for complete conversation groups.

use std::collections::HashSet;

use polaris_sdk::registry::{ChatMessage, ChatRole};

use crate::ContextError;

/// Validated conversation history represented by the exclusive end of each complete group.
///
/// The message content remains owned by the caller; this type stores only group boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationWindow {
    group_ends: Vec<usize>,
}

impl ConversationWindow {
    /// Validate tool-call/result structure and record the exclusive end of each complete group.
    ///
    /// A normal message forms a one-message group. An assistant message with tool calls and all
    /// matching tool results forms one indivisible group, regardless of result order.
    ///
    /// # Errors
    /// Returns [`ContextError::ToolHistory`] when calls or results are orphaned, missing,
    /// duplicated, reused, mismatched, or attached to an invalid role.
    pub fn new(messages: &[ChatMessage]) -> Result<Self, ContextError> {
        let mut group_ends = Vec::new();
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

            if message.tool_calls.is_empty() {
                index = index.checked_add(1).ok_or(ContextError::EstimateOverflow)?;
                group_ends.push(index);
                continue;
            }

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
                index = index.checked_add(1).ok_or(ContextError::EstimateOverflow)?;
            }
            group_ends.push(index);
        }

        Ok(Self { group_ends })
    }

    /// Exclusive message indices ending each complete group.
    #[must_use]
    pub fn group_ends(&self) -> &[usize] {
        &self.group_ends
    }
}
