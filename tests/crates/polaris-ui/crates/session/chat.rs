//! Interactive chat turns on the session's own kernel with a scripted model.
use polaris_sdk::{
    ids::{ModelId, SessionId, TokenCount},
    registry::{
        ChatChunk, ChatChunkSource, ChatModel, ChatRequest, FinishReason, ModelError,
        PromptTokenCount, PromptTokenizer,
    },
};
use polaris_ui_session::{ChatAgent, ChatProgress, ChatSettings, OfflineSession};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct ScriptedModel {
    turns: Mutex<VecDeque<Vec<ChatChunk>>>,
    requests: Mutex<Vec<ChatRequest>>,
}
impl ScriptedModel {
    fn new(turns: Vec<Vec<ChatChunk>>) -> Self {
        Self {
            turns: Mutex::new(turns.into()),
            requests: Mutex::new(Vec::new()),
        }
    }
    fn requests(&self) -> Vec<ChatRequest> {
        self.requests
            .lock()
            .map(|requests| requests.clone())
            .unwrap_or_default()
    }
}
struct ScriptedStream(VecDeque<ChatChunk>);
impl ChatChunkSource for ScriptedStream {
    async fn next_chunk(&mut self) -> Option<Result<ChatChunk, ModelError>> {
        self.0.pop_front().map(Ok)
    }
}
impl ChatModel for ScriptedModel {
    type Stream = ScriptedStream;
    async fn stream(&self, req: ChatRequest) -> Result<Self::Stream, ModelError> {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(req);
        }
        let next = self
            .turns
            .lock()
            .ok()
            .and_then(|mut turns| turns.pop_front());
        next.map(|chunks| ScriptedStream(chunks.into()))
            .ok_or_else(|| ModelError::Protocol("script exhausted".to_owned()))
    }
}
impl PromptTokenizer for ScriptedModel {
    async fn count_prompt(&self, request: &ChatRequest) -> Result<PromptTokenCount, ModelError> {
        let bytes: usize = request.messages.iter().map(|m| m.content.len()).sum();
        Ok(PromptTokenCount {
            input_tokens: TokenCount::new(u32::try_from(bytes).unwrap_or(u32::MAX)),
            max_context_tokens: TokenCount::new(131_072),
        })
    }
}

fn text_turn(text: &str) -> Vec<ChatChunk> {
    vec![
        ChatChunk::Text(text.to_owned()),
        ChatChunk::Finished(FinishReason::Stop),
    ]
}

fn read_turn(path: &str) -> Vec<ChatChunk> {
    vec![
        ChatChunk::ToolCallDelta {
            index: 0,
            id: Some("call_1".to_owned()),
            name: Some("read_file".to_owned()),
            arguments_fragment: format!("{{\"path\":\"{path}\"}}"),
        },
        ChatChunk::Finished(FinishReason::ToolCalls),
    ]
}

fn drain(rx: &mut tokio::sync::mpsc::Receiver<ChatProgress>) -> String {
    let mut text = String::new();
    while let Ok(item) = rx.try_recv() {
        if let ChatProgress::Text(delta) = item {
            text.push_str(&delta);
        }
    }
    text
}

#[tokio::test]
async fn consecutive_turns_share_one_frame_and_carry_the_conversation() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    std::fs::write(workspace.path().join("notes.txt"), "the answer is 42")?;
    let mut session =
        OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let model = Arc::new(ScriptedModel::new(vec![
        read_turn("notes.txt"),
        text_turn("notes.txt says 42."),
        text_turn("Still 42."),
    ]));
    let mut agent = ChatAgent::new(
        session.kernel(),
        session.workspace(),
        ModelId::new("scripted-model")?,
        Arc::clone(&model),
        131_072,
        ChatSettings::default(),
    )
    .await?;
    let cancel = CancellationToken::new();

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let first = agent
        .turn(session.kernel(), "What is in notes.txt?", &cancel, tx)
        .await?;
    assert_eq!(first.answer.as_deref(), Some("notes.txt says 42."));
    assert_eq!(first.tool_rounds, 1);
    assert_eq!(drain(&mut rx), "notes.txt says 42.");

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let second = agent
        .turn(session.kernel(), "And now?", &cancel, tx)
        .await?;
    assert_eq!(second.answer.as_deref(), Some("Still 42."));
    assert_eq!(drain(&mut rx), "Still 42.");
    assert_eq!(agent.exchanges(), 2);

    let requests = model.requests();
    assert_eq!(requests.len(), 3);
    let tool_result_seen = requests[1]
        .messages
        .iter()
        .any(|message| message.content.contains("the answer is 42"));
    assert!(tool_result_seen, "the read tool result reaches the model");
    let carried = requests[2].messages.iter().any(|message| {
        message.content.contains("What is in notes.txt?")
            && message.content.contains("notes.txt says 42.")
    });
    assert!(carried, "the second turn carries the first exchange");
    session.close().await?;
    Ok(())
}

