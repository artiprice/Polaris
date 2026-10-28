use std::{
    error::Error,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use polaris_runtime_context::{
    ByteHeuristic, ContextConfig, ContextError, ContextVirtualizer, ConversationWindow,
    archive_window, window_omission_notice,
};
use polaris_sdk::registry::{ChatMessage, ChatRole, ChatToolCall};
use polaris_storage_cas::{Cas, CasConfig};

type TestResult = Result<(), Box<dyn Error>>;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestStore {
    path: PathBuf,
    cas: Cas,
}

impl TestStore {
    fn new() -> Result<Self, Box<dyn Error>> {
        Self::with_limit(u64::MAX)
    }

    fn with_limit(max_blob_bytes: u64) -> Result<Self, Box<dyn Error>> {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "polaris-context-window-{}-{id}",
            std::process::id()
        ));
        let mut config = CasConfig::new(&path);
        config.max_blob_bytes = max_blob_bytes;
        let cas = Cas::open(config)?;
        Ok(Self { path, cas })
    }
}

impl Drop for TestStore {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.path));
    }
}

fn tool_history() -> Vec<ChatMessage> {
    let mut assistant = ChatMessage::text(ChatRole::Assistant, "checking".to_owned());
    assistant.tool_calls = vec![
        ChatToolCall {
            id: "call-a".to_owned(),
            name: "read_file".to_owned(),
            arguments_json: "{\"path\":\"a\"}".to_owned(),
        },
        ChatToolCall {
            id: "call-b".to_owned(),
            name: "read_file".to_owned(),
            arguments_json: "{\"path\":\"b\"}".to_owned(),
        },
    ];
    vec![
        ChatMessage::text(ChatRole::User, "inspect".to_owned()),
        assistant,
        ChatMessage::tool_result("call-b".to_owned(), "B".to_owned()),
        ChatMessage::tool_result("call-a".to_owned(), "A".to_owned()),
        ChatMessage::text(ChatRole::Assistant, "done".to_owned()),
    ]
}

#[test]
fn window_preserves_complete_tool_groups() -> TestResult {
    let messages = tool_history();
    let window = ConversationWindow::new(&messages)?;
    if window.group_ends() != [1, 4, 5] {
        return Err("tool call and both results were not kept in one complete group".into());
    }
    Ok(())
}

#[test]
fn window_rejects_malformed_history() -> TestResult {
    let orphan = [ChatMessage::tool_result(
        "missing".to_owned(),
        "x".to_owned(),
    )];
    if !matches!(
        ConversationWindow::new(&orphan),
        Err(ContextError::ToolHistory)
    ) {
        return Err("orphan tool result was accepted".into());
    }

    let mut assistant = ChatMessage::text(ChatRole::Assistant, String::new());
    assistant.tool_calls = vec![ChatToolCall {
        id: "call-a".to_owned(),
        name: "tool".to_owned(),
        arguments_json: "{}".to_owned(),
    }];
    let missing = [assistant.clone()];
    if !matches!(
        ConversationWindow::new(&missing),
        Err(ContextError::ToolHistory)
    ) {
        return Err("missing tool result was accepted".into());
    }

    let mut duplicate = assistant.clone();
    duplicate.tool_calls.push(ChatToolCall {
        id: "call-a".to_owned(),
        name: "tool".to_owned(),
        arguments_json: "{}".to_owned(),
    });
    let duplicate = [
        duplicate,
        ChatMessage::tool_result("call-a".to_owned(), "x".to_owned()),
    ];
    if !matches!(
        ConversationWindow::new(&duplicate),
        Err(ContextError::ToolHistory)
    ) {
        return Err("duplicate tool call id was accepted".into());
    }

    let duplicate_result = [
        ChatMessage {
            tool_calls: vec![
                ChatToolCall {
                    id: "call-a".to_owned(),
                    name: "tool".to_owned(),
                    arguments_json: "{}".to_owned(),
                },
                ChatToolCall {
                    id: "call-b".to_owned(),
                    name: "tool".to_owned(),
                    arguments_json: "{}".to_owned(),
                },
            ],
            ..ChatMessage::text(ChatRole::Assistant, String::new())
        },
        ChatMessage::tool_result("call-a".to_owned(), "first".to_owned()),
        ChatMessage::tool_result("call-a".to_owned(), "second".to_owned()),
    ];
    if !matches!(
        ConversationWindow::new(&duplicate_result),
        Err(ContextError::ToolHistory)
    ) {
        return Err("duplicate tool result was accepted".into());
    }

    let reused = [
        assistant.clone(),
        ChatMessage::tool_result("call-a".to_owned(), "x".to_owned()),
        assistant.clone(),
        ChatMessage::tool_result("call-a".to_owned(), "x".to_owned()),
    ];
    if !matches!(
        ConversationWindow::new(&reused),
        Err(ContextError::ToolHistory)
    ) {
        return Err("reused tool call id was accepted".into());
    }

    let mismatched = [
        assistant,
        ChatMessage::tool_result("other".to_owned(), "x".to_owned()),
    ];
    if !matches!(
        ConversationWindow::new(&mismatched),
        Err(ContextError::ToolHistory)
    ) {
        return Err("mismatched tool result was accepted".into());
    }

    let mut non_assistant = ChatMessage::text(ChatRole::User, String::new());
    non_assistant.tool_calls = vec![ChatToolCall {
        id: "call-a".to_owned(),
        name: "tool".to_owned(),
        arguments_json: "{}".to_owned(),
    }];
    if !matches!(
        ConversationWindow::new(&[non_assistant]),
        Err(ContextError::ToolHistory)
    ) {
        return Err("tool call attached to a user message was accepted".into());
    }
    Ok(())
}

#[test]
fn window_archive_roundtrip_is_bounded_and_verified() -> TestResult {
    let store = TestStore::new()?;
    let messages = tool_history();
    let archive = archive_window(&store.cas, &messages, 16 * 1024 * 1024)?;
    let notice = window_omission_notice(archive.reference);
    if !notice.contains(&archive.reference.to_marker()) {
        return Err("omission notice does not include its archive reference".into());
    }
    let config = ContextConfig {
        max_archived_window_bytes: 16 * 1024 * 1024,
        ..ContextConfig::default()
    };
    let virtualizer = ContextVirtualizer::new(store.cas.clone(), ByteHeuristic, config);
    let restored = virtualizer.expand_omitted_window(&archive)?;
    if restored != messages {
        return Err("verified archive expansion changed conversation messages".into());
    }
    if !matches!(
        archive_window(&store.cas, &messages, 1),
        Err(ContextError::WindowArchiveTooLarge)
    ) {
        return Err("archive exceeding caller byte cap was written".into());
    }
    let empty = [];
    if !matches!(
        archive_window(&store.cas, &empty, 1024),
        Err(ContextError::ToolHistory)
    ) {
        return Err("empty conversation archive was accepted".into());
    }
    let bounded_store = TestStore::with_limit(32)?;
    if !matches!(
        archive_window(&bounded_store.cas, &messages, 16 * 1024 * 1024),
        Err(ContextError::Store(
            polaris_storage_cas::CasError::TooLarge { .. }
        ))
    ) {
        return Err("backend blob limit was not enforced by archive write".into());
    }
    Ok(())
}
