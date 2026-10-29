// Unit tests of crates/polaris-runtime/crates/context/src/assemble.rs (`lossless_window_tests`); compiled into that module via #[path].
use std::{error::Error, path::PathBuf};

use polaris_sdk::{
    ids::Cid,
    registry::{ChatMessage, ChatRole, ChatToolCall},
};
use polaris_storage_cas::{Cas, CasConfig};

use super::{
    AssembleRequest, ByteHeuristic, ContextConfig, ContextVirtualizer, WINDOW_ARCHIVE_VERSION,
    WindowArchive, WindowArchiveRef,
};
use crate::{EntryKind, EpistemicScratchpad};

type TestResult = Result<(), Box<dyn Error>>;

struct TestStore {
    path: PathBuf,
    cas: Option<Cas>,
}

impl Drop for TestStore {
    fn drop(&mut self) {
        drop(std::fs::remove_dir_all(&self.path));
    }
}

fn test_store() -> Result<TestStore, Box<dyn Error>> {
    let current_thread = std::thread::current();
    let thread_name = current_thread.name().ok_or("test thread has no name")?;
    let safe_thread_name = thread_name.replace(':', "_");
    let path = std::env::temp_dir().join(format!(
        "polaris-runtime-context-{}-{safe_thread_name}",
        std::process::id()
    ));
    std::fs::create_dir(&path)?;
    let cas = Cas::open(CasConfig::new(path.join("cas")))?;
    Ok(TestStore {
        path,
        cas: Some(cas),
    })
}

fn request<'a>(
    window: &'a [ChatMessage],
    scratchpad: &'a EpistemicScratchpad,
) -> AssembleRequest<'a> {
    AssembleRequest {
        system_prompt: "system",
        tools: &[],
        repo_map_version: "v1",
        repo_map: "",
        scratchpad,
        active_scope: "scope",
        window,
    }
}

fn rich_tool_history() -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            role: ChatRole::Assistant,
            content: "calling tool".to_owned(),
            tool_calls: vec![ChatToolCall {
                id: "call-1".to_owned(),
                name: "lookup".to_owned(),
                arguments_json: "{\"q\":\"first\"}".to_owned(),
            }],
            tool_call_id: None,
            reasoning: Some("private reasoning".to_owned()),
        },
        ChatMessage {
            role: ChatRole::Tool,
            content: "tool result".to_owned(),
            tool_calls: Vec::new(),
            tool_call_id: Some("call-1".to_owned()),
            reasoning: None,
        },
        ChatMessage::text(ChatRole::User, "newest turn with enough tokens".to_owned()),
    ]
}

fn bounded_config(max_context_tokens: u64) -> ContextConfig {
    ContextConfig {
        max_context_tokens,
        ..ContextConfig::default()
    }
}

fn archive_capped_config(
    max_context_tokens: u64,
    max_archived_window_bytes: usize,
) -> ContextConfig {
    ContextConfig {
        max_context_tokens,
        max_archived_window_bytes,
        ..ContextConfig::default()
    }
}

#[test]
fn archived_messages_expand_byte_exactly_and_preserve_tool_correlation() -> TestResult {
    let mut store = test_store()?;
    let cas = store.cas.take().ok_or("test CAS was already moved")?;
    let virtualizer = ContextVirtualizer::new(cas, ByteHeuristic, bounded_config(80));
    let scratchpad = EpistemicScratchpad::default();
    let history = rich_tool_history();
    let prompt = virtualizer.assemble(&request(&history, &scratchpad))?;
    let archived = prompt
        .omitted_window
        .as_ref()
        .ok_or("missing archive reference")?;
    let archived_bytes = virtualizer.expand(archived.reference, None)?;
    if archived.message_count != 2
        || archived.content_id != polaris_storage_cas::content_id(&archived_bytes)
    {
        return Err(format!(
            "archive metadata mismatch: message_count={}, id_matches={}",
            archived.message_count,
            archived.content_id == polaris_storage_cas::content_id(&archived_bytes)
        )
        .into());
    }
    let restored = virtualizer.expand_omitted_window(archived)?;
    if restored.as_slice() != history.get(..2).ok_or("missing archived fixture prefix")? {
        return Err("structured archive did not round trip".into());
    }
    if !prompt.omission_notice.as_deref().is_some_and(|notice| {
        notice.contains("UNTRUSTED") && notice.contains("polaris_expand") && notice.contains("JSON")
    }) {
        return Err("returned prompt omitted the correct untrusted JSON expansion notice".into());
    }
    Ok(())
}

