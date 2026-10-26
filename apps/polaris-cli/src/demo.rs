//! Deterministic presentation-only runtime for the explicit demo mode.
use polaris_sdk::{effort::Effort, ids::ModelId};
use polaris_ui_model::{
    error::ViewError,
    questionnaire::{
        AnswerSubmitError, QuestionKind, QuestionOption, QuestionSpec, QuestionnaireBatch,
        QuestionnaireError, QuestionnaireSpec, QuestionnaireState,
    },
    view::{EntryKind, ViewState},
};
use std::{collections::VecDeque, time::Duration};
use thiserror::Error;

/// Maximum prompt size retained by the offline demo.
pub const MAX_DEMO_PROMPT_BYTES: usize = polaris_ui_model::composer::MAX_TEXT_BYTES;
/// Maximum accumulated scripted assistant output.
pub const MAX_DEMO_STREAM_BYTES: usize = 32 * 1024;

/// Errors produced by the local presentation script.
#[derive(Debug, Error)]
pub enum DemoError {
    /// The prompt is empty.
    #[error("demo prompt is empty")]
    EmptyPrompt,
    /// The prompt exceeds the demo's memory bound.
    #[error("demo prompt exceeds the composer byte limit")]
    PromptTooLarge,
    /// The incremental assistant entry exceeds its demo memory bound.
    #[error("demo assistant stream exceeds the 32 KiB limit")]
    StreamTooLarge,
    /// A submission cannot start while another demo flow is active.
    #[error("demo is busy or waiting for questionnaire answers")]
    Busy,
    /// Replay was requested before any prompt was submitted.
    #[error("there is no demo prompt to replay")]
    NoReplay,
    /// The demo questionnaire is not active.
    #[error("there is no demo questionnaire to accept")]
    NoQuestionnaire,
    /// The fixed demo model identity could not be constructed.
    #[error("failed to initialize the demo view")]
    ModelIdentity,
    /// The demo view rejected a bounded presentation entry.
    #[error(transparent)]
    View(#[from] ViewError),
    /// The fixed demo questionnaire failed validation.
    #[error(transparent)]
    Questionnaire(#[from] QuestionnaireError),
    /// The answer batch is incomplete or invalid; questionnaire state is retained.
    #[error(transparent)]
    Answer(#[from] AnswerSubmitError),
}

#[derive(Debug, Clone, Copy)]
enum DemoBeat {
    System(&'static str),
    AssistantChunk(&'static str),
    Error(&'static str),
    ToolStart,
    ToolComplete,
    Questionnaire,
}

#[derive(Debug, Clone, Copy)]
struct TimedBeat {
    due: Duration,
    beat: DemoBeat,
}

/// Local scripted conversation, simulated tool/diff/error states, and a real UI questionnaire draft.
#[derive(Debug)]
pub struct DemoRuntime {
    view: ViewState,
    beats: VecDeque<TimedBeat>,
    elapsed: Duration,
    last_prompt: Option<String>,
    question: Option<QuestionnaireState>,
    assistant_stream_open: bool,
    tools: Vec<polaris_ui_cli::screen::PresentationTool>,
    tools_expanded: bool,
}

impl DemoRuntime {
    /// Creates an empty demo view without opening a session or contacting the kernel.
    pub fn new() -> Result<Self, DemoError> {
        let model = ModelId::new("nvidia/llama-3.3-nemotron-super-49b-v1.5")
            .map_err(|_| DemoError::ModelIdentity)?;
        Ok(Self {
            view: ViewState::new(model, Effort::High),
            beats: VecDeque::new(),
            elapsed: Duration::ZERO,
            last_prompt: None,
            question: None,
            assistant_stream_open: false,
            tools: Vec::new(),
            tools_expanded: false,
        })
    }

    /// Borrows the demo conversation and status view.
    pub fn view(&self) -> &ViewState {
        &self.view
    }

    /// Mutably borrows the demo view for the renderer's local state updates.
    pub fn view_mut(&mut self) -> &mut ViewState {
        &mut self.view
    }

    /// Adds a user prompt and schedules the deterministic presentation script.
    ///
    /// # Errors
    /// Returns an error for an empty/oversized prompt, an active flow, or a view capacity limit.
    pub fn submit(&mut self, text: &str) -> Result<(), DemoError> {
        if !self.beats.is_empty() || self.question.is_some() {
            return Err(DemoError::Busy);
        }
        if text.trim().is_empty() {
            return Err(DemoError::EmptyPrompt);
        }
        if text.len() > MAX_DEMO_PROMPT_BYTES {
            return Err(DemoError::PromptTooLarge);
        }
        self.assistant_stream_open = false;
        self.view.push_user(format!("[DEMO] Prompt: {text}"))?;
        self.last_prompt = Some(text.to_owned());
        self.schedule_script();
        Ok(())
    }

    /// Advances the script using absolute monotonic time from the terminal's start.
    ///
    /// A backwards timestamp does not rewind the runtime. Beats are scheduled relative to the
    /// latest timestamp observed when the prompt was submitted.
    ///
    /// # Errors
    /// Returns an error if appending a presentation entry reaches the view's configured cap.
    pub fn tick(&mut self, elapsed: Duration) -> Result<bool, DemoError> {
        if elapsed > self.elapsed {
            self.elapsed = elapsed;
        }
        let mut changed = false;
        loop {
            let due = self
                .beats
                .front()
                .is_some_and(|beat| beat.due <= self.elapsed);
            if !due {
                break;
            }
            let Some(timed) = self.beats.front().copied() else {
                break;
            };
            match timed.beat {
                DemoBeat::System(text) => self.view.push_system(format!("[DEMO] {text}"))?,
                DemoBeat::AssistantChunk(text) => self.append_assistant_chunk(text)?,
                DemoBeat::Error(text) => self.view.push_error(format!("[DEMO] {text}"))?,
                DemoBeat::ToolStart => {
                    let index = self.view.conversation.len();
                    self.view.push_system("[DEMO] Tool: inspect_workspace\nTwo fixture matches: src/parser.rs and tests/parser.rs.\nNo files were read; this is scripted tool output.")?;
                    self.tools.push(polaris_ui_cli::screen::PresentationTool {
                        entry_index: index,
                        expanded: self.tools_expanded,
                        status: polaris_ui_cli::screen::PresentationToolStatus::Working,
                        duration_ms: None,
                    });
                }
                DemoBeat::ToolComplete => {
                    if let Some(tool) = self.tools.last_mut() {
                        tool.status = polaris_ui_cli::screen::PresentationToolStatus::Completed;
                        tool.duration_ms = Some(180);
                    }
                }
                DemoBeat::Questionnaire => {
                    let question = QuestionnaireState::new(demo_questionnaire())?;
                    self.view.push_system(
                        "[DEMO] The following questionnaire is local UI practice; no answer is sent.",
                    )?;
                    self.question = Some(question);
                }
            }
            let _removed = self.beats.pop_front();
            changed = true;
        }
        Ok(changed)
    }

    /// Open the explicit questionnaire scenario without fabricating a backend request.
    pub fn start_questionnaire(&mut self) -> Result<(), DemoError> {
        self.beats.clear();
        self.question = Some(QuestionnaireState::new(demo_questionnaire())?);
        Ok(())
    }

    /// Isolated presentation tools whose source belongs to bounded conversation entries.
    pub fn presentation_tools(&self) -> &[polaris_ui_cli::screen::PresentationTool] {
        &self.tools
    }
    /// Toggle the entry identified by a visible presentation title hit.
    pub fn toggle_tool(&mut self, entry_index: usize) {
        if let Some(tool) = self
            .tools
            .iter_mut()
            .find(|tool| tool.entry_index == entry_index)
        {
            tool.expanded = !tool.expanded;
        }
    }
    /// Apply an expansion state to all present and future demo tools.
    pub fn toggle_all_tools(&mut self) {
        self.tools_expanded = !self.tools_expanded;
        for tool in &mut self.tools {
            tool.expanded = self.tools_expanded;
        }
    }

    /// Returns whether scripted beats remain to be displayed.
    pub fn busy(&self) -> bool {
        !self.beats.is_empty()
    }

    /// Cancels pending presentation beats and clears a pending demo questionnaire.
    pub fn cancel(&mut self) -> Result<(), DemoError> {
        if self.busy() || self.question.is_some() {
            self.view
                .push_system("[DEMO] Simulation cancelled locally; no work was sent.")?;
            self.beats.clear();
            self.question = None;
        }
        Ok(())
    }

    /// Load an explicitly labelled, bounded static transcript fixture.
    pub fn load_fixture(&mut self, name: &str) -> Result<(), DemoError> {
        self.reset()?;
        match name {
            "conversation" | "editor" => {
                self.view
                    .push_user("[DEMO] Extract a shared validation helper.")?;
                self.view.push_assistant("[DEMO] The parser and CLI share bounded validation.\n\n```rust\nfn validate(text: &str) -> Result<(), ValidationError> {\n    if text.is_empty() { return Err(ValidationError::Empty); }\n    Ok(())\n}\n```\n\nReview https://example.com/design for the sample contract. No tools ran and no files changed.")?;
                self.view
                    .push_assistant("[DEMO] Worked for 2m 14s · 09:42 (fixture clock)")?;
            }
            "messages" => {
                self.view.push_assistant(
                    "[DEMO] Ordinary message. This is a local presentation fixture.",
                )?;
                self.view
                    .push_warning("[DEMO] Preview notice: this warning preserves the draft.")?;
                self.view.push_error("[DEMO] Preview failure: this is a sample error, not an actual provider failure.")?;
            }
            "approval" => {
                self.view
                    .push_user("[DEMO] Review the proposed validation patch.")?;
                self.view.push_assistant("[DEMO] Proposed patch:\n```diff\n--- a/src/parser.rs\n+++ b/src/parser.rs\n@@ -1 +1 @@\n-parse_unchecked(input)\n+parse_validated(input)?\n```\nNo change is applied. Open /diff to inspect the fixture.")?;
            }
            "error" => {
                self.view.push_error("[DEMO] Connection unavailable. Draft retained; /providers inspects memory-only configuration.")?;
            }
            _ => {
                self.view.push_system("[DEMO] Unknown transcript fixture")?;
            }
        }
        Ok(())
    }

    /// Clears the transcript and returns the runtime to its initial local state.
    pub fn reset(&mut self) -> Result<(), DemoError> {
        let model = self.view.model.clone();
        let effort = self.view.effort;
        let mode = self.view.mode;
        self.view = ViewState::new(model, effort);
        self.view.mode = mode;
        self.tools.clear();
        self.beats.clear();
        self.last_prompt = None;
        self.question = None;
        self.assistant_stream_open = false;
        Ok(())
    }

    /// Restarts the latest prompt's deterministic presentation from an empty demo transcript.
    ///
    /// # Errors
    /// Returns DemoError::NoReplay if no prompt has been submitted.
    pub fn replay(&mut self) -> Result<(), DemoError> {
        let Some(prompt) = self.last_prompt.clone() else {
            return Err(DemoError::NoReplay);
        };
        self.reset()?;
        self.submit(&prompt)
    }

    /// Borrows the active local questionnaire, if the script has reached it.
    pub fn question(&self) -> Option<&QuestionnaireState> {
        self.question.as_ref()
    }

    /// Mutably borrows the questionnaire for bounded navigation and answer editing.
    pub fn question_mut(&mut self) -> Option<&mut QuestionnaireState> {
        self.question.as_mut()
    }

    /// Validates every required answer and returns one local batch without dispatching it.
    ///
    /// The questionnaire remains intact on validation or view-capacity failure.
    pub fn accept_answer(&mut self) -> Result<QuestionnaireBatch, DemoError> {
        let Some(question) = self.question.as_ref() else {
            return Err(DemoError::NoQuestionnaire);
        };
        let batch = question.prepare_answer()?;
        let summary = format!(
            "[DEMO] Locally accepted {} answers for {}; nothing was sent.",
            batch.answers.len(),
            batch.request_id
        );
        self.view.push_system(summary)?;
        self.question = None;
        Ok(batch)
    }

    fn schedule_script(&mut self) {
        let base = self.elapsed;
        let is_long = self.last_prompt.as_deref().is_some_and(|prompt| {
            prompt
                .split_whitespace()
                .any(|word| word.eq_ignore_ascii_case("long"))
        });
        let mut beats = vec![
            (180, DemoBeat::System("Planning a response…")),
            (360, DemoBeat::ToolStart),
            (540, DemoBeat::ToolComplete),
            (
                720,
                DemoBeat::System(
                    "Diff preview: + demo output is presentation only; no file changed.",
                ),
            ),
            (
                900,
                DemoBeat::Error("Simulated provider timeout. Retry remains local to the demo."),
            ),
            (1_080, DemoBeat::System("Retrying the simulated step…")),
        ];
        if is_long {
            const LONG_CHUNK: &str = "Long demo stream: naïve café · Καλημέρα · こんにちは · 👩‍💻 — bounded incremental output. ";
            for part in 0_u64..64 {
                let millis = 1_260_u64.saturating_add(part.saturating_mul(45));
                beats.push((millis, DemoBeat::AssistantChunk(LONG_CHUNK)));
            }
        } else {
            beats.extend([
                (
                    1_260,
                    DemoBeat::AssistantChunk("Stream: naïve café · Καλημέρα · こんにちは "),
                ),
                (
                    1_440,
                    DemoBeat::AssistantChunk("👩‍💻 keeps the Unicode sequence intact. "),
                ),
                (
                    1_620,
                    DemoBeat::AssistantChunk("This response is scripted demo text."),
                ),
            ]);
        }
        let asks_questions = self.last_prompt.as_deref().is_some_and(|prompt| {
            prompt.split_whitespace().any(|word| {
                matches!(
                    word.to_ascii_lowercase().as_str(),
                    "questions" | "questionnaire"
                )
            })
        });
        if asks_questions {
            beats.push((if is_long { 4_200 } else { 1_800 }, DemoBeat::Questionnaire));
        }
        self.beats = beats
            .into_iter()
            .map(|(millis, beat)| TimedBeat {
                due: base.saturating_add(Duration::from_millis(millis)),
                beat,
            })
            .collect();
    }

    fn append_assistant_chunk(&mut self, chunk: &str) -> Result<(), DemoError> {
        let current_len = self
            .view
            .conversation
            .last()
            .filter(|entry| entry.kind == EntryKind::Assistant)
            .map_or(0, |entry| entry.text.len());
        let prefix_len = if self.assistant_stream_open { 0 } else { 7 };
        let attempted_len = current_len
            .checked_add(prefix_len)
            .and_then(|length| length.checked_add(chunk.len()))
            .ok_or(DemoError::StreamTooLarge)?;
        if attempted_len > MAX_DEMO_STREAM_BYTES {
            return Err(DemoError::StreamTooLarge);
        }
        if !self.assistant_stream_open {
            self.view.push_assistant("[DEMO] ")?;
            self.assistant_stream_open = true;
        }
        let Some(entry) = self.view.conversation.last_mut() else {
            return Err(DemoError::StreamTooLarge);
        };
        if entry.kind != EntryKind::Assistant {
            return Err(DemoError::StreamTooLarge);
        }
        entry
            .text
            .try_reserve(chunk.len())
            .map_err(|_| DemoError::StreamTooLarge)?;
        entry.text.push_str(chunk);
        Ok(())
    }
}

fn demo_questionnaire() -> QuestionnaireSpec {
    QuestionnaireSpec {
        request_id: "demo-questionnaire".to_owned(),
        revision: 1,
        requester_label: "DEMO walkthrough".to_owned(),
        questions: vec![
            QuestionSpec {
                id: "preview_scope".to_owned(),
                prompt: "Which preview should the demo show?".to_owned(),
                required: true,
                kind: QuestionKind::SingleChoice {
                    options: vec![
                        QuestionOption {
                            id: "summary".to_owned(),
                            label: "Short summary".to_owned(),
                        },
                        QuestionOption {
                            id: "details".to_owned(),
                            label: "Detailed walkthrough".to_owned(),
                        },
                    ],
                },
            },
            QuestionSpec {
                id: "signals".to_owned(),
                prompt: "Which simulated signals should be highlighted?".to_owned(),
                required: true,
                kind: QuestionKind::MultiChoice {
                    options: vec![
                        QuestionOption {
                            id: "stream".to_owned(),
                            label: "Streaming".to_owned(),
                        },
                        QuestionOption {
                            id: "retry".to_owned(),
                            label: "Retry state".to_owned(),
                        },
                    ],
                },
            },
            QuestionSpec {
                id: "notes".to_owned(),
                prompt: "Optional note for this demo?".to_owned(),
                required: false,
                kind: QuestionKind::FreeText,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::{DemoError, DemoRuntime, MAX_DEMO_STREAM_BYTES};
    use polaris_ui_model::questionnaire::QuestionnaireAction;
    use polaris_ui_model::view::EntryKind;
    use std::{error::Error, time::Duration};

    type TestResult = Result<(), Box<dyn Error>>;

    #[test]
    fn runtime_is_local_bounded_and_runs_scripted_beats() -> TestResult {
        let mut runtime = DemoRuntime::new()?;
        runtime.submit("show the Unicode stream questionnaire")?;
        assert!(runtime.busy());
        assert!(runtime.tick(Duration::from_millis(1_800))?);
        assert!(!runtime.busy());
        let texts: Vec<&str> = runtime
            .view()
            .conversation
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert!(texts.iter().all(|text| text.starts_with("[DEMO]")));
        assert!(
            texts
                .iter()
                .any(|text| text.contains("Tool: inspect_workspace"))
        );
        assert!(texts.iter().any(|text| text.contains("Diff preview")));
        assert!(texts.iter().any(|text| text.contains("provider timeout")));
        assert!(texts.iter().any(|text| text.contains("Retrying")));
        assert!(texts.iter().any(|text| text.contains("こんにちは")));
        assert_eq!(
            texts
                .iter()
                .filter(|text| text.contains("Stream:") || text.contains("This response"))
                .count(),
            1,
            "incremental output must remain one assistant entry"
        );
        assert!(texts.iter().any(|text| text.contains("👩‍💻")));
        let stream = runtime
            .view()
            .conversation
            .iter()
            .find(|entry| entry.kind == EntryKind::Assistant)
            .ok_or_else(|| std::io::Error::other("assistant stream missing"))?;
        assert!(stream.text.len() <= MAX_DEMO_STREAM_BYTES);
        assert!(runtime.question().is_some());
        assert!(matches!(runtime.submit("another"), Err(DemoError::Busy)));
        Ok(())
    }

    #[test]
    fn questionnaire_acceptance_is_atomic_and_contains_all_questions() -> TestResult {
        let mut runtime = DemoRuntime::new()?;
        runtime.submit("questionnaire practice")?;
        runtime.tick(Duration::from_secs(2))?;
        let question = runtime
            .question()
            .ok_or("questionnaire was not scheduled")?;
        assert_eq!(question.spec().questions.len(), 3);
        assert!(runtime.accept_answer().is_err());
        assert!(
            runtime.question().is_some(),
            "incomplete batch must retain all drafts"
        );

        let question = runtime.question_mut().ok_or("questionnaire missing")?;
        question.apply(QuestionnaireAction::ToggleChoice)?;
        question.navigate(1);
        question.apply(QuestionnaireAction::ToggleChoice)?;
        question.navigate(1);
        question.apply(QuestionnaireAction::FocusNotes)?;
        question.apply(QuestionnaireAction::InsertText("DEMO note".to_owned()))?;

        let batch = runtime.accept_answer()?;
        assert_eq!(batch.answers.len(), 3);
        assert!(runtime.question().is_none());
        assert!(
            runtime
                .view()
                .conversation
                .last()
                .is_some_and(|entry| entry.text.contains("nothing was sent"))
        );
        Ok(())
    }

    #[test]
    fn cancel_reset_and_replay_have_bounded_local_lifecycles() -> TestResult {
        let mut runtime = DemoRuntime::new()?;
        assert!(matches!(runtime.replay(), Err(DemoError::NoReplay)));
        runtime.submit("retryable demo")?;
        runtime.tick(Duration::from_secs(10))?;
        runtime.cancel()?;
        assert!(!runtime.busy());
        assert!(
            runtime
                .view()
                .conversation
                .last()
                .is_some_and(|entry| entry.text.starts_with("[DEMO]"))
        );
        runtime.replay()?;
        assert!(runtime.busy());
        assert!(!runtime.tick(Duration::from_secs(10))?);
        assert!(runtime.tick(Duration::from_millis(10_180))?);
        runtime.reset()?;
        assert!(!runtime.busy());
        assert!(runtime.view().conversation.is_empty());
        assert!(matches!(runtime.replay(), Err(DemoError::NoReplay)));
        Ok(())
    }

    #[test]
    fn long_stream_is_incremental_in_one_bounded_entry() -> TestResult {
        let mut runtime = DemoRuntime::new()?;
        runtime.submit("please run the long demo questionnaire")?;
        assert!(runtime.tick(Duration::from_millis(1_260))?);
        let assistant_count = runtime
            .view()
            .conversation
            .iter()
            .filter(|entry| entry.kind == EntryKind::Assistant)
            .count();
        assert_eq!(assistant_count, 1);
        let partial_len = runtime
            .view()
            .conversation
            .last()
            .ok_or("stream entry missing")?
            .text
            .len();
        assert!(partial_len > 7);

        runtime.tick(Duration::from_millis(4_200))?;
        let streams: Vec<&str> = runtime
            .view()
            .conversation
            .iter()
            .filter(|entry| entry.kind == EntryKind::Assistant)
            .map(|entry| entry.text.as_str())
            .collect();
        assert_eq!(streams.len(), 1);
        let stream = streams.first().ok_or("long assistant stream missing")?;
        assert!(stream.len() <= MAX_DEMO_STREAM_BYTES);
        assert!(stream.len() > 4_000);
        assert!(runtime.question().is_some());
        Ok(())
    }
}
