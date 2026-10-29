use rmcp::model::{CallToolResult, ContentBlock};

use crate::{
    error::McpError,
    model::{ToolContent, ToolOutcome},
};

/// Converts an rmcp tool response into the configured byte bound.
pub(crate) fn bounded_outcome(
    result: CallToolResult,
    max_bytes: usize,
) -> Result<ToolOutcome, McpError> {
    let mut content = Vec::new();
    let mut used_bytes = 0_usize;
    let mut truncated = false;
    for block in result.content {
        let remaining = max_bytes.saturating_sub(used_bytes);
        match block {
            ContentBlock::Text(text) => {
                if text.text.len() <= remaining {
                    used_bytes = used_bytes
                        .checked_add(text.text.len())
                        .ok_or(McpError::ResultTooLarge)?;
                    content.push(ToolContent::Text(text.text));
                } else {
                    content.push(ToolContent::Text(utf8_prefix(&text.text, remaining)));
                    truncated = true;
                    break;
                }
            }
            other => {
                let value = serde_json::to_value(other).map_err(|_| McpError::Transport)?;
                let serialized = serde_json::to_vec(&value).map_err(|_| McpError::Transport)?;
                if serialized.len() > remaining {
                    truncated = true;
                    break;
                }
                used_bytes = used_bytes
                    .checked_add(serialized.len())
                    .ok_or(McpError::ResultTooLarge)?;
                content.push(ToolContent::Structured(value));
            }
        }
    }
    if let Some(structured) = result.structured_content {
        let remaining = max_bytes.saturating_sub(used_bytes);
        let serialized = serde_json::to_vec(&structured).map_err(|_| McpError::Transport)?;
        if serialized.len() > remaining {
            truncated = true;
        } else {
            content.push(ToolContent::Structured(structured));
        }
    }
    let is_error = result.is_error == Some(true);
    Ok(ToolOutcome {
        content,
        is_error,
        truncated,
    })
}

fn utf8_prefix(value: &str, max_bytes: usize) -> String {
    let mut end = 0_usize;
    for (offset, character) in value.char_indices() {
        let next_end = match offset.checked_add(character.len_utf8()) {
            Some(value) => value,
            None => break,
        };
        if next_end > max_bytes {
            break;
        }
        end = next_end;
    }
    match value.get(..end) {
        Some(prefix) => prefix.to_owned(),
        None => String::new(),
    }
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-ext/crates/mcp/src/call.rs"]
mod tests;