#[test]
fn archive_reference_is_exposed_by_context_map() -> TestResult {
    let mut store = test_store()?;
    let cas = store.cas.take().ok_or("test CAS was already moved")?;
    let virtualizer = ContextVirtualizer::new(cas, ByteHeuristic, bounded_config(80));
    let scratchpad = EpistemicScratchpad::default();
    let history = rich_tool_history();
    let prompt = virtualizer.assemble(&request(&history, &scratchpad))?;
    let report = virtualizer.context_map(&prompt);
    if report.omitted_window != prompt.omitted_window {
        return Err("context map archive reference differs from the assembled prompt".into());
    }
    Ok(())
}

#[test]
fn tool_call_groups_are_archived_and_retained_atomically() -> TestResult {
    let mut store = test_store()?;
    let cas = store.cas.take().ok_or("test CAS was already moved")?;
    let virtualizer = ContextVirtualizer::new(cas, ByteHeuristic, bounded_config(80));
    let scratchpad = EpistemicScratchpad::default();
    let history = rich_tool_history();
    let prompt = virtualizer.assemble(&request(&history, &scratchpad))?;
    let archived = prompt
        .omitted_window
        .as_ref()
        .ok_or("tool group was not archived")?;
    let restored = virtualizer.expand_omitted_window(archived)?;
    if restored.len() != 2
        || restored.first() != history.first()
        || restored.get(1) != history.get(1)
    {
        return Err(format!(
                "tool call and correlated result were split or changed: count={}, first_matches={}, second_matches={}",
                restored.len(),
                restored.first() == history.first(),
                restored.get(1) == history.get(1)
            )
            .into());
    }
    if prompt.window.first() != history.get(2) {
        return Err("newest conversation message was not retained".into());
    }
    if !prompt.prefix.bytes.contains("system")
        || !prompt.scratchpad.is_empty()
        || prompt.active_scope != "scope"
    {
        return Err("prefix, scratchpad, or active-scope placement changed".into());
    }
    Ok(())
}

#[test]
fn prefix_scratchpad_and_active_scope_order_remains_stable() -> TestResult {
    let mut store = test_store()?;
    let cas = store.cas.take().ok_or("test CAS was already moved")?;
    let virtualizer = ContextVirtualizer::new(cas, ByteHeuristic, bounded_config(80));
    let mut scratchpad = EpistemicScratchpad::default();
    virtualizer.append_scratchpad(&mut scratchpad, 1, EntryKind::Invariant, "fixed invariant")?;
    let history = rich_tool_history();
    let prompt = virtualizer.assemble(&request(&history, &scratchpad))?;
    if !prompt.prefix.bytes.contains("system")
        || !prompt.scratchpad.contains("fixed invariant")
        || prompt.active_scope != "scope"
    {
        return Err("prefix, scratchpad, or active-scope placement changed".into());
    }
    Ok(())
}

#[test]
fn archive_byte_cap_fails_assembly_without_claiming_recovery() -> TestResult {
    let mut store = test_store()?;
    let cas = store.cas.take().ok_or("test CAS was already moved")?;
    let virtualizer = ContextVirtualizer::new(cas, ByteHeuristic, archive_capped_config(80, 1));
    let scratchpad = EpistemicScratchpad::default();
    let history = rich_tool_history();
    match virtualizer.assemble(&request(&history, &scratchpad)) {
        Err(crate::ContextError::WindowArchiveTooLarge) => Ok(()),
        Err(error) => Err(format!("unexpected assembly error: {error}").into()),
        Ok(_) => Err("assembly claimed success beyond the configured archive byte cap".into()),
    }
}

