//! Local agent activity model and explicitly labeled demo driver. No execution backend.
use crate::app::{App, Entry, EntryKind, Screen};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{collections::BTreeMap, time::Instant};
pub type AgentId = u64;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    Queued,
    Working,
    Waiting,
    Paused,
    Completed,
    Failed,
    Cancelled,
}
impl AgentStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Working => "Working",
            Self::Waiting => "Waiting",
            Self::Paused => "Paused",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Working | Self::Waiting | Self::Paused
        )
    }
}
#[derive(Clone, Debug)]
pub struct AgentRecord {
    pub id: AgentId,
    pub name: String,
    pub task: String,
    pub status: AgentStatus,
    pub started: Option<Instant>,
    pub elapsed_seconds: Option<u64>,
    pub tokens: Option<u64>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub transcript: Vec<Entry>,
    pub scroll: usize,
    pub tools_expanded: bool,
    pub tool_overrides: BTreeMap<usize, bool>,
}
impl AgentRecord {
    pub fn new(id: AgentId, name: &str, task: &str, status: AgentStatus) -> Self {
        Self {
            id,
            name: crate::app::safe_text(name).replace('\n', " "),
            task: crate::app::safe_text(task).replace('\n', " "),
            status,
            started: None,
            elapsed_seconds: None,
            tokens: None,
            model: None,
            effort: None,
            transcript: vec![],
            scroll: 0,
            tools_expanded: false,
            tool_overrides: BTreeMap::new(),
        }
    }
    pub fn elapsed(&self) -> Option<u64> {
        self.elapsed_seconds.or_else(|| {
            self.started
                .map(|s| crate::frame::now().saturating_duration_since(s).as_secs())
        })
    }
}
#[derive(Clone, Debug, Default)]
pub struct AgentActivity {
    pub records: Vec<AgentRecord>,
    pub viewed: Option<AgentId>,
    pub selected: Option<AgentId>,
    pub focused: bool,
    pub expanded: bool,
    pub opened: bool,
    pub manually_collapsed: bool,
    pub offset: usize,
    pub main_scroll: usize,
    pub demo: bool,
    pub demo_ticks: usize,
    pub animate_demo: bool,
    pub launch_entries: BTreeMap<usize, Vec<AgentId>>,
}
impl AgentActivity {
    pub fn get(&self, id: AgentId) -> Option<&AgentRecord> {
        self.records.iter().find(|a| a.id == id)
    }
    pub fn get_mut(&mut self, id: AgentId) -> Option<&mut AgentRecord> {
        self.records.iter_mut().find(|a| a.id == id)
    }
    pub fn active(&self) -> bool {
        self.records.iter().any(|a| a.status.active())
    }
    pub fn visible(&self) -> bool {
        !self.records.is_empty() || self.opened
    }
    pub fn insert(&mut self, record: AgentRecord) {
        let new_batch = !self.active() && record.status.active();
        if let Some(old) = self.get_mut(record.id) {
            *old = record;
        } else {
            self.records.push(record);
        }
        if new_batch {
            self.expanded = true;
            self.manually_collapsed = false;
        }
    }
    pub fn set_status(&mut self, id: AgentId, status: AgentStatus) {
        let was_active = self.active();
        if let Some(record) = self.get_mut(id) {
            if !status.active() && record.status.active() {
                record.elapsed_seconds = record.elapsed();
                record.started = None;
            }
            record.status = status;
        }
        if !was_active && self.active() {
            self.expanded = true;
            self.manually_collapsed = false;
        }
        if was_active && !self.active() {
            self.expanded = false;
            self.focused = false;
        }
    }
    pub fn summary(&self) -> String {
        if self.records.is_empty() {
            return "No subagents started".into();
        }
        let mut parts = vec![];
        for (status, label) in [
            (AgentStatus::Working, "working"),
            (AgentStatus::Queued, "queued"),
            (AgentStatus::Waiting, "waiting"),
            (AgentStatus::Paused, "paused"),
            (AgentStatus::Completed, "done"),
            (AgentStatus::Failed, "failed"),
            (AgentStatus::Cancelled, "cancelled"),
        ] {
            let n = self.records.iter().filter(|a| a.status == status).count();
            if n > 0 {
                parts.push(format!("{n} {label}"));
            }
        }
        parts.join(" / ")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentAction {
    Toggle,
    Open(Option<AgentId>),
    Focus,
}
impl App {
    pub fn visible_entries(&self) -> &[Entry] {
        self.agents
            .viewed
            .and_then(|id| self.agents.get(id))
            .map_or(&self.entries, |a| &a.transcript)
    }
    pub fn view_agent(&mut self, id: Option<AgentId>) {
        if id.is_some_and(|id| self.agents.get(id).is_none()) {
            return;
        }
        if self.agents.viewed != id {
            let scroll = self.conversation_scroll();
            if let Some(current) = self.agents.viewed {
                if let Some(a) = self.agents.get_mut(current) {
                    a.scroll = scroll;
                }
            } else {
                self.agents.main_scroll = scroll;
            }
            self.agents.viewed = id;
            self.scroll = id
                .and_then(|id| self.agents.get(id))
                .map_or(self.agents.main_scroll, |a| a.scroll);
            self.view_selection = None;
        }
        self.agents.selected = id;
        self.agents.focused = false;
    }
    pub fn open_agents(&mut self) {
        self.menu_dismissed = true;
        self.agents.opened = true;
        self.agents.expanded = true;
        self.agents.manually_collapsed = false;
        self.agents.focused = true;
        self.agents.selected = self.agents.viewed;
    }
    pub fn agents_key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::F(3)
            && self.screen == Screen::Home
            && self.preview.is_none()
            && !self.approval
        {
            if self.agents.focused {
                self.agents.focused = false;
            } else {
                self.open_agents();
            }
            return true;
        }
        if !self.agents.focused
            || self.screen != Screen::Home
            || self.preview.is_some()
            || self.approval
        {
            return false;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return false;
        }
        match key.code {
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {
                let pos = self
                    .agents
                    .selected
                    .and_then(|id| self.agents.records.iter().position(|a| a.id == id))
                    .map_or(0, |i| i + 1);
                let step = if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
                    3
                } else {
                    1
                };
                let next = if matches!(key.code, KeyCode::Up | KeyCode::PageUp) {
                    pos.saturating_sub(step)
                } else {
                    (pos + step).min(self.agents.records.len())
                };
                self.agents.selected = next
                    .checked_sub(1)
                    .and_then(|i| self.agents.records.get(i))
                    .map(|a| a.id);
            }
            KeyCode::Enter => self.view_agent(self.agents.selected),
            KeyCode::Left => {
                self.agents.expanded = false;
                self.agents.manually_collapsed = true;
                self.agents.focused = false;
            }
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => self.agents.focused = false,
            _ => {
                self.agents.focused = false;
                return false;
            }
        }
        true
    }
}
pub fn activate(app: &mut App, action: AgentAction) {
    match action {
        AgentAction::Toggle => {
            app.agents.opened = true;
            app.agents.expanded = !app.agents.expanded;
            app.agents.manually_collapsed = !app.agents.expanded;
            if !app.agents.expanded {
                app.agents.focused = false;
            }
        }
        AgentAction::Open(id) => app.view_agent(id),
        AgentAction::Focus => app.open_agents(),
    }
}
/// Explicit fixtures, not derived from enabling $subagents.
pub fn load_demo(app: &mut App, scenario: &str) {
    app.agents.demo = true;
    app.agents.animate_demo = scenario == "agents-working";
    if app.agents.animate_demo {
        app.working = true;
        app.working_since = Some(Instant::now());
    }
    let rows = if scenario == "agents-many" {
        9
    } else if scenario == "agents-mixed" {
        7
    } else {
        2
    };
    for i in 0..rows {
        let (name, task) = match i {
            0 => (
                "Registry".to_owned(),
                "Implement storage registry".to_owned(),
            ),
            1 => ("RTK tools".into(), "Implement tool registry".into()),
            _ => (
                format!("Review {}", i - 1),
                format!("Review implementation area {}", i - 1),
            ),
        };
        let status = if scenario == "agents-completed" {
            AgentStatus::Completed
        } else if scenario == "agents-mixed" {
            [
                AgentStatus::Working,
                AgentStatus::Waiting,
                AgentStatus::Paused,
                AgentStatus::Queued,
                AgentStatus::Completed,
                AgentStatus::Failed,
                AgentStatus::Cancelled,
            ][i]
        } else {
            AgentStatus::Working
        };
        let mut a = AgentRecord::new(i as u64 + 1, &name, &task, status);
        a.elapsed_seconds = Some(12 + i as u64 * 3);
        a.tokens = if i == 1 {
            None
        } else {
            Some(12400 + i as u64 * 3100)
        };
        a.model = Some("Nemotron Super 49B v1.5".into());
        a.effort = Some("high".into());
        a.transcript = vec![
            Entry::new(
                EntryKind::Note,
                "Agent task [demo]",
                &[
                    &task,
                    "Local UI preview / no agent process or provider request.",
                ],
            ),
            Entry::new(
                EntryKind::Tool,
                "Read / registry.rs [demo]",
                &[
                    "Inspecting the registry interface.",
                    "Checking shared validation boundaries.",
                ],
            ),
            Entry::new(
                EntryKind::Assistant,
                "",
                &[
                    &format!("{name} activity [demo]"),
                    "The registry should expose typed entries and validate names at its boundary.",
                    "",
                    "Suggested checks:",
                    "1. Duplicate names return a clear error.",
                    "2. Unknown entries remain distinguishable from empty ones.",
                ],
            ),
        ];
        if status == AgentStatus::Failed {
            a.transcript.push(Entry::new(
                EntryKind::Error,
                "Sample failure [demo]",
                &["This is a labeled failure fixture; no backend request failed."],
            ));
        }
        app.agents.insert(a);
    }
    app.entries.push(Entry::new(
        EntryKind::Assistant,
        "",
        &[
            "Splitting the registry work into independent tasks. [demo]",
            "This is a scripted activity preview; no agents are executed.",
        ],
    ));
    let ids = app.agents.records.iter().map(|a| a.id).collect::<Vec<_>>();
    let task_lines = app
        .agents
        .records
        .iter()
        .map(|a| format!("{} / {}", a.name, a.task))
        .collect::<Vec<_>>();
    let mut entry = Entry::new(
        EntryKind::AgentLaunch,
        &format!("{} agents launched [demo]", ids.len()),
        &[],
    );
    entry.lines = task_lines;
    app.agents.launch_entries.insert(app.entries.len(), ids);
    app.entries.push(entry);
    if !app.agents.active() {
        app.agents.expanded = false;
    }
}
pub fn tick_demo(app: &mut App) {
    if !app.agents.demo || !app.agents.animate_demo || !app.agents.active() {
        return;
    }
    app.agents.demo_ticks += 1;
    if app.agents.demo_ticks.is_multiple_of(5) {
        for a in &mut app.agents.records {
            if a.status == AgentStatus::Working {
                a.elapsed_seconds = Some(a.elapsed_seconds.unwrap_or(0) + 1);
            }
        }
    }
    for (tick, id) in [(40, 1), (80, 2)] {
        if app.agents.demo_ticks == tick {
            app.agents.set_status(id, AgentStatus::Completed);
            if let Some(a) = app.agents.get_mut(id) {
                a.transcript.push(Entry::new(
                    EntryKind::Completion,
                    "",
                    &["Task complete [demo] / no files changed."],
                ));
            }
        }
    }
}