#[tokio::test]
async fn cancelled_turn_leaves_the_agent_usable() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let mut session =
        OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let model = Arc::new(ScriptedModel::new(vec![text_turn("after cancel")]));
    let mut agent = ChatAgent::new(
        session.kernel(),
        session.workspace(),
        ModelId::new("scripted-model")?,
        Arc::clone(&model),
        131_072,
        ChatSettings::default(),
    )
    .await?;
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let result = agent.turn(session.kernel(), "first", &cancelled, tx).await;
    assert!(result.is_err(), "a cancelled turn reports failure");
    let (tx, _rx) = tokio::sync::mpsc::channel(8);
    let next = agent
        .turn(session.kernel(), "second", &CancellationToken::new(), tx)
        .await?;
    assert_eq!(next.answer.as_deref(), Some("after cancel"));
    session.close().await?;
    Ok(())
}

#[tokio::test]
async fn switching_models_keeps_the_conversation_on_the_same_root() -> TestResult {
    use polaris_sdk::effort::Effort;
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let mut session =
        OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let first = Arc::new(ScriptedModel::new(vec![text_turn("from the first model")]));
    let second = Arc::new(ScriptedModel::new(vec![
        text_turn("from the second model"),
        text_turn("still the second model"),
    ]));
    let mut agent = ChatAgent::new(
        session.kernel(),
        session.workspace(),
        ModelId::new("first-model")?,
        Arc::clone(&first),
        131_072,
        ChatSettings::default(),
    )
    .await?;
    let cancel = CancellationToken::new();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    agent.turn(session.kernel(), "one", &cancel, tx).await?;
    agent.switch_model(
        ModelId::new("second-model")?,
        Arc::clone(&second),
        65_536,
        Effort::High,
    )?;
    assert_eq!(agent.model_id().as_str(), "second-model");
    assert_eq!(agent.effort(), Effort::High);
    for prompt in ["two", "three"] {
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let turn = agent.turn(session.kernel(), prompt, &cancel, tx).await?;
        assert!(
            turn.answer
                .is_some_and(|answer| answer.contains("second model"))
        );
    }
    let requests = second.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.content.contains("from the first model")),
        "the switched model sees the earlier exchange"
    );
    assert!(
        requests
            .iter()
            .all(|request| request.model.as_str() == "second-model")
    );
    session.close().await?;
    Ok(())
}

fn call_turn(id: &str, name: &str, arguments: &str) -> Vec<ChatChunk> {
    vec![
        ChatChunk::ToolCallDelta {
            index: 0,
            id: Some(id.to_owned()),
            name: Some(name.to_owned()),
            arguments_fragment: arguments.to_owned(),
        },
        ChatChunk::Finished(FinishReason::ToolCalls),
    ]
}