#[test]
fn expand_rejects_archives_larger_than_the_receiving_virtualizer_cap() -> TestResult {
    let mut store = test_store()?;
    let cas = store.cas.take().ok_or("test CAS was already moved")?;
    let writer = ContextVirtualizer::new(cas.clone(), ByteHeuristic, bounded_config(80));
    let scratchpad = EpistemicScratchpad::default();
    let history = rich_tool_history();
    let prompt = writer.assemble(&request(&history, &scratchpad))?;
    let archive = prompt.omitted_window.ok_or("missing archive reference")?;
    if archive.original_bytes <= 1 {
        return Err("archive fixture did not exceed the reader cap".into());
    }
    let bounded_reader = ContextVirtualizer::new(cas, ByteHeuristic, archive_capped_config(80, 1));
    if !matches!(
        bounded_reader.expand_omitted_window(&archive),
        Err(crate::ContextError::WindowArchiveTooLarge)
    ) {
        return Err("oversized archive metadata was not rejected before the CAS read".into());
    }
    let mut understated = archive;
    understated.original_bytes = 1;
    match bounded_reader.expand_omitted_window(&understated) {
        Err(crate::ContextError::Store(polaris_storage_cas::CasError::TooLarge { .. })) => Ok(()),
        Err(error) => Err(format!("unexpected bounded-read error: {error}").into()),
        Ok(_) => Err("understated archive metadata bypassed the bounded CAS read".into()),
    }
}

#[test]
fn archive_metadata_tampering_is_rejected_on_recovery() -> TestResult {
    let mut store = test_store()?;
    let cas = store.cas.take().ok_or("test CAS was already moved")?;
    let virtualizer = ContextVirtualizer::new(cas, ByteHeuristic, bounded_config(80));
    let scratchpad = EpistemicScratchpad::default();
    let history = rich_tool_history();
    let prompt = virtualizer.assemble(&request(&history, &scratchpad))?;
    let mut archive = prompt.omitted_window.ok_or("missing archive reference")?;
    archive.original_bytes = 0;
    match virtualizer.expand_omitted_window(&archive) {
        Err(crate::ContextError::InvalidWindowArchive) => Ok(()),
        Err(error) => Err(format!("unexpected archive error: {error}").into()),
        Ok(_) => Err("tampered archive metadata passed integrity validation".into()),
    }
}

#[test]
fn archive_cid_version_and_message_count_are_validated() -> TestResult {
    let mut store = test_store()?;
    let cas = store.cas.take().ok_or("test CAS was already moved")?;
    let virtualizer = ContextVirtualizer::new(cas, ByteHeuristic, bounded_config(80));
    let scratchpad = EpistemicScratchpad::default();
    let history = rich_tool_history();
    let prompt = virtualizer.assemble(&request(&history, &scratchpad))?;
    let archive = prompt.omitted_window.ok_or("missing archive reference")?;

    let mut bad_cid = archive;
    bad_cid.content_id = Cid::from_bytes([255; 32]);
    if !matches!(
        virtualizer.expand_omitted_window(&bad_cid),
        Err(crate::ContextError::WindowArchiveIntegrity)
    ) {
        return Err("archive CID mismatch was not rejected".into());
    }

    let mut bad_version = archive;
    bad_version.format_version = 2;
    if !matches!(
        virtualizer.expand_omitted_window(&bad_version),
        Err(crate::ContextError::InvalidWindowArchive)
    ) {
        return Err("unsupported archive version was not rejected".into());
    }

    let mut bad_count = archive;
    bad_count.message_count = 0;
    if !matches!(
        virtualizer.expand_omitted_window(&bad_count),
        Err(crate::ContextError::InvalidWindowArchive)
    ) {
        return Err("archive message-count mismatch was not rejected".into());
    }
    Ok(())
}

