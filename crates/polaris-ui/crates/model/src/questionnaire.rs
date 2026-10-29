//! Pure, bounded questionnaire draft state. This module does not dispatch human requests.
use crate::composer::Composer;

/// Maximum UTF-8 byte length accepted for one prompt, option, answer, or notes field.
pub const MAX_FIELD_BYTES: usize = 16 * 1024;
/// Maximum questions retained in one questionnaire draft.
pub const MAX_QUESTIONS: usize = 8;
/// Maximum named options accepted for one choice question.
pub const MAX_OPTIONS: usize = 16;
/// Maximum conservative encoded-size estimate for request metadata.
pub const MAX_REQUEST_BYTES: usize = 256 * 1024;
/// Maximum conservative encoded-size estimate for one answer batch.
///
/// This bounds presentation drafts before a domain wire codec exists; it is not an
/// authoritative serialized-payload measurement.
pub const MAX_ANSWER_BYTES: usize = 128 * 1024;

const REQUEST_ENVELOPE_OVERHEAD: usize = 256;
const QUESTION_ENVELOPE_OVERHEAD: usize = 256;
const OPTION_ENVELOPE_OVERHEAD: usize = 128;
const ANSWER_ENVELOPE_OVERHEAD: usize = 256;

fn conservative_json_string_len(value: &str) -> usize {
    value.chars().fold(2usize, |size, character| {
        let encoded = if character.is_ascii_control()
            || !character.is_ascii() && character.len_utf16() == 1
        {
            6
        } else if character == '"' || character == '\\' {
            2
        } else if character.is_ascii() {
            1
        } else {
            12
        };
        size.saturating_add(encoded)
    })
}

fn estimate_request_size(spec: &QuestionnaireSpec) -> usize {
    let mut size = REQUEST_ENVELOPE_OVERHEAD
        .saturating_add(conservative_json_string_len(&spec.request_id))
        .saturating_add(conservative_json_string_len(&spec.requester_label));
    for question in &spec.questions {
        size = size
            .saturating_add(QUESTION_ENVELOPE_OVERHEAD)
            .saturating_add(conservative_json_string_len(&question.id))
            .saturating_add(conservative_json_string_len(&question.prompt));
        if let QuestionKind::SingleChoice { options } | QuestionKind::MultiChoice { options } =
            &question.kind
        {
            for option in options {
                size = size
                    .saturating_add(OPTION_ENVELOPE_OVERHEAD)
                    .saturating_add(conservative_json_string_len(&option.id))
                    .saturating_add(conservative_json_string_len(&option.label));
            }
        }
    }
    size
}