/// Answers every review with `apply` and `note`; returns the applied paths and closed reviews.
async fn review_all(
    kernel: polaris_kernel::KernelHandle,
    mut rx: tokio::sync::mpsc::Receiver<ChatProgress>,
    apply: bool,
    note: Option<&str>,
) -> Result<(Vec<String>, Vec<String>, usize), polaris_ui_session::ChatError> {
    let mut prompts = Vec::new();
    let mut applied = Vec::new();
    let mut closed = 0;
    while let Some(item) = rx.recv().await {
        match item {
            ChatProgress::Review(review) => {
                prompts.push(review.prompt.clone());
                polaris_ui_session::answer_review(&kernel, &review, apply, note.map(str::to_owned))
                    .await?;
            }
            ChatProgress::ReviewClosed(_) => closed += 1,
            ChatProgress::Applied(path) => applied.push(path),
            _ => {}
        }
    }
    Ok((prompts, applied, closed))
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_mode_change_is_reviewed_then_written() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    std::fs::write(workspace.path().join("notes.txt"), "status: draft\n")?;
    let session = OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let base = polaris_kernel::storage::cas::content_id(b"status: draft\n");
    let model = Arc::new(ScriptedModel::new(vec![
        call_turn(
            "edit-1",
            "edit_file",
            &format!(
                r#"{{"path":"notes.txt","old_text":"draft","new_text":"final","expected_base":"{base}"}}"#
            ),
        ),
        text_turn("Changed notes.txt."),
    ]));
    let mut agent = ChatAgent::new(
        session.kernel(),
        session.workspace(),
        ModelId::new("scripted-model")?,
        Arc::clone(&model),
        131_072,
        ChatSettings::default(),
    )
    .await?;
    let cancel = CancellationToken::new();
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let reviewer = tokio::spawn(review_all(session.kernel().clone(), rx, true, None));
    let turn = agent
        .turn(session.kernel(), "Mark the notes final.", &cancel, tx)
        .await?;
    let (prompts, applied, closed) = reviewer.await.map_err(|_| "reviewer panicked")??;
    assert_eq!(turn.answer.as_deref(), Some("Changed notes.txt."));
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("notes.txt"))?,
        "status: final\n"
    );
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0].starts_with("Change `notes.txt`?")
            && prompts[0].contains("-status: draft")
            && prompts[0].contains("+status: final"),
        "{}",
        prompts[0]
    );
    assert_eq!(applied, vec!["notes.txt".to_owned()]);
    assert_eq!(closed, 1);
    let first = model.requests().into_iter().next().ok_or("no request")?;
    assert!(
        first
            .tools
            .iter()
            .any(|tool| tool.name.as_ref() == "edit_file"),
        "auto mode offers the edit tools"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn plan_mode_offers_no_edit_tools() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let session = OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let model = Arc::new(ScriptedModel::new(vec![text_turn("1. Edit notes.txt.")]));
    let mut agent = ChatAgent::new(
        session.kernel(),
        session.workspace(),
        ModelId::new("scripted-model")?,
        Arc::clone(&model),
        131_072,
        ChatSettings::default(),
    )
    .await?;
    agent.set_plan_mode(true);
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let _turn = agent
        .turn(
            session.kernel(),
            "Plan the change.",
            &CancellationToken::new(),
            tx,
        )
        .await?;
    let first = model.requests().into_iter().next().ok_or("no request")?;
    assert!(
        !first.tools.iter().any(|tool| {
            matches!(
                tool.name.as_ref(),
                "edit_file" | "write_file" | "apply_patch" | "delete_file"
            )
        }),
        "plan mode is read-only"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn declined_change_is_not_written() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let session = OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let model = Arc::new(ScriptedModel::new(vec![
        call_turn(
            "write-1",
            "write_file",
            r#"{"path":"todo.md","contents":"- ship\n","expected_base":null}"#,
        ),
        text_turn("Left it."),
    ]));
    let mut agent = ChatAgent::new(
        session.kernel(),
        session.workspace(),
        ModelId::new("scripted-model")?,
        Arc::clone(&model),
        131_072,
        ChatSettings::default(),
    )
    .await?;
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    let reviewer = tokio::spawn(review_all(
        session.kernel().clone(),
        rx,
        false,
        Some("Not now."),
    ));
    let _turn = agent
        .turn(
            session.kernel(),
            "Add a todo file.",
            &CancellationToken::new(),
            tx,
        )
        .await?;
    let (prompts, applied, _closed) = reviewer.await.map_err(|_| "reviewer panicked")??;
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0].starts_with("Create `todo.md`?"),
        "{}",
        prompts[0]
    );
    assert!(applied.is_empty());
    assert!(!workspace.path().join("todo.md").exists());
    let last = model.requests().into_iter().last().ok_or("no request")?;
    assert!(
        last.messages
            .iter()
            .any(|message| message.content.contains("Not now.")),
        "the note reaches the model"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reviewed_deletion_is_reported_as_deleted() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    std::fs::write(workspace.path().join("old.md"), "# Old\n")?;
    let session = OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let base = polaris_kernel::storage::cas::content_id(b"# Old\n");
    let model = Arc::new(ScriptedModel::new(vec![
        call_turn(
            "delete-1",
            "delete_file",
            &format!(r#"{{"path":"old.md","expected_base":"{base}"}}"#),
        ),
        text_turn("Removed old.md."),
    ]));
    let mut agent = ChatAgent::new(
        session.kernel(),
        session.workspace(),
        ModelId::new("scripted-model")?,
        Arc::clone(&model),
        131_072,
        ChatSettings::default(),
    )
    .await?;
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let kernel = session.kernel().clone();
    let reviewer = tokio::spawn(async move {
        let mut deleted = Vec::new();
        while let Some(item) = rx.recv().await {
            match item {
                ChatProgress::Review(review) => {
                    polaris_ui_session::answer_review(&kernel, &review, true, None).await?;
                }
                ChatProgress::Deleted(path) => deleted.push(path),
                _ => {}
            }
        }
        Ok::<_, polaris_ui_session::ChatError>(deleted)
    });
    let turn = agent
        .turn(
            session.kernel(),
            "Delete old.md.",
            &CancellationToken::new(),
            tx,
        )
        .await?;
    let deleted = reviewer.await.map_err(|_| "reviewer panicked")??;
    assert_eq!(turn.answer.as_deref(), Some("Removed old.md."));
    assert_eq!(deleted, vec!["old.md".to_owned()]);
    assert!(!workspace.path().join("old.md").exists());
    Ok(())
}

/// P5: under `$nvidia(auto)` (the default) a turn in a CUDA workspace carries its NVIDIA profile
/// and advisor findings to the model; after `$nvidia(off)` it does not.
#[tokio::test]
async fn a_cuda_workspace_turn_carries_the_profile_until_nvidia_is_off() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    std::fs::write(
        workspace.path().join("kernel.cu"),
        "__global__ void zero(float* x) {\n    int i = blockIdx.x * blockDim.x + threadIdx.x;\n    x[i] = 0.0f;\n}\n",
    )?;
    let mut session =
        OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let model = Arc::new(ScriptedModel::new(vec![
        text_turn("Add a bounds check."),
        text_turn("Still needs a bounds check."),
    ]));
    let mut agent = ChatAgent::new(
        session.kernel(),
        session.workspace(),
        ModelId::new("scripted-model")?,
        Arc::clone(&model),
        131_072,
        ChatSettings::default(),
    )
    .await?;
    let cancel = CancellationToken::new();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    agent
        .turn(session.kernel(), "Is this kernel safe?", &cancel, tx)
        .await?;
    let _shown = drain(&mut rx);
    session.apply_directive("$nvidia(off)").await?;
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    agent
        .turn(session.kernel(), "And now?", &cancel, tx)
        .await?;
    let _shown = drain(&mut rx);

    let requests = model.requests();
    let carries = |request: &ChatRequest, needle: &str| {
        request
            .messages
            .iter()
            .any(|message| message.content.contains(needle))
    };
    assert_eq!(requests.len(), 2);
    assert!(carries(
        &requests[0],
        "NVIDIA profile of this workspace ($nvidia(auto))"
    ));
    assert!(carries(&requests[0], "- kernel.cu:2 [missing-grid-bounds]"));
    assert!(!carries(&requests[1], "NVIDIA profile of this workspace"));
    session.close().await?;
    Ok(())
}