#[test]
fn malformed_archived_tool_group_is_rejected() -> TestResult {
    let mut store = test_store()?;
    let cas = store.cas.as_ref().ok_or("test CAS is missing")?;
    let messages = vec![ChatMessage::text(
        ChatRole::Tool,
        "orphan tool result".to_owned(),
    )];
    let bytes = serde_json::to_vec(&WindowArchive {
        format_version: WINDOW_ARCHIVE_VERSION,
        messages: &messages,
    })?;
    let cid = cas.put(&bytes)?;
    let archive = WindowArchiveRef {
        reference: cas.short_ref_checked(&cid)?,
        content_id: cid,
        format_version: WINDOW_ARCHIVE_VERSION,
        original_bytes: u64::try_from(bytes.len())?,
        message_count: 1,
    };
    let cas = store.cas.take().ok_or("test CAS was already moved")?;
    let virtualizer = ContextVirtualizer::new(cas, ByteHeuristic, bounded_config(80));
    if !matches!(
        virtualizer.expand_omitted_window(&archive),
        Err(crate::ContextError::ToolHistory)
    ) {
        return Err("malformed archived tool group was not rejected".into());
    }
    Ok(())
}

#[test]
fn identical_omitted_windows_produce_identical_archive_references() -> TestResult {
    let mut store = test_store()?;
    let cas = store.cas.take().ok_or("test CAS was already moved")?;
    let virtualizer = ContextVirtualizer::new(cas, ByteHeuristic, bounded_config(64));
    let scratchpad = EpistemicScratchpad::default();
    let history = rich_tool_history();
    let first = virtualizer.assemble(&request(&history, &scratchpad))?;
    let second = virtualizer.assemble(&request(&history, &scratchpad))?;
    if first.omitted_window != second.omitted_window
        || first.omission_notice != second.omission_notice
    {
        return Err("identical omitted histories produced different archive references".into());
    }
    Ok(())
}

#[test]
fn no_omission_emits_no_archive_marker() -> TestResult {
    let mut store = test_store()?;
    let cas = store.cas.take().ok_or("test CAS was already moved")?;
    let virtualizer = ContextVirtualizer::new(cas, ByteHeuristic, bounded_config(512));
    let scratchpad = EpistemicScratchpad::default();
    let history = rich_tool_history();
    let prompt = virtualizer.assemble(&request(&history, &scratchpad))?;
    if prompt.omitted_window.is_some()
        || prompt.omission_notice.is_some()
        || prompt.stats.omitted_messages != 0
    {
        return Err("uncompacted conversation emitted a false archive reference".into());
    }
    let expected_total = prompt
        .stats
        .prefix_tokens
        .checked_add(prompt.stats.scratchpad_tokens)
        .and_then(|sum| sum.checked_add(prompt.stats.active_scope_tokens))
        .and_then(|sum| sum.checked_add(prompt.stats.window_tokens))
        .ok_or("test token arithmetic overflowed")?;
    if prompt.stats.total_tokens != expected_total || prompt.stats.omission_notice_tokens != 0 {
        return Err("uncompacted prompt accounting included a nonexistent notice".into());
    }
    Ok(())
}

#[test]
fn archive_notice_is_included_in_the_context_budget() -> TestResult {
    let mut store = test_store()?;
    let cas = store.cas.take().ok_or("test CAS was already moved")?;
    let virtualizer = ContextVirtualizer::new(cas, ByteHeuristic, bounded_config(80));
    let scratchpad = EpistemicScratchpad::default();
    let history = rich_tool_history();
    let prompt = virtualizer.assemble(&request(&history, &scratchpad))?;
    if prompt.omitted_window.is_none() || prompt.omission_notice.is_none() {
        return Err("omitted messages lack their explicit prompt notice".into());
    }
    if prompt.stats.omission_notice_tokens == 0 || prompt.stats.total_tokens > 80 {
        return Err("archive notice was not included within the bounded prompt budget".into());
    }
    if virtualizer.context_map(&prompt).active_scope_tokens
        != prompt
            .stats
            .active_scope_tokens
            .saturating_add(prompt.stats.window_tokens)
            .saturating_add(prompt.stats.omission_notice_tokens)
    {
        return Err("context map omitted archive notice tokens".into());
    }
    Ok(())
}
