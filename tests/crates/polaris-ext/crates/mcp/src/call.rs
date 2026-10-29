// Unit tests of crates/polaris-ext/crates/mcp/src/call.rs (`tests`); compiled into that module via #[path].
use rmcp::model::{CallToolResult, ContentBlock, TextContent};

use super::bounded_outcome;
use crate::model::{ToolContent, ToolOutcome};

#[test]
fn truncates_tool_text_at_utf8_boundary() {
    let mut result = CallToolResult::default();
    result.content = vec![ContentBlock::Text(TextContent::new("ééé"))];
    let bounded = bounded_outcome(result, 3);
    assert_eq!(
        bounded,
        Ok(ToolOutcome {
            content: vec![ToolContent::Text("é".to_owned())],
            is_error: false,
            truncated: true,
        })
    );
}

#[test]
fn result_byte_limit_applies_across_text_and_structured_content() {
    let mut result = CallToolResult::default();
    result.content = vec![ContentBlock::Text(TextContent::new("abcd"))];
    result.structured_content = Some(serde_json::json!({"payload": "too large"}));
    assert_eq!(
        bounded_outcome(result, 4),
        Ok(ToolOutcome {
            content: vec![ToolContent::Text("abcd".to_owned())],
            is_error: false,
            truncated: true,
        })
    );
}