/// P5 keyless `$rag`: with `$rag` on, a turn's request carries the best workspace passages with
/// their provenance; without it, none.
#[tokio::test]
async fn rag_carries_workspace_passages_into_the_request() -> TestResult {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    std::fs::write(
        workspace.path().join("billing.rs"),
        "/// Monthly invoice total with the regional tax rate applied.\npub fn invoice_total(amount: u64, tax_rate: u64) -> u64 {\n    amount + amount * tax_rate / 100\n}\n",
    )?;
    let mut session =
        OfflineSession::open(state.path(), workspace.path(), SessionId::new_v7()).await?;
    let model = Arc::new(ScriptedModel::new(vec![
        text_turn("It adds the tax."),
        text_turn("Still adds the tax."),
    ]));
    let mut agent = ChatAgent::new(
        session.kernel(),
        session.workspace(),
        ModelId::new("scripted-model")?,
        Arc::clone(&model),
        131_072,
        ChatSettings::default(),
    )
    .await?;
    let cancel = CancellationToken::new();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    agent
        .turn(
            session.kernel(),
            "How is the invoice total taxed?",
            &cancel,
            tx,
        )
        .await?;
    let _shown = drain(&mut rx);
    session.apply_directive("$rag").await?;
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    agent
        .turn(
            session.kernel(),
            "How is the invoice total taxed?",
            &cancel,
            tx,
        )
        .await?;
    let _shown = drain(&mut rx);
    let requests = model.requests();
    let carries = |request: &ChatRequest, needle: &str| {
        request
            .messages
            .iter()
            .any(|message| message.content.contains(needle))
    };
    assert_eq!(requests.len(), 2);
    assert!(!carries(
        &requests[0],
        "Workspace passages for this question"
    ));
    assert!(carries(
        &requests[1],
        "Workspace passages for this question ($rag"
    ));
    assert!(carries(&requests[1], "--- billing.rs:1-4"));
    session.close().await?;
    Ok(())
}
