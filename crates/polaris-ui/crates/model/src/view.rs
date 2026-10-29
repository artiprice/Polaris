//! Serializable frontend state. Event payload text is resolved separately from CAS by the caller.

use crate::error::{ProjectionResource, ReduceError, ViewError};
use polaris_sdk::effort::Effort;
use polaris_sdk::ids::{Cid, FrameId, ModelId, Seq, SessionId, ShortRef};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_CONVERSATION_ENTRIES: usize = 1024;
pub const MAX_CONVERSATION_BYTES: usize = 16 << 20;
pub const MAX_TOOL_ACTIVITIES: usize = 4096;
pub const MAX_TOOL_METADATA_BYTES: usize = 16 << 20;
pub const MAX_SESSION_ROWS: usize = 1024;
pub const MAX_SESSION_VIEWS: usize = 1024;

/// Current conversation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// The agent can propose execution.
    Auto,
    /// The agent prepares a plan.
    Plan,
}
impl Mode {
    /// The other mode.
    #[must_use]
    pub const fn toggle(self) -> Self {
        match self {
            Self::Auto => Self::Plan,
            Self::Plan => Self::Auto,
        }
    }
}

/// Source/presentation category of a conversation entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    /// Submitted prompt.
    User,
    /// Model response.
    Assistant,
    /// Application notice.
    System,
    /// Nonfatal warning visible to the user.
    Warning,
    /// Failure visible to the user.
    Error,
}
/// Resolved conversation text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Presentation role.
    pub kind: EntryKind,
    /// Semantic source text, never terminal escape commands.
    pub text: String,
}

/// Tool execution state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolStatus {
    /// No completion record has arrived.
    Running,
    /// The tool completed successfully.
    Succeeded,
    /// The tool reported failure.
    Failed,
}
/// One tool call and its frontend disclosure state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolActivity {
    /// Owning context frame.
    pub frame: FrameId,
    /// CAS reference to the call record.
    pub call: Cid,
    /// Title after the application resolves the call's text.
    pub title: Option<String>,
    /// Execution state.
    pub status: ToolStatus,
    /// Measured duration when complete.
    pub latency_us: Option<u64>,
    /// Local disclosure override; `None` follows the global setting.
    pub expanded: Option<bool>,
}
impl ToolActivity {
    /// Resolved title or an explicit unresolved call reference.
    #[must_use]
    pub fn title_text(&self) -> String {
        match &self.title {
            Some(title) => title.clone(),
            None => format!("tool call {}", ShortRef::of(&self.call).to_marker()),
        }
    }
}

/// Whether time is currently accumulating toward a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalState {
    /// Work is active.
    Active,
    /// Work is paused.
    Paused,
}
/// A goal and its active-time clock; the caller injects time in milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalLine {
    /// Goal description.
    pub text: String,
    /// Current state.
    pub state: GoalState,
    accumulated_ms: u64,
    active_since_ms: u64,
}
impl GoalLine {
    /// Accumulated active milliseconds; a backward clock contributes zero new time.
    #[must_use]
    pub fn active_ms(&self, now_ms: u64) -> u64 {
        match self.state {
            GoalState::Active => self
                .accumulated_ms
                .saturating_add(now_ms.saturating_sub(self.active_since_ms)),
            GoalState::Paused => self.accumulated_ms,
        }
    }
    /// Status text shown above the composer.
    #[must_use]
    pub fn label(&self, now_ms: u64) -> String {
        let state = match self.state {
            GoalState::Active => "active",
            GoalState::Paused => "paused",
        };
        format!(
            "goal {state} ({})",
            format_duration_ms(self.active_ms(now_ms))
        )
    }
}

/// Format a duration with second precision and no wrapping arithmetic.
#[must_use]
pub fn format_duration_ms(ms: u64) -> String {
    let seconds = match ms.checked_div(1000) {
        Some(value) => value,
        None => 0,
    };
    let hours = match seconds.checked_div(3600) {
        Some(value) => value,
        None => 0,
    };
    let minutes = match seconds.checked_div(60).and_then(|v| v.checked_rem(60)) {
        Some(value) => value,
        None => 0,
    };
    let rest = match seconds.checked_rem(60) {
        Some(value) => value,
        None => 0,
    };
    if hours > 0 {
        format!("{hours}h {minutes}m {rest}s")
    } else if minutes > 0 {
        format!("{minutes}m {rest}s")
    } else {
        format!("{rest}s")
    }
}