fn estimate_answer_size(
    spec: &QuestionnaireSpec,
    drafts: &[AnswerDraft],
    replacement: Option<(usize, QuestionnaireFocus, &str)>,
) -> usize {
    let mut size =
        ANSWER_ENVELOPE_OVERHEAD.saturating_add(conservative_json_string_len(&spec.request_id));
    for (index, question) in spec.questions.iter().enumerate() {
        let Some(draft) = drafts.get(index) else {
            return usize::MAX;
        };
        size = size
            .saturating_add(QUESTION_ENVELOPE_OVERHEAD)
            .saturating_add(conservative_json_string_len(&question.id));
        let draft_text = if replacement
            .is_some_and(|(at, focus, _)| at == index && focus == QuestionnaireFocus::Choices)
        {
            let prospective = replacement.map_or("", |(_, _, text)| text);
            conservative_json_string_len(prospective)
        } else {
            conservative_json_string_len(draft.text.text())
        };
        let draft_notes = if replacement
            .is_some_and(|(at, focus, _)| at == index && focus == QuestionnaireFocus::Notes)
        {
            let prospective = replacement.map_or("", |(_, _, text)| text);
            conservative_json_string_len(prospective)
        } else {
            conservative_json_string_len(draft.notes.text())
        };
        size = size.saturating_add(draft_text).saturating_add(draft_notes);
        match &question.kind {
            QuestionKind::SingleChoice { options } | QuestionKind::MultiChoice { options } => {
                for (option, selected) in options.iter().zip(draft.selected.iter()) {
                    if *selected {
                        size = size.saturating_add(conservative_json_string_len(&option.id));
                    }
                }
            }
            QuestionKind::FreeText => {}
        }
    }
    size
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionOption {
    /// Stable option identifier returned in the answer batch.
    pub id: String,
    /// Human-readable label shown to the responder.
    pub label: String,
}
/// Supported response shape for one question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionKind {
    /// Exactly one named option or “None of the above” may be selected.
    SingleChoice { options: Vec<QuestionOption> },
    /// One or more named options, or exclusively “None of the above”, may be selected.
    MultiChoice { options: Vec<QuestionOption> },
    /// The responder supplies a text answer.
    FreeText,
}
/// Immutable, validated question metadata supplied by the requesting service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionSpec {
    /// Stable question identifier unique within the request.
    pub id: String,
    /// Prompt displayed to the responder.
    pub prompt: String,
    /// Whether a concrete answer is required before submission.
    pub required: bool,
    /// Permitted response form.
    pub kind: QuestionKind,
}
/// Bounded questionnaire metadata used to initialize a local draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionnaireSpec {
    /// Identity of the outstanding request.
    pub request_id: String,
    /// Request revision used to reject stale answers.
    pub revision: u64,
    /// Display label for the requester.
    pub requester_label: String,
    /// Questions in display and submission order.
    pub questions: Vec<QuestionSpec>,
}
/// Current keyboard target within the active question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestionnaireFocus {
    /// Choice navigation and selection are active.
    Choices,
    /// Notes or free-text editing is active.
    Notes,
}
/// Validation or editing failure for a questionnaire draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionnaireError {
    /// Request identity contains no non-whitespace text.
    EmptyRequestId,
    /// Question count exceeds the configured bound.
    TooManyQuestions,
    /// Metadata is empty, duplicated, or structurally invalid.
    InvalidQuestion,
    /// A choice question has too many named options.
    TooManyOptions,
    /// The current question has no selectable choice.
    NoChoice,
    /// A field or aggregate draft exceeds its configured byte bound.
    FieldTooLarge,
}
impl std::fmt::Display for QuestionnaireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for QuestionnaireError {}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionnaireAction {
    /// Move the choice cursor down, bounded by the last choice.
    ChoiceDown,
    /// Move the choice cursor up, bounded by the first choice.
    ChoiceUp,
    /// Toggle the highlighted choice, including exclusive “None of the above”.
    ToggleChoice,
    /// Direct subsequent inserted text to optional notes.
    FocusNotes,
    /// Leave notes and return keyboard focus to choices.
    FocusChoices,
    /// Append text to the active notes or free-text field.
    InsertText(String),
    /// Remove the preceding grapheme or the selected text.
    Backspace,
    /// Remove the following grapheme or the selected text.
    Delete,
    /// Move through the active text editor.
    MoveLeft { extend: bool, word: bool },
    /// Move through the active text editor.
    MoveRight { extend: bool, word: bool },
    /// Move to the logical line start.
    Home { extend: bool },
    /// Move to the logical line end.
    End { extend: bool },
    /// Set a checked grapheme cursor offset.
    SetCursor(usize),
    /// Set a range selection in the active notes or free-text editor.
    SetSelection { anchor: usize, head: usize },
    /// Select all text in the active field.
    SelectAll,
    /// Undo the active field edit.
    Undo,
    /// Redo the active field edit.
    Redo,
}
/// Multi-select response; the explicit none choice excludes named options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultiChoiceAnswer {
    /// Selected stable option identifiers.
    Options(Vec<String>),
    /// Explicitly chose none of the named options.
    NoneOfTheAbove,
}
/// Single-choice response, including the explicit none choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoiceAnswer {
    /// Stable identifier of the selected named option.
    Option(String),
    /// Explicitly chose none of the named options.
    NoneOfTheAbove,
}
/// Serialized response state for one question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuestionAnswer {
    /// No answer was supplied; valid only for optional questions.
    Unanswered,
    /// One choice was supplied.
    SingleChoice(ChoiceAnswer),
    /// A set of choices was supplied.
    MultiChoice(MultiChoiceAnswer),
    /// Nonblank free-text response.
    FreeText(String),
}
/// One answer and its optional notes, in question order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerEntry {
    /// Identifier of the answered question.
    pub question_id: String,
    /// Answer payload or explicit unanswered state.
    pub answer: QuestionAnswer,
    /// Optional additional context from the responder.
    pub notes: String,
}
/// Atomic response payload for a request revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionnaireBatch {
    /// Identity of the request being answered.
    pub request_id: String,
    /// Revision the response was prepared against.
    pub revision: u64,
    /// Responses in the original question order.
    pub answers: Vec<AnswerEntry>,
}
/// Identifies which required question prevented batch submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// Stable identifier of the unanswered question.
    pub question_id: String,
    /// Zero-based position in the questionnaire.
    pub question_index: usize,
}
/// Failure to prepare a complete and structurally consistent answer batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerSubmitError {
    /// A required question is unanswered.
    Incomplete(ValidationError),
    /// Draft and specification structure do not agree.
    Invalid,
    /// Conservative encoded-size estimate exceeds the answer batch limit.
    TooLarge,
}
impl std::fmt::Display for AnswerSubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for AnswerSubmitError {}
#[derive(Debug, Clone, PartialEq, Eq)]
struct AnswerDraft {
    selected: Vec<bool>,
    none: bool,
    text: Composer,
    notes: Composer,
}
/// Bounded editable response state for a validated questionnaire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionnaireState {
    spec: QuestionnaireSpec,
    answers: Vec<AnswerDraft>,
    index: usize,
    focus: QuestionnaireFocus,
    option_cursor: usize,
}
impl QuestionnaireState {
    /// Validate and initialize a bounded local questionnaire draft.
    ///
    /// # Errors
    /// Returns an error when metadata is invalid or exceeds a configured bound.
    pub fn new(spec: QuestionnaireSpec) -> Result<Self, QuestionnaireError> {
        if spec.request_id.trim().is_empty() {
            return Err(QuestionnaireError::EmptyRequestId);
        }
        if spec.request_id.len() > MAX_FIELD_BYTES {
            return Err(QuestionnaireError::FieldTooLarge);
        }
        if spec.questions.is_empty() {
            return Err(QuestionnaireError::InvalidQuestion);
        }
        if spec.questions.len() > MAX_QUESTIONS {
            return Err(QuestionnaireError::TooManyQuestions);
        }
        if spec.requester_label.len() > MAX_FIELD_BYTES {
            return Err(QuestionnaireError::FieldTooLarge);
        }
        if estimate_request_size(&spec) > MAX_REQUEST_BYTES {
            return Err(QuestionnaireError::FieldTooLarge);
        }
        let mut question_ids = std::collections::BTreeSet::new();
        for q in &spec.questions {
            if !question_ids.insert(q.id.as_str()) {
                return Err(QuestionnaireError::InvalidQuestion);
            }
            if q.id.trim().is_empty() || q.prompt.trim().is_empty() {
                return Err(QuestionnaireError::InvalidQuestion);
            }
            if q.id.len() > MAX_FIELD_BYTES || q.prompt.len() > MAX_FIELD_BYTES {
                return Err(QuestionnaireError::FieldTooLarge);
            }
            if let QuestionKind::SingleChoice { options } | QuestionKind::MultiChoice { options } =
                &q.kind
            {
                if options.is_empty() {
                    return Err(QuestionnaireError::InvalidQuestion);
                }
                for option in options {
                    if option.id.len() > MAX_FIELD_BYTES || option.label.len() > MAX_FIELD_BYTES {
                        return Err(QuestionnaireError::FieldTooLarge);
                    }
                }
                if options.len() > MAX_OPTIONS {
                    return Err(QuestionnaireError::TooManyOptions);
                }
                let mut ids = std::collections::BTreeSet::new();
                if options.iter().any(|o| {
                    o.id.trim().is_empty()
                        || o.label.trim().is_empty()
                        || !ids.insert(o.id.as_str())
                }) {
                    return Err(QuestionnaireError::InvalidQuestion);
                }
            }
        }
        let answers = spec
            .questions
            .iter()
            .map(|q| {
                let n = match &q.kind {
                    QuestionKind::SingleChoice { options }
                    | QuestionKind::MultiChoice { options } => options.len().saturating_add(1),
                    QuestionKind::FreeText => 0,
                };
                AnswerDraft {
                    selected: vec![false; n],
                    none: false,
                    text: Composer::new(),
                    notes: Composer::new(),
                }
            })
            .collect();
        Ok(Self {
            spec,
            answers,
            index: 0,
            focus: QuestionnaireFocus::Choices,
            option_cursor: 0,
        })
    }
    fn question(&self) -> Option<&QuestionSpec> {
        self.spec.questions.get(self.index)
    }
    fn current(&self) -> Option<&AnswerDraft> {
        self.answers.get(self.index)
    }
    fn current_mut(&mut self) -> Option<&mut AnswerDraft> {
        self.answers.get_mut(self.index)
    }
    #[must_use]
    /// Zero-based index of the active question.
    pub const fn question_index(&self) -> usize {
        self.index
    }
    #[must_use]
    /// Validated questionnaire metadata backing this draft.
    pub fn spec(&self) -> &QuestionnaireSpec {
        &self.spec
    }
    #[must_use]
    /// Active question, or `None` if internal state is inconsistent.
    pub fn current_question(&self) -> Option<&QuestionSpec> {
        self.question()
    }
    #[must_use]
    /// Zero-based choice cursor, including the final “None of the above” row.
    pub const fn choice_cursor(&self) -> usize {
        self.option_cursor
    }
    #[must_use]
    /// Current keyboard focus target.
    pub const fn focus(&self) -> QuestionnaireFocus {
        self.focus
    }
    #[must_use]
    /// Number of questions with any answer or nonblank free-text draft.
    pub fn answered_count(&self) -> usize {
        self.answers
            .iter()
            .filter(|a| a.none || a.selected.iter().any(|x| *x) || !a.text.text().trim().is_empty())
            .count()
    }
    #[must_use]
    /// Selected named option identifiers for the active choice question.
    pub fn selected_named_options(&self) -> Vec<String> {
        let (Some(q), Some(a)) = (self.question(), self.current()) else {
            return Vec::new();
        };
        let opts = match &q.kind {
            QuestionKind::SingleChoice { options } | QuestionKind::MultiChoice { options } => {
                options
            }
            _ => return Vec::new(),
        };
        opts.iter()
            .zip(a.selected.iter())
            .filter_map(|(o, selected)| if *selected { Some(o.id.clone()) } else { None })
            .collect()
    }
    #[must_use]
    /// Whether “None of the above” is selected for the active question.
    pub fn none_of_the_above_selected(&self) -> bool {
        self.current().is_some_and(|a| a.none)
    }
    #[must_use]
    /// Optional notes entered for the active question.
    pub fn notes(&self) -> &str {
        self.current().map_or("", |a| a.notes.text())
    }
    #[must_use]
    /// Free-text answer entered for the active question.
    pub fn answer_text(&self) -> &str {
        self.current().map_or("", |a| a.text.text())
    }
    /// Borrow the active notes/free-text editor with shared Unicode and selection semantics.
    #[must_use]
    pub fn active_editor(&self) -> Option<&Composer> {
        let answer = self.current()?;
        Some(if self.focus == QuestionnaireFocus::Notes {
            &answer.notes
        } else {
            &answer.text
        })
    }
    fn active_editor_mut(&mut self) -> Option<&mut Composer> {
        let notes = self.focus == QuestionnaireFocus::Notes;
        let answer = self.current_mut()?;
        Some(if notes {
            &mut answer.notes
        } else {
            &mut answer.text
        })
    }
    /// Current UTF-8 byte cursor in the active text field.
    #[must_use]
    pub fn active_text_cursor(&self) -> usize {
        self.active_editor().map_or(0, Composer::cursor)
    }
    /// Ordered review projection, including explicit unanswered fields; does not submit or validate.
    #[must_use]
    pub fn draft_answers(&self) -> Vec<AnswerEntry> {
        self.spec
            .questions
            .iter()
            .zip(self.answers.iter())
            .map(|(q, a)| {
                let answer = match &q.kind {
                    QuestionKind::SingleChoice { options } => {
                        if a.none {
                            QuestionAnswer::SingleChoice(ChoiceAnswer::NoneOfTheAbove)
                        } else if let Some(option) =
                            options
                                .iter()
                                .zip(a.selected.iter())
                                .find_map(
                                    |(option, selected)| {
                                        if *selected { Some(option) } else { None }
                                    },
                                )
                        {
                            QuestionAnswer::SingleChoice(ChoiceAnswer::Option(option.id.clone()))
                        } else {
                            QuestionAnswer::Unanswered
                        }
                    }
                    QuestionKind::MultiChoice { options } => {
                        if a.none {
                            QuestionAnswer::MultiChoice(MultiChoiceAnswer::NoneOfTheAbove)
                        } else {
                            let selected: Vec<_> = options
                                .iter()
                                .zip(a.selected.iter())
                                .filter_map(|(option, selected)| {
                                    if *selected {
                                        Some(option.id.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                            if selected.is_empty() {
                                QuestionAnswer::Unanswered
                            } else {
                                QuestionAnswer::MultiChoice(MultiChoiceAnswer::Options(selected))
                            }
                        }
                    }
                    QuestionKind::FreeText => {
                        if a.text.text().trim().is_empty() {
                            QuestionAnswer::Unanswered
                        } else {
                            QuestionAnswer::FreeText(a.text.text().to_owned())
                        }
                    }
                };
                AnswerEntry {
                    question_id: q.id.clone(),
                    answer,
                    notes: a.notes.text().to_owned(),
                }
            })
            .collect()
    }
    /// Remaining conservative answer-byte budget for the current draft.
    #[must_use]
    pub fn remaining_answer_bytes(&self) -> usize {
        MAX_ANSWER_BYTES.saturating_sub(estimate_answer_size(&self.spec, &self.answers, None))
    }
    /// Select a valid question directly, preserving every answer and editor snapshot.
    pub fn select_question(&mut self, index: usize) -> bool {
        if index >= self.spec.questions.len() {
            return false;
        }
        self.index = index;
        self.focus = QuestionnaireFocus::Choices;
        self.option_cursor = 0;
        true
    }
    /// Move to another question, clamping at either end of the questionnaire.
    pub fn navigate(&mut self, delta: isize) {
        if self.spec.questions.is_empty() {
            return;
        }
        let next = if delta < 0 {
            self.index.saturating_sub(delta.unsigned_abs())
        } else {
            self.index
                .saturating_add(match usize::try_from(delta) {
                    Ok(value) => value,
                    Err(_) => usize::MAX,
                })
                .min(self.spec.questions.len().saturating_sub(1))
        };
        self.index = next;
        self.focus = QuestionnaireFocus::Choices;
        self.option_cursor = 0;
    }
    fn commit_editor(&mut self, candidate: Composer) -> Result<(), QuestionnaireError> {
        if candidate.text().len() > MAX_FIELD_BYTES
            || estimate_answer_size(
                &self.spec,
                &self.answers,
                Some((self.index, self.focus, candidate.text())),
            ) > MAX_ANSWER_BYTES
        {
            return Err(QuestionnaireError::FieldTooLarge);
        }
        let editor = self
            .active_editor_mut()
            .ok_or(QuestionnaireError::InvalidQuestion)?;
        *editor = candidate;
        Ok(())
    }
    /// Apply one bounded local editing action.
    ///
    /// # Errors
    /// Returns [`QuestionnaireError::NoChoice`] for invalid choice operations and
    /// [`QuestionnaireError::FieldTooLarge`] when text would exceed a byte limit.
    pub fn apply(&mut self, action: QuestionnaireAction) -> Result<(), QuestionnaireError> {
        match action {
            QuestionnaireAction::ChoiceDown | QuestionnaireAction::ChoiceUp => {
                let count = self.question().map_or(0, |q| match &q.kind {
                    QuestionKind::SingleChoice { options }
                    | QuestionKind::MultiChoice { options } => options.len().saturating_add(1),
                    QuestionKind::FreeText => 0,
                });
                if count > 0 {
                    self.option_cursor = if matches!(action, QuestionnaireAction::ChoiceDown) {
                        self.option_cursor
                            .saturating_add(1)
                            .min(count.saturating_sub(1))
                    } else {
                        self.option_cursor.saturating_sub(1)
                    };
                }
            }
            QuestionnaireAction::FocusNotes => self.focus = QuestionnaireFocus::Notes,
            QuestionnaireAction::FocusChoices => self.focus = QuestionnaireFocus::Choices,
            QuestionnaireAction::ToggleChoice => {
                let q = self.question().ok_or(QuestionnaireError::NoChoice)?;
                if matches!(&q.kind, QuestionKind::FreeText) {
                    return Err(QuestionnaireError::NoChoice);
                }
                let option_cursor = self.option_cursor;
                let single = matches!(&q.kind, QuestionKind::SingleChoice { .. });
                let a = self.current_mut().ok_or(QuestionnaireError::NoChoice)?;
                let len = a.selected.len();
                if option_cursor >= len {
                    return Err(QuestionnaireError::NoChoice);
                }
                let is_none = option_cursor == len.saturating_sub(1);
                if is_none {
                    a.none = !a.none;
                    if a.none {
                        a.selected.fill(false);
                    }
                } else if single {
                    let was_selected = match a.selected.get(option_cursor) {
                        Some(value) => *value,
                        None => false,
                    };
                    a.selected.fill(false);
                    if !was_selected && let Some(selected) = a.selected.get_mut(option_cursor) {
                        *selected = true;
                    }
                    a.none = false;
                } else if let Some(selected) = a.selected.get_mut(option_cursor) {
                    *selected = !*selected;
                    if *selected {
                        a.none = false;
                    }
                }
            }
            QuestionnaireAction::InsertText(text) => {
                let mut candidate = self
                    .active_editor()
                    .ok_or(QuestionnaireError::InvalidQuestion)?
                    .clone();
                let range = match candidate.selection() {
                    Some(range) => range,
                    None => candidate.cursor()..candidate.cursor(),
                };
                candidate
                    .apply_completion(range, &text)
                    .map_err(|_| QuestionnaireError::FieldTooLarge)?;
                self.commit_editor(candidate)?;
            }
            QuestionnaireAction::Backspace => {
                if let Some(editor) = self.active_editor_mut() {
                    let _changed = editor.backspace();
                }
            }
            QuestionnaireAction::Delete => {
                if let Some(editor) = self.active_editor_mut() {
                    let _changed = editor.delete();
                }
            }
            QuestionnaireAction::MoveLeft { extend, word } => {
                if let Some(editor) = self.active_editor_mut() {
                    if word {
                        editor.move_word_left_with_selection(extend);
                    } else {
                        editor.move_left_with_selection(extend);
                    }
                }
            }
            QuestionnaireAction::MoveRight { extend, word } => {
                if let Some(editor) = self.active_editor_mut() {
                    if word {
                        editor.move_word_right_with_selection(extend);
                    } else {
                        editor.move_right_with_selection(extend);
                    }
                }
            }
            QuestionnaireAction::Home { extend } => {
                if let Some(editor) = self.active_editor_mut() {
                    editor.move_line_home_with_selection(extend);
                }
            }
            QuestionnaireAction::End { extend } => {
                if let Some(editor) = self.active_editor_mut() {
                    editor.move_line_end_with_selection(extend);
                }
            }
            QuestionnaireAction::SetCursor(at) => {
                if let Some(editor) = self.active_editor_mut() {
                    editor.set_cursor(at);
                }
            }
            QuestionnaireAction::SetSelection { anchor, head } => {
                if let Some(editor) = self.active_editor_mut() {
                    editor.set_selection(anchor..head);
                }
            }
            QuestionnaireAction::SelectAll => {
                if let Some(editor) = self.active_editor_mut() {
                    editor.select_all();
                }
            }
            QuestionnaireAction::Undo => {
                if let Some(mut candidate) = self.active_editor().cloned()
                    && candidate.undo()
                {
                    self.commit_editor(candidate)?;
                }
            }
            QuestionnaireAction::Redo => {
                if let Some(mut candidate) = self.active_editor().cloned()
                    && candidate.redo()
                {
                    self.commit_editor(candidate)?;
                }
            }
        }
        Ok(())
    }
    /// Validate required answers and prepare one ordered response batch.
    ///
    /// This method does not submit the batch to a requester or persist it.
    ///
    /// # Errors
    /// Returns [`AnswerSubmitError::Incomplete`] when a required question is unanswered,
    /// [`AnswerSubmitError::Invalid`] when draft structure is inconsistent, or
    /// [`AnswerSubmitError::TooLarge`] when the conservative batch-size estimate exceeds its cap.
    pub fn prepare_answer(&self) -> Result<QuestionnaireBatch, AnswerSubmitError> {
        if estimate_answer_size(&self.spec, &self.answers, None) > MAX_ANSWER_BYTES {
            return Err(AnswerSubmitError::TooLarge);
        }
        let entries = self.draft_answers();
        if entries.len() != self.spec.questions.len() {
            return Err(AnswerSubmitError::Invalid);
        }
        for (index, (question, entry)) in self.spec.questions.iter().zip(entries.iter()).enumerate()
        {
            if question.required && entry.answer == QuestionAnswer::Unanswered {
                return Err(AnswerSubmitError::Incomplete(ValidationError {
                    question_id: question.id.clone(),
                    question_index: index,
                }));
            }
        }
        Ok(QuestionnaireBatch {
            request_id: self.spec.request_id.clone(),
            revision: self.spec.revision,
            answers: entries,
        })
    }
}