/// Latest completion-verification outcome observed on the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProofOutcome {
    /// The gate passed.
    Passed,
    /// The gate rejected the claim.
    Blocked,
}
/// Latest restart count for each worker's current intensity window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerRestarts {
    /// System One.
    pub s1: u32,
    /// Document ingestion.
    pub ingest: u32,
    /// Needle worker.
    pub nano: u32,
}
/// Operational counters derived only from observed events.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSnapshot {
    /// Whether a session is open.
    pub session_open: bool,
    /// Worker restart-window counters.
    pub worker_restarts: WorkerRestarts,
    /// Circuit-open events.
    pub circuit_trips: u64,
    /// Dead-letter events.
    pub dead_letters: u64,
    /// Approved diff events.
    pub diffs_approved: u64,
    /// Rejected diff events.
    pub diffs_rejected: u64,
    /// Latest proof gate result, absent until observed.
    pub proof_of_done: Option<ProofOutcome>,
    /// Applied directive events.
    pub directives_applied: u64,
}

/// Token and latency measurements; zero counts mean no corresponding records were observed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenWeather {
    /// Completed model requests.
    pub requests: u64,
    /// Total prompt tokens.
    pub prompt_tokens: u64,
    /// Total generated tokens.
    pub completion_tokens: u64,
    /// Total cached prompt tokens.
    pub cached_tokens: u64,
    /// Total billed micro-US dollars.
    pub cost_micro_usd: u64,
    /// Prompt tokens in the latest request.
    pub last_prompt_tokens: u32,
    /// Most recent System Two latency.
    pub last_s2_latency_us: Option<u64>,
    /// Most recent System One latency.
    pub last_s1_latency_us: Option<u64>,
    /// Completed System One decisions.
    pub decisions: u64,
}
impl TokenWeather {
    /// Cached share per thousand, absent before prompt usage is observed.
    #[must_use]
    pub fn cache_hit_permille(&self) -> Option<u16> {
        let ratio = u128::from(self.cached_tokens)
            .saturating_mul(1000)
            .checked_div(u128::from(self.prompt_tokens))?;
        u16::try_from(ratio.min(1000)).ok()
    }
}

/// Session lifecycle observed during replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRow {
    /// Session id.
    pub session: SessionId,
    /// Workspace descriptor, absent if replay began after the start event.
    pub workspace: Option<Cid>,
    /// Start time in Unix milliseconds (or first observed close time for partial replay).
    pub started_at_ms: i64,
    /// First observed sequence.
    pub started_seq: Seq,
    /// Whether a close event was observed.
    pub closed: bool,
}

/// Frontend state, independent of terminal, network and persistence I/O.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewState {
    /// Resolved conversation entries.
    pub conversation: Vec<Entry>,
    /// Tool activity in arrival order.
    pub tools: Vec<ToolActivity>,
    /// Global tool disclosure preference.
    pub tools_expanded: bool,
    /// Active or paused goal.
    pub goal: Option<GoalLine>,
    /// Observed usage and latency.
    pub hud: TokenWeather,
    /// Session lifecycle rows.
    pub sessions: Vec<SessionRow>,
    /// Conversation mode.
    pub mode: Mode,
    /// Active model.
    pub model: ModelId,
    /// Active model's effort.
    pub effort: Effort,
    /// Observed operational status.
    pub status: StatusSnapshot,
    /// Last replayed record; absent before replay starts.
    pub last_seq: Option<Seq>,
    effort_preferences: BTreeMap<String, Effort>,
}
impl ViewState {
    /// Empty view with explicit model and effort; no simulated measurements.
    #[must_use]
    pub fn new(model: ModelId, effort: Effort) -> Self {
        let effort_preferences = BTreeMap::from([(model.as_str().to_owned(), effort)]);
        Self {
            conversation: Vec::new(),
            tools: Vec::new(),
            tools_expanded: false,
            goal: None,
            hud: TokenWeather::default(),
            sessions: Vec::new(),
            mode: Mode::Auto,
            model,
            effort,
            status: StatusSnapshot::default(),
            last_seq: None,
            effort_preferences,
        }
    }
    fn conversation_bytes(&self) -> Result<usize, ViewError> {
        let base = self
            .conversation
            .len()
            .checked_mul(core::mem::size_of::<Entry>())
            .ok_or(ViewError::AccountingOverflow)?;
        self.conversation.iter().try_fold(base, |total, entry| {
            total
                .checked_add(entry.text.capacity())
                .ok_or(ViewError::AccountingOverflow)
        })
    }
    fn check_conversation_capacity(&self, text: &String) -> Result<(), ViewError> {
        if self.conversation.len() >= MAX_CONVERSATION_ENTRIES {
            return Err(ViewError::ConversationEntries {
                limit: MAX_CONVERSATION_ENTRIES,
            });
        }
        let current = self.conversation_bytes()?;
        let attempted = current
            .checked_add(core::mem::size_of::<Entry>())
            .and_then(|bytes| bytes.checked_add(text.capacity()))
            .ok_or(ViewError::AccountingOverflow)?;
        if attempted > MAX_CONVERSATION_BYTES {
            return Err(ViewError::ConversationBytes {
                limit: MAX_CONVERSATION_BYTES,
            });
        }
        Ok(())
    }
    /// Check whether this exact resolved user-text allocation can be retained as one entry.
    ///
    /// Pass the same String to push_user after this check; capacity is included so the preflight
    /// and append enforce the same memory bound.
    ///
    /// # Errors
    /// Returns the exact capacity or accounting error that push_user would return.
    pub fn check_user_capacity(&self, text: &String) -> Result<(), ViewError> {
        self.check_conversation_capacity(text)
    }
    fn push(&mut self, kind: EntryKind, text: impl Into<String>) -> Result<(), ViewError> {
        let text = text.into();
        self.check_conversation_capacity(&text)?;
        self.conversation
            .try_reserve(1)
            .map_err(|_| ViewError::AllocationFailed)?;
        self.conversation.push(Entry { kind, text });
        Ok(())
    }
    pub fn push_user(&mut self, text: impl Into<String>) -> Result<(), ViewError> {
        self.push(EntryKind::User, text)
    }
    pub fn push_assistant(&mut self, text: impl Into<String>) -> Result<(), ViewError> {
        self.push(EntryKind::Assistant, text)
    }
    pub fn push_system(&mut self, text: impl Into<String>) -> Result<(), ViewError> {
        self.push(EntryKind::System, text)
    }
    /// Append a bounded nonfatal warning with a distinct presentation role.
    pub fn push_warning(&mut self, text: impl Into<String>) -> Result<(), ViewError> {
        self.push(EntryKind::Warning, text)
    }
    pub fn push_error(&mut self, text: impl Into<String>) -> Result<(), ViewError> {
        self.push(EntryKind::Error, text)
    }
    /// Append streamed text to the entry at `index`, keeping one response in one entry.
    ///
    /// # Errors
    /// Unknown entry, conversation byte limit or allocation failure; the view is unchanged.
    pub fn append_to_entry(&mut self, index: usize, delta: &str) -> Result<(), ViewError> {
        let current = self.conversation_bytes()?;
        let attempted = current
            .checked_add(delta.len())
            .ok_or(ViewError::AccountingOverflow)?;
        if attempted > MAX_CONVERSATION_BYTES {
            return Err(ViewError::ConversationBytes {
                limit: MAX_CONVERSATION_BYTES,
            });
        }
        let entry = self
            .conversation
            .get_mut(index)
            .ok_or(ViewError::AccountingOverflow)?;
        entry
            .text
            .try_reserve_exact(delta.len())
            .map_err(|_| ViewError::AllocationFailed)?;
        entry.text.push_str(delta);
        Ok(())
    }
    pub(crate) fn tool_metadata_bytes(&self) -> Result<usize, ViewError> {
        let base = self
            .tools
            .len()
            .checked_mul(core::mem::size_of::<ToolActivity>())
            .ok_or(ViewError::AccountingOverflow)?;
        self.tools.iter().try_fold(base, |total, tool| {
            let title = tool.title.as_ref().map_or(0, String::capacity);
            total
                .checked_add(title)
                .ok_or(ViewError::AccountingOverflow)
        })
    }
    pub(crate) fn add_tool_activity(&mut self, activity: ToolActivity) -> Result<(), ReduceError> {
        if self.tools.len() >= MAX_TOOL_ACTIVITIES {
            return Err(ReduceError::CapacityExceeded {
                resource: ProjectionResource::ToolActivities,
                limit: MAX_TOOL_ACTIVITIES,
            });
        }
        let current = self
            .tool_metadata_bytes()
            .map_err(|_| ReduceError::CapacityExceeded {
                resource: ProjectionResource::ToolMetadataBytes,
                limit: MAX_TOOL_METADATA_BYTES,
            })?;
        let attempted = current
            .checked_add(core::mem::size_of::<ToolActivity>())
            .ok_or(ReduceError::CapacityExceeded {
                resource: ProjectionResource::ToolMetadataBytes,
                limit: MAX_TOOL_METADATA_BYTES,
            })?;
        if attempted > MAX_TOOL_METADATA_BYTES {
            return Err(ReduceError::CapacityExceeded {
                resource: ProjectionResource::ToolMetadataBytes,
                limit: MAX_TOOL_METADATA_BYTES,
            });
        }
        self.tools
            .try_reserve(1)
            .map_err(|_| ReduceError::AllocationFailed)?;
        self.tools.push(activity);
        Ok(())
    }
    pub(crate) fn add_session_row(&mut self, row: SessionRow) -> Result<(), ReduceError> {
        if self.sessions.len() >= MAX_SESSION_ROWS {
            return Err(ReduceError::CapacityExceeded {
                resource: ProjectionResource::SessionRows,
                limit: MAX_SESSION_ROWS,
            });
        }
        self.sessions
            .try_reserve(1)
            .map_err(|_| ReduceError::AllocationFailed)?;
        self.sessions.push(row);
        Ok(())
    }
    /// Change conversation mode.
    pub const fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }
    /// Toggle conversation mode.
    pub const fn toggle_mode(&mut self) {
        self.mode = self.mode.toggle();
    }
    /// Remember an effort preference for the active model.
    pub fn set_effort(&mut self, effort: Effort) {
        self.effort = effort;
        let _previous = self
            .effort_preferences
            .insert(self.model.as_str().to_owned(), effort);
    }
    /// Switch models, restoring that model's preference or carrying over the current effort.
    pub fn set_model(&mut self, model: ModelId) {
        if let Some(effort) = self.effort_preferences.get(model.as_str()) {
            self.effort = *effort;
        }
        self.model = model;
        self.set_effort(self.effort);
    }
    /// Raise effort one step; return whether it changed.
    pub fn effort_up(&mut self) -> bool {
        let next = match self.effort {
            Effort::Low => Effort::Medium,
            Effort::Medium => Effort::High,
            Effort::High => Effort::XHigh,
            Effort::XHigh => Effort::Max,
            Effort::Max => return false,
        };
        self.set_effort(next);
        true
    }
    /// Lower effort one step; return whether it changed.
    pub fn effort_down(&mut self) -> bool {
        let next = match self.effort {
            Effort::Low => return false,
            Effort::Medium => Effort::Low,
            Effort::High => Effort::Medium,
            Effort::XHigh => Effort::High,
            Effort::Max => Effort::XHigh,
        };
        self.set_effort(next);
        true
    }
    /// Resolve the title of a known call.
    /// Resolve the title of a known call, rejecting titles that exceed the tool metadata budget.
    pub fn set_tool_title(&mut self, call: Cid, title: impl Into<String>) -> Result<(), ViewError> {
        let Some(index) = self.tools.iter().position(|tool| tool.call == call) else {
            return Ok(());
        };
        let title = title.into();
        let current = self.tool_metadata_bytes()?;
        let old = self
            .tools
            .get(index)
            .and_then(|tool| tool.title.as_ref())
            .map_or(0, String::capacity);
        let attempted = current
            .checked_sub(old)
            .and_then(|bytes| bytes.checked_add(title.capacity()))
            .ok_or(ViewError::AccountingOverflow)?;
        if attempted > MAX_TOOL_METADATA_BYTES {
            return Err(ViewError::ToolMetadataBytes {
                limit: MAX_TOOL_METADATA_BYTES,
            });
        }
        let tool = self
            .tools
            .get_mut(index)
            .ok_or(ViewError::AccountingOverflow)?;
        tool.title = Some(title);
        Ok(())
    }
    /// Whether the tool at `index` is expanded. Unknown entries return false.
    #[must_use]
    pub fn tool_expanded(&self, index: usize) -> bool {
        self.tools
            .get(index)
            .is_some_and(|tool| match tool.expanded {
                Some(expanded) => expanded,
                None => self.tools_expanded,
            })
    }
    /// Toggle one disclosure override; return false for an unknown entry.
    pub fn toggle_tool(&mut self, index: usize) -> bool {
        let expanded = self.tool_expanded(index);
        let Some(tool) = self.tools.get_mut(index) else {
            return false;
        };
        tool.expanded = Some(!expanded);
        true
    }
    /// Toggle the global setting and clear every per-entry override.
    pub fn toggle_all_tools(&mut self) {
        self.tools_expanded = !self.tools_expanded;
        for tool in &mut self.tools {
            tool.expanded = None;
        }
    }
    /// Replace the goal and start its injected clock.
    pub fn goal_start(&mut self, text: impl Into<String>, now_ms: u64) {
        self.goal = Some(GoalLine {
            text: text.into(),
            state: GoalState::Active,
            accumulated_ms: 0,
            active_since_ms: now_ms,
        });
    }
    /// Pause once, accumulating only elapsed active time.
    pub fn goal_pause(&mut self, now_ms: u64) {
        if let Some(goal) = &mut self.goal {
            if goal.state == GoalState::Active {
                goal.accumulated_ms = goal.active_ms(now_ms);
                goal.state = GoalState::Paused;
            }
        }
    }
    /// Resume a paused goal; repeated resumes do not reset its clock.
    pub fn goal_resume(&mut self, now_ms: u64) {
        if let Some(goal) = &mut self.goal {
            if goal.state == GoalState::Paused {
                goal.active_since_ms = now_ms;
                goal.state = GoalState::Active;
            }
        }
    }
    /// Remove the goal.
    pub fn goal_clear(&mut self) {
        self.goal = None;
    }
}
