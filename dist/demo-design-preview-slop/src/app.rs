use crate::{
    completion::{CompletionKind, FileIndex, active_token},
    composer::{Attachment, AttachmentPayload, Composer, layout_rows},
    interaction::ViewSelection,
};
use crate::{
    models::NVIDIA_MODELS,
    theme::{Theme, ThemeMode},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeMap;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub const EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Home,
    Preview,
    Settings,
    Models,
    Providers,
    ProviderEdit,
    ProviderPicker,
    Sessions,
    Status,
    Usage,
    Stats,
    Diff,
    Transcript,
    Onboarding,
    Help,
}
impl Screen {
    pub fn title(self) -> &'static str {
        match self {
            Self::Home => "Conversation",
            Self::Preview => "Preview gallery",
            Self::Settings => "Settings",
            Self::Models => "Choose a model",
            Self::Providers => "Providers",
            Self::ProviderEdit => "Provider configuration",
            Self::ProviderPicker => "Choose provider",
            Self::Sessions => "Session history",
            Self::Status => "Session status",
            Self::Usage => "Usage",
            Self::Stats => "Statistics",
            Self::Diff => "Review changes",
            Self::Transcript => "Transcript",
            Self::Onboarding => "Welcome to polaris",
            Self::Help => "Help & shortcuts",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProviderDescriptor {
    pub id: String,
    pub label: String,
    pub endpoint: String,
    pub credential_source: String,
    pub models: Vec<String>,
    pub configured: bool,
}
impl ProviderDescriptor {
    pub fn state(&self) -> &'static str {
        if self.configured {
            "Preview configured"
        } else {
            "Not configured"
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    User,
    Assistant,
    Tool,
    Error,
    Warning,
    Completion,
    Note,
    AgentLaunch,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub kind: EntryKind,
    pub title: String,
    pub lines: Vec<String>,
    pub draft: Option<Composer>,
}
impl Entry {
    pub fn new(kind: EntryKind, title: &str, lines: &[&str]) -> Self {
        Self {
            kind,
            draft: None,
            title: title.into(),
            lines: lines.iter().map(|s| (*s).into()).collect(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Editor {
    pub text: String,
    pub cursor: usize,
}
impl Editor {
    pub fn set(&mut self, text: &str) {
        self.text = safe_text(text);
        self.cursor = self.text.len();
    }
    pub fn insert(&mut self, text: &str) {
        let clean = safe_text(text);
        // Bound pasted input so the visual preview cannot allocate an unbounded composer.
        let room = 32_768usize.saturating_sub(self.text.len());
        let mut end = room.min(clean.len());
        while !clean.is_char_boundary(end) {
            end -= 1;
        }
        self.text.insert_str(self.cursor, &clean[..end]);
        self.cursor += end;
    }
    pub fn left(&mut self) {
        self.cursor = self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i);
    }
    pub fn right(&mut self) {
        if let Some(ch) = self.text[self.cursor..].chars().next() {
            self.cursor += ch.len_utf8();
        }
    }
    pub fn backspace(&mut self) {
        let end = self.cursor;
        self.left();
        self.text.replace_range(self.cursor..end, "");
    }
    pub fn delete(&mut self) {
        let start = self.cursor;
        self.right();
        let end = self.cursor;
        self.text.replace_range(start..end, "");
        self.cursor = start;
    }
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }
    pub fn key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.len(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&ch.to_string())
            }
            _ => {}
        }
    }
}

pub fn safe_text(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect()
}

pub const COMMANDS: &[(&str, &str)] = &[
    ("/preview", "Explore every screen and state"),
    ("/model", "Choose provider and model"),
    (
        "/settings",
        "Appearance, providers, interaction, accessibility",
    ),
    ("/sessions", "Browse demo session history"),
    ("/status", "View the current session"),
    ("/usage", "Session usage and provider limits"),
    ("/agents", "Inspect agent activity and conversations"),
    ("/stats", "Activity and statistics"),
    ("/diff", "Review a demo patch"),
    ("/providers", "Configure providers in memory"),
    ("/onboarding", "Walk through first-run setup"),
    ("/help", "Commands and keyboard shortcuts"),
    ("/clear", "Start a fresh preview conversation"),
    ("/quit", "Exit the preview"),
];
pub const SCENARIOS: &[(&str, &str)] = &[
    ("welcome", "Clean first-run conversation"),
    ("conversation", "Completed coding conversation"),
    ("working", "Simulated activity and tool previews"),
    ("agents-working", "Active parallel agents [demo]"),
    ("agents-mixed", "Agent states and failures [demo]"),
    ("agents-completed", "Completed agent summary [demo]"),
    ("agents-many", "Scrollable agent list [demo]"),
    ("messages", "Ordinary, warning and error styles [demo]"),
    ("approval", "Inline action approval"),
    ("diff", "Unified patch viewer"),
    ("error", "Failure with a preserved draft"),
    ("onboarding", "First-run provider setup"),
    ("models", "Provider-neutral model selection"),
    ("providers", "Provider registry and configuration"),
    ("settings", "Theme and interaction preferences"),
    ("sessions", "Searchable demo session history"),
    ("status", "Workspace and connection status"),
    ("usage", "Usage with unavailable metrics"),
    ("usage-demo", "Labeled sample usage"),
    ("stats", "Statistics empty state"),
    ("stats-stacked", "Stacked model timeline / demo"),
    ("stats-lines", "Combined model lines / demo"),
    ("stats-trends", "Individual model trends / demo"),
    ("stats-details", "Expanded token breakdown / demo"),
    ("stats-demo", "Labeled sample activity"),
    ("transcript", "Expanded tool output and search"),
    ("help", "Commands and shortcuts"),
];
pub const SESSIONS: &[(&str, &str, &str)] = &[
    (
        "Refactor request validation",
        "NVIDIA NIM",
        "2026-10-18 09:42 · demo",
    ),
    (
        "Explain the streaming parser",
        "Anthropic",
        "2026-10-17 16:08 · demo",
    ),
    (
        "Add keyboard navigation",
        "OpenAI",
        "2026-10-17 11:24 · demo",
    ),
    (
        "Review local inference config",
        "Local",
        "2026-10-16 14:31 · demo",
    ),
];

#[derive(Clone, Debug)]
struct Navigation {
    screen: Screen,
    selected: usize,
    scroll: usize,
    query: Editor,
    tab: usize,
}

#[derive(Debug)]
pub struct App {
    pub screen: Screen,
    pub theme: Theme,
    pub ascii: bool,
    pub reduce_motion: bool,
    pub input: Composer,
    pub file_index: Option<FileIndex>,
    pub preview: Option<Arc<Attachment>>,
    pub preview_scroll: usize,
    pub view_selection: Option<ViewSelection>,
    pub toast: Option<(String, Instant)>,
    pub mouse_position: Option<(u16, u16)>,
    pub clipboard_actions: Vec<ClipboardAction>,
    pub clipboard_pending: BTreeMap<u64, ClipboardPurpose>,
    pub submissions: Vec<Submission>,
    pub prompt_settings: crate::prompt_settings::PromptSettings,
    pub editor_width: usize,
    pub composer_scroll: usize,
    pub attachments: BTreeMap<u64, Arc<Attachment>>,
    next_attachment: u64,
    next_image: u64,
    next_clipboard: u64,
    pub query: Editor,
    pub selected: usize,
    pub scroll: usize,
    pub tab: usize,
    pub providers: Vec<ProviderDescriptor>,
    pub active_provider: usize,
    pub model_provider: usize,
    pub account_email: Option<String>,
    pub active_model: Option<String>,
    pub editing_provider: usize,
    pub form: Vec<Editor>,
    pub entries: Vec<Entry>,
    pub agents: crate::agents::AgentActivity,
    pub working: bool,
    pub working_since: Option<Instant>,
    pub goal: Option<GoalProgress>,
    pub ticks: usize,
    pub approval: bool,
    pub approval_selected: usize,
    pub notice: String,
    pub workspace: String,
    pub metrics_demo: bool,
    pub usage: crate::usage::UsageViewState,
    pub preferences: crate::preferences::PreferenceState,
    pub settings: crate::settings::SettingsViewState,
    pub stats: crate::stats::StatsViewState,
    pub effort: usize,
    model_efforts: BTreeMap<(String, String), usize>,
    pub expanded: bool,
    pub tool_details_expanded: bool,
    tool_details_overrides: BTreeMap<usize, bool>,
    pub mode: usize,
    pub quit: bool,
    pub exit_armed: bool,
    pub onboarding_step: usize,
    pub menu_dismissed: bool,
    stack: Vec<Navigation>,
    history: Vec<Composer>,
    history_index: usize,
    history_draft: Option<Composer>,
}

/// Local preview goal activity; no backend goal runner is implied.
#[derive(Clone, Debug)]
pub struct GoalProgress {
    pub running_since: Option<Instant>,
    pub accumulated: Duration,
}
impl GoalProgress {
    pub fn elapsed(&self) -> Duration {
        self.accumulated
            + self.running_since.map_or(Duration::ZERO, |s| {
                crate::frame::now().saturating_duration_since(s)
            })
    }
    pub fn is_active(&self) -> bool {
        self.running_since.is_some()
    }
    fn resume(&mut self) {
        if self.running_since.is_none() {
            self.running_since = Some(Instant::now());
        }
    }
    fn pause(&mut self) {
        if let Some(since) = self.running_since.take() {
            self.accumulated += since.elapsed();
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new("C:\\mDev\\nvvi-code".into())
    }
}
impl App {
    pub fn new(workspace: String) -> Self {
        let providers = [
            ("nvidia-nim", "NVIDIA NIM"),
            ("anthropic", "Anthropic"),
            ("openai", "OpenAI"),
            ("local", "Local"),
        ]
        .iter()
        .map(|(id, label)| ProviderDescriptor {
            id: (*id).into(),
            label: (*label).into(),
            endpoint: String::new(),
            credential_source: String::new(),
            models: vec![],
            configured: false,
        })
        .collect();
        Self {
            screen: Screen::Home,
            theme: Theme::default(),
            ascii: false,
            reduce_motion: false,
            input: Composer::default(),
            file_index: None,
            preview: None,
            preview_scroll: 0,
            view_selection: None,
            toast: None,
            mouse_position: None,
            clipboard_actions: vec![],
            clipboard_pending: BTreeMap::new(),
            submissions: vec![],
            prompt_settings: crate::prompt_settings::PromptSettings::default(),
            editor_width: 70,
            composer_scroll: 0,
            attachments: BTreeMap::new(),
            next_attachment: 1,
            next_image: 1,
            next_clipboard: 1,
            query: Editor::default(),
            selected: 0,
            scroll: 0,
            tab: 0,
            providers,
            active_provider: 0,
            model_provider: 0,
            account_email: None,
            active_model: Some(NVIDIA_MODELS[0].id.into()),
            editing_provider: 0,
            form: vec![],
            entries: vec![],
            agents: crate::agents::AgentActivity::default(),
            working: false,
            working_since: None,
            goal: None,
            ticks: 0,
            approval: false,
            approval_selected: 1,
            notice: String::new(),
            workspace: safe_text(&workspace),
            metrics_demo: false,
            usage: crate::usage::UsageViewState::default(),
            preferences: crate::preferences::PreferenceState::default(),
            settings: crate::settings::SettingsViewState::default(),
            stats: crate::stats::StatsViewState::default(),
            effort: 2,
            model_efforts: BTreeMap::new(),
            expanded: true,
            tool_details_expanded: false,
            tool_details_overrides: BTreeMap::new(),
            mode: 0,
            quit: false,
            exit_armed: false,
            onboarding_step: 0,
            menu_dismissed: false,
            stack: vec![],
            history: vec![],
            history_index: 0,
            history_draft: None,
        }
    }
    pub fn provider(&self) -> &ProviderDescriptor {
        &self.providers[self.active_provider]
    }
    pub fn model_label(&self) -> &str {
        self.active_model.as_deref().unwrap_or("no model")
    }
    pub fn mode_label(&self) -> &'static str {
        ["auto", "plan"][self.mode]
    }
    pub fn mode_status(&self) -> &'static str {
        ["auto mode on", "plan mode on"][self.mode]
    }
    pub fn effort_label(&self) -> &'static str {
        EFFORTS[self.effort.min(EFFORTS.len() - 1)]
    }
    fn effort_key(&self) -> Option<(String, String)> {
        self.model_rows()
            .get(self.selected)
            .and_then(|(provider, model)| {
                model
                    .as_ref()
                    .map(|model| (self.providers[*provider].id.clone(), model.clone()))
            })
    }
    pub fn picker_effort(&self) -> usize {
        self.effort_key()
            .and_then(|key| self.model_efforts.get(&key).copied())
            .unwrap_or(2)
    }
    fn stepped_effort(current: usize, increase: bool) -> usize {
        let current = current.min(EFFORTS.len() - 1);
        if increase {
            (current + 1).min(EFFORTS.len() - 1)
        } else {
            current.saturating_sub(1)
        }
    }
    fn adjust_active_effort(&mut self, increase: bool) {
        if let Some(model) = self.active_model.clone() {
            self.effort = Self::stepped_effort(self.effort, increase);
            self.model_efforts
                .insert((self.provider().id.clone(), model), self.effort);
        }
    }
    fn adjust_effort(&mut self, increase: bool) {
        if let Some(key) = self.effort_key() {
            let next = Self::stepped_effort(self.picker_effort(), increase);
            self.model_efforts.insert(key, next);
        }
    }
    pub fn filtered_commands(&self) -> Vec<(&'static str, &'static str)> {
        COMMANDS
            .iter()
            .copied()
            .filter(|(name, _)| name.starts_with(&self.input.text))
            .collect()
    }
    pub fn menu_open(&self) -> bool {
        self.screen == Screen::Home
            && self.input.text.starts_with('/')
            && !self.input.text.contains(char::is_whitespace)
            && !self.menu_dismissed
            && !self.approval
    }
    pub fn filtered_sessions(&self) -> Vec<usize> {
        SESSIONS
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                format!("{} {}", s.0, s.1)
                    .to_lowercase()
                    .contains(&self.query.text.to_lowercase())
            })
            .map(|(i, _)| i)
            .collect()
    }
    pub fn models_for_provider(&self, provider: usize) -> Vec<String> {
        let mut models = self.providers[provider].models.clone();
        if self.providers[provider].id == "nvidia-nim" {
            for model in NVIDIA_MODELS {
                if !models.iter().any(|id| id == model.id) {
                    models.push(model.id.into());
                }
            }
        }
        models
    }
    pub fn model_rows(&self) -> Vec<(usize, Option<String>)> {
        let models = self.models_for_provider(self.model_provider);
        if models.is_empty() {
            return vec![(self.model_provider, None)];
        }
        models
            .into_iter()
            .map(|model| (self.model_provider, Some(model)))
            .collect()
    }
    pub fn conversation_scroll(&self) -> usize {
        if self.screen == Screen::Home {
            self.scroll
        } else {
            self.stack
                .iter()
                .find(|nav| nav.screen == Screen::Home)
                .map_or(0, |nav| nav.scroll)
        }
    }
    pub fn warning_messages(&self) -> Vec<&'static str> {
        let mut warnings = Vec::new();
        if !self.provider().configured {
            warnings.push("Provider is not configured");
        }
        if self.active_model.is_none() {
            warnings.push("No model selected");
        }
        warnings
    }
    pub fn open(&mut self, screen: Screen) {
        self.stack.push(Navigation {
            screen: self.screen,
            selected: self.selected,
            scroll: self.scroll,
            query: self.query.clone(),
            tab: self.tab,
        });
        self.screen = screen;
        if screen == Screen::Stats {
            self.stats = crate::stats::StatsViewState::default();
        }
        if screen == Screen::Settings {
            self.settings = crate::settings::SettingsViewState::default();
        }
        if screen == Screen::Models {
            self.model_provider = self.active_provider;
        }
        self.selected = if screen == Screen::Models {
            self.models_for_provider(self.model_provider)
                .iter()
                .position(|model| Some(model) == self.active_model.as_ref())
                .unwrap_or(0)
        } else {
            0
        };
        self.scroll = 0;
        self.query.clear();
        self.tab = 0;
        self.notice.clear();
    }
    pub fn close(&mut self) {
        if self.screen == Screen::Stats {
            self.stats.preview = None;
        }
        if self.screen == Screen::Settings {
            self.settings.highlighted_value = None;
        }
        if let Some(nav) = self.stack.pop() {
            self.screen = nav.screen;
            self.selected = nav.selected;
            self.scroll = nav.scroll;
            self.query = nav.query;
            self.tab = nav.tab;
        } else {
            self.screen = Screen::Home;
            self.selected = 0;
            self.scroll = 0;
            self.query.clear();
        }
        self.notice.clear();
    }
    pub fn home(&mut self) {
        self.screen = Screen::Home;
        self.stack.clear();
        self.selected = 0;
        self.scroll = 0;
        self.query.clear();
    }
    pub fn add(&mut self, entry: Entry) {
        self.entries.push(entry);
    }
    pub fn command(&mut self, command: &str) {
        if command == "/agents" {
            self.open_agents();
            return;
        }
        let screen = match command {
            "/preview" => Some(Screen::Preview),
            "/settings" => Some(Screen::Settings),
            "/model" => Some(Screen::Models),
            "/providers" => Some(Screen::Providers),
            "/sessions" => Some(Screen::Sessions),
            "/status" => Some(Screen::Status),
            "/usage" => Some(Screen::Usage),
            "/stats" => Some(Screen::Stats),
            "/diff" => Some(Screen::Diff),
            "/onboarding" => Some(Screen::Onboarding),
            "/help" => Some(Screen::Help),
            _ => None,
        };
        if let Some(screen) = screen {
            self.open(screen);
        } else if command == "/quit" {
            self.quit = true;
        } else if command == "/clear" {
            self.load_scenario("welcome").expect("known scenario");
            self.attachments.clear();
            self.submissions.clear();
            self.history.clear();
            self.history_index = 0;
            self.history_draft = None;
            self.next_attachment = 1;
            self.next_image = 1;
            self.preview = None;
            self.view_selection = None;
            self.notice.clear();
        } else {
            self.notice = format!("Unknown command: {command}. Use /help.");
        }
    }
    pub fn submit(&mut self) {
        if self.input.text.trim().is_empty() {
            return;
        }
        let draft = self.input.submission_snapshot();
        let text = draft.text.trim().to_string();
        let is_command =
            COMMANDS.iter().any(|(command, _)| *command == text) && draft.spans.is_empty();
        let mut request = parse_submission(&draft);
        if self.approval || (self.working && !is_command && !request.settings_only) {
            return;
        }
        if !is_command && !request.settings_errors.is_empty() {
            self.notice = request.settings_errors.join(" · ");
            return;
        }
        self.input.clear();
        self.menu_dismissed = false;
        self.selected = 0;
        self.history.push(draft.clone());
        self.history_index = self.history.len();
        self.history_draft = None;
        self.exit_armed = false;
        if is_command {
            self.command(&text);
            return;
        }
        self.prompt_settings.apply_changes(&request.settings);
        request.settings = self.prompt_settings.clone();
        if request.settings_only {
            self.notice =
                "Settings updated for this preview session. Services are not connected.".into();
            return;
        }
        self.view_agent(None);
        self.scroll = 0;
        self.add(Entry::new(
            EntryKind::User,
            "you",
            &text.lines().collect::<Vec<_>>(),
        ));
        if let Some(entry) = self.entries.last_mut() {
            entry.draft = Some(draft.clone());
        }
        if request.goal && self.goal.is_none() {
            self.goal = Some(GoalProgress {
                running_since: None,
                accumulated: Duration::ZERO,
            });
        }
        if let Some(goal) = &mut self.goal {
            goal.resume();
        }
        if request.planning {
            self.mode = 1;
        }
        self.submissions.push(request);
        self.working = true;
        self.working_since = Some(Instant::now());
        self.ticks = 0;
        self.notice.clear();
    }
    pub fn tick(&mut self) {
        crate::agents::tick_demo(self);
        if !self.working {
            return;
        }
        self.ticks += 1;
        if self.ticks == 5 {
            self.add(Entry::new(
                EntryKind::Tool,
                "Read · src/validation.rs · complete [demo]",
                &[
                    "Inspecting the request validation module.",
                    "Found a repeated guard in two handlers.",
                    "Preparing a small shared helper.",
                    "Additional simulated output line 4.",
                    "Additional simulated output line 5.",
                    "Additional simulated output line 6.",
                ],
            ));
        }
        if self.ticks >= 18 {
            let elapsed = self.working_since.map(|since| since.elapsed());
            self.working = false;
            self.working_since = None;
            if let Some(goal) = &mut self.goal {
                goal.pause();
            }
            self.add(Entry::new(
                EntryKind::Assistant,
                "polaris · demo response",
                &[
                    "The repeated validation can move into a shared helper.",
                    "",
                    "Suggested approach:",
                    "1. Extract the request guard.",
                    "2. Keep error messages at the handler boundary.",
                    "3. Add cases for missing and malformed input.",
                    "",
                    "This is a scripted UI demonstration. No files were read or changed.",
                    "Open /diff to review the sample patch.",
                ],
            ));
            if let Some(elapsed) = elapsed {
                self.add(Entry::new(
                    EntryKind::Completion,
                    "",
                    &[&crate::work_summary::completed_line(elapsed)],
                ));
            }
        }
    }
    fn interrupt_work(&mut self) {
        if self.working {
            if let Some(goal) = &mut self.goal {
                goal.pause();
            }
            self.working = false;
            self.working_since = None;
            self.add(Entry::new(
                EntryKind::Note,
                "Interrupted [demo]",
                &["Simulated operation stopped. No files were changed."],
            ));
        }
    }
    pub fn activity(&self) -> &'static str {
        if self.ticks < 5 {
            "Inspecting workspace [simulated]"
        } else if self.ticks < 11 {
            "Reading source [simulated]"
        } else {
            "Preparing response [simulated]"
        }
    }
    pub fn tool_expanded(&self, index: usize) -> bool {
        if let Some(agent) = self.agents.viewed.and_then(|id| self.agents.get(id)) {
            return agent
                .tool_overrides
                .get(&index)
                .copied()
                .unwrap_or(agent.tools_expanded);
        }
        self.tool_details_overrides
            .get(&index)
            .copied()
            .unwrap_or(self.tool_details_expanded)
    }
    pub fn toggle_tool_details(&mut self, index: usize) {
        if let Some(agent) = self.agents.viewed.and_then(|id| self.agents.get_mut(id)) {
            if agent
                .transcript
                .get(index)
                .is_some_and(|e| e.kind == EntryKind::Tool)
            {
                let expanded = agent
                    .tool_overrides
                    .get(&index)
                    .copied()
                    .unwrap_or(agent.tools_expanded);
                agent.tool_overrides.insert(index, !expanded);
                self.view_selection = None;
            }
            return;
        }
        if self
            .entries
            .get(index)
            .is_some_and(|entry| entry.kind == EntryKind::Tool)
        {
            self.tool_details_overrides
                .insert(index, !self.tool_expanded(index));
            self.view_selection = None;
        }
    }
    pub fn working_elapsed(&self) -> Duration {
        if self.working {
            self.working_since.map_or(Duration::ZERO, |since| {
                crate::frame::now().saturating_duration_since(since)
            })
        } else {
            Duration::ZERO
        }
    }
    pub fn start_provider_edit(&mut self, index: usize) {
        self.editing_provider = index;
        let p = &self.providers[index];
        self.form = [
            p.label.clone(),
            p.endpoint.clone(),
            p.credential_source.clone(),
            p.models.first().cloned().unwrap_or_default(),
        ]
        .iter()
        .map(|v| {
            let mut e = Editor::default();
            e.set(v);
            e
        })
        .collect();
        self.open(Screen::ProviderEdit);
    }
    pub fn save_provider(&mut self) {
        if self.form[0].text.trim().is_empty() {
            self.notice = "A provider label is required for the preview.".into();
            return;
        }
        if self.form[3].text.trim().is_empty() {
            self.notice = "Enter a model label or identifier; no model is inferred.".into();
            self.selected = 3;
            return;
        }
        let p = &mut self.providers[self.editing_provider];
        p.label = self.form[0].text.trim().into();
        p.endpoint = self.form[1].text.trim().into();
        p.credential_source = self.form[2].text.trim().into();
        p.models = vec![self.form[3].text.trim().into()];
        p.configured = true;
        if self.active_provider == self.editing_provider {
            self.active_model = p.models.first().cloned();
        }
        self.close();
        self.notice =
            "Saved for this preview only. Endpoint and credentials have not been validated.".into();
    }
    pub fn load_scenario(&mut self, name: &str) -> Result<(), String> {
        if name != "usage-bars" && !SCENARIOS.iter().any(|(n, _)| *n == name) {
            return Err(format!("Unknown scenario: {name}"));
        }
        self.home();
        self.entries.clear();
        self.agents = crate::agents::AgentActivity::default();
        self.tool_details_overrides.clear();
        self.input.clear();
        self.history_index = self.history.len();
        self.history_draft = None;
        self.notice.clear();
        self.working = false;
        self.goal = None;
        self.approval = false;
        self.metrics_demo = false;
        self.stats = crate::stats::StatsViewState::default();
        self.ticks = 0;
        self.onboarding_step = 0;
        match name {
            "welcome" => {}
            "agents-working" | "agents-mixed" | "agents-completed" | "agents-many" => {
                crate::agents::load_demo(self, name)
            }
            "conversation" | "working" | "transcript" => {
                self.entries = conversation_fixture();
                if name == "working" {
                    self.entries.truncate(1);
                    self.working = true;
                    self.working_since = Some(Instant::now());
                }
                if name == "transcript" {
                    self.open(Screen::Transcript);
                }
            }
            "approval" => {
                self.entries.push(Entry::new(
                    EntryKind::User,
                    "you · demo",
                    &["Extract the shared validation helper."],
                ));
                self.approval = true;
                self.approval_selected = 1;
            }
            "messages" => {
                self.entries.push(Entry::new(
                    EntryKind::Assistant,
                    "",
                    &[
                        "Ordinary message [demo]",
                        "This preview shows message styles; no provider request was made.",
                    ],
                ));
                self.entries.push(Entry::new(
                    EntryKind::Warning,
                    "Preview notice [demo]",
                    &["This is a sample warning. Your draft remains available."],
                ));
                self.entries.push(Entry::new(
                    EntryKind::Error,
                    "Preview failure [demo]",
                    &["This is a sample error, not an actual provider failure."],
                ));
            }
            "error" => {
                self.entries.push(Entry::new(EntryKind::Error,"Connection unavailable [demo]",&[
                "The simulated provider did not respond. Your draft is preserved.","Open /providers to inspect configuration, or submit again to retry the demo."]));
                self.input.set("Continue reviewing the validation helper");
            }
            "models" => self.open(Screen::Models),
            "usage-demo" => {
                self.metrics_demo = true;
                self.open(Screen::Usage);
            }
            "usage-bars" => {
                self.open(Screen::Settings);
                self.tab = 1;
            }
            "stats-demo" | "stats-stacked" | "stats-lines" | "stats-trends" | "stats-details" => {
                self.open(Screen::Stats);
                self.stats.demo = true;
                if name != "stats-demo" {
                    self.stats.tab = crate::stats::StatsTab::Models;
                }
                self.stats.preview = match name {
                    "stats-lines" => Some(crate::stats::ChartStyle::Lines),
                    "stats-trends" => Some(crate::stats::ChartStyle::Trends),
                    _ => None,
                };
                self.stats.details = name == "stats-details";
            }
            other => {
                self.command(&format!("/{other}"));
            }
        }
        Ok(())
    }
    fn list_len(&self) -> usize {
        match self.screen {
            Screen::Preview => SCENARIOS.len(),
            Screen::Models => self.model_rows().len() + 1,
            Screen::ProviderPicker => self.providers.len(),
            Screen::Providers => self.providers.len() + 1,
            Screen::Sessions => self.filtered_sessions().len(),
            Screen::ProviderEdit => 5,
            Screen::Settings => crate::settings::row_count(self.tab),
            _ => 1,
        }
    }
    pub fn paste(&mut self, text: &str) {
        match self.screen {
            Screen::Home => {
                self.paste_text(text);
                self.menu_dismissed = false;
                self.selected = 0;
            }
            Screen::ProviderEdit if self.selected < 4 => {
                self.form[self.selected].insert(&text.replace('\n', " "))
            }
            Screen::Sessions | Screen::Transcript | Screen::Help => {
                self.query.insert(&text.replace('\n', " "));
                self.scroll = 0;
                self.selected = 0;
            }
            _ => {}
        }
    }
    pub fn key(&mut self, key: KeyEvent) {
        let key = crate::input::normalize_key(key, false);
        if key.kind == crossterm::event::KeyEventKind::Release {
            return;
        }
        if self.screen == Screen::Home
            && self.preview.is_none()
            && !self.approval
            && !self.agents.focused
            && key.modifiers == KeyModifiers::SHIFT
            && matches!(key.code, KeyCode::Up | KeyCode::Down)
        {
            self.adjust_active_effort(key.code == KeyCode::Up);
            self.exit_armed = false;
            return;
        }
        if self.rich_key(key) {
            return;
        }
        if self.agents_key(key) {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => {
                    if self.working {
                        self.interrupt_work();
                    } else if self.screen != Screen::Home {
                        self.close();
                    } else if self.approval {
                        self.resolve_approval(false);
                    } else if !self.input.text.is_empty() {
                        self.input.clear();
                        self.history_index = self.history.len();
                        self.history_draft = None;
                        self.notice = "Draft cleared. Ctrl+C again for exit confirmation.".into();
                    } else if self.exit_armed {
                        self.quit = true;
                    } else {
                        self.exit_armed = true;
                        self.notice =
                            "Exit preview? Press Ctrl+C again; any other key cancels.".into();
                    }
                    return;
                }
                KeyCode::Char('o') if self.screen == Screen::Home => {
                    if let Some(agent) = self.agents.viewed.and_then(|id| self.agents.get_mut(id)) {
                        agent.tools_expanded = !agent.tools_expanded;
                        agent.tool_overrides.clear();
                    } else {
                        self.tool_details_expanded = !self.tool_details_expanded;
                        self.tool_details_overrides.clear();
                    }
                    // Inline expansion changes conversation source offsets, not the draft.
                    self.view_selection = None;
                    return;
                }
                KeyCode::Char('o') if self.screen == Screen::Transcript => {
                    self.expanded = !self.expanded;
                    return;
                }
                KeyCode::Char('s') if self.screen == Screen::ProviderEdit => {
                    self.save_provider();
                    return;
                }
                KeyCode::Char('d') if self.input.text.is_empty() && self.screen == Screen::Home => {
                    self.quit = true;
                    return;
                }
                _ => {}
            }
        }
        self.exit_armed = false;
        if key.code == KeyCode::F(2) {
            self.open(Screen::Status);
            return;
        }
        if self.screen == Screen::Models {
            match key.code {
                KeyCode::Char('p') => {
                    self.open(Screen::ProviderPicker);
                    return;
                }
                KeyCode::Char('c') => {
                    self.start_provider_edit(self.model_provider);
                    return;
                }
                KeyCode::Left | KeyCode::Right => {
                    self.adjust_effort(key.code == KeyCode::Right);
                    return;
                }
                KeyCode::Tab => {
                    self.selected = self.model_rows().len();
                    return;
                }
                _ => {}
            }
        }
        if key.code == KeyCode::Esc {
            if self.screen != Screen::Home {
                self.close();
            } else if self.completion_open() {
                self.menu_dismissed = true;
            } else if self.approval {
                self.resolve_approval(false);
            } else if self.agents.viewed.is_some() {
                self.view_agent(None);
            } else if self.working {
                self.interrupt_work();
            } else {
                self.notice.clear();
            }
            return;
        }
        if self.screen == Screen::Home {
            self.home_key(key);
            return;
        }
        if key.code == KeyCode::Enter
            && matches!(
                self.screen,
                Screen::Status
                    | Screen::Usage
                    | Screen::Stats
                    | Screen::Diff
                    | Screen::Transcript
                    | Screen::Help
            )
        {
            self.close();
            return;
        }
        if self.screen == Screen::Settings {
            crate::settings::handle_key(self, key);
            return;
        }
        if self.screen == Screen::Usage && crate::usage::handle_key(self, key) {
            return;
        }
        if key.code == KeyCode::Char('d') && self.screen == Screen::Usage {
            self.metrics_demo = !self.metrics_demo;
            self.scroll = 0;
            return;
        }
        if self.screen == Screen::Stats && crate::stats::handle_key(self, key) {
            return;
        }
        match key.code {
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(10);
                return;
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(10);
                return;
            }
            _ => {}
        }
        if self.screen == Screen::ProviderEdit {
            match key.code {
                KeyCode::Tab | KeyCode::Down => self.selected = (self.selected + 1) % 5,
                KeyCode::BackTab | KeyCode::Up => self.selected = (self.selected + 4) % 5,
                KeyCode::Enter if self.selected == 4 => self.save_provider(),
                KeyCode::Enter => self.selected = (self.selected + 1) % 5,
                _ if self.selected < 4 => self.form[self.selected].key(key),
                _ => {}
            }
            return;
        }
        if self.screen == Screen::Onboarding {
            match key.code {
                KeyCode::Left => self.onboarding_step = self.onboarding_step.saturating_sub(1),
                KeyCode::Enter | KeyCode::Right => {
                    if self.onboarding_step < 2 {
                        self.onboarding_step += 1;
                    } else {
                        self.home();
                        self.notice =
                            "Setup preview complete. Provider configuration is session-only."
                                .into();
                    }
                }
                KeyCode::Char('t') => self.toggle_theme(),
                KeyCode::Char('p') => self.open(Screen::Providers),
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Left | KeyCode::Right if self.screen == Screen::Diff => {
                self.tab = (self.tab + 1) % 2;
                self.scroll = 0;
            }
            KeyCode::Tab if self.screen == Screen::Diff => {
                self.tab = (self.tab + 1) % 2;
                self.scroll = 0;
            }
            KeyCode::Down => {
                let len = self.list_len();
                if len > 0 {
                    self.selected = (self.selected + 1) % len;
                }
                if matches!(
                    self.screen,
                    Screen::Transcript | Screen::Diff | Screen::Help
                ) {
                    self.scroll += 1;
                }
            }
            KeyCode::Up => {
                let len = self.list_len();
                if len > 0 {
                    self.selected = (self.selected + len - 1) % len;
                }
                if matches!(
                    self.screen,
                    Screen::Transcript | Screen::Diff | Screen::Help
                ) {
                    self.scroll = self.scroll.saturating_sub(1);
                }
            }
            KeyCode::Enter => self.activate(),
            KeyCode::Tab if self.screen == Screen::Transcript => self.expanded = !self.expanded,
            _ if matches!(
                self.screen,
                Screen::Sessions | Screen::Transcript | Screen::Help
            ) =>
            {
                self.query.key(key);
                self.scroll = 0;
                self.selected = 0;
            }
            _ => {}
        }
    }
    fn home_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_add(10);
                return;
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(10);
                return;
            }
            _ => {}
        }
        if self.approval {
            match key.code {
                KeyCode::Left | KeyCode::BackTab => {
                    self.approval_selected = (self.approval_selected + 2) % 3
                }
                KeyCode::Right | KeyCode::Tab => {
                    self.approval_selected = (self.approval_selected + 1) % 3
                }
                KeyCode::Enter => match self.approval_selected {
                    0 => self.resolve_approval(true),
                    1 => self.resolve_approval(false),
                    _ => self.open(Screen::Diff),
                },
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Char('?') if self.input.text.is_empty() => self.open(Screen::Help),
            KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.insert("\n")
            }
            KeyCode::Char('\n' | '\r') => self.input.insert("\n"),
            KeyCode::Enter
                if key.modifiers.intersects(
                    KeyModifiers::ALT | KeyModifiers::SHIFT | KeyModifiers::CONTROL,
                ) =>
            {
                self.input.insert("\n")
            }
            KeyCode::Enter
                if self.completion_open()
                    && !self.suggestions().is_empty()
                    && self.suggestions()[self.selected.min(self.suggestions().len() - 1)].kind
                        != SuggestionKind::Command
                    && !active_token(&self.input.text, self.input.cursor).is_some_and(
                        |token| {
                            token.kind == CompletionKind::Setting
                                && matches!(token.query.as_str(), "websearch" | "subagents")
                        },
                    ) =>
            {
                self.accept_completion(self.selected);
            }
            KeyCode::Enter => {
                if self.menu_open() {
                    let items = self.filtered_commands();
                    if let Some((name, _)) =
                        items.get(self.selected.min(items.len().saturating_sub(1)))
                    {
                        self.input.set(name);
                    }
                }
                self.submit();
            }
            KeyCode::Tab if self.completion_open() => {
                self.accept_completion(self.selected);
            }
            KeyCode::Tab if self.menu_open() => {
                let commands = self.filtered_commands();
                if let Some((name, _)) =
                    commands.get(self.selected.min(commands.len().saturating_sub(1)))
                {
                    self.input.set(name);
                    self.selected = 0;
                }
            }
            KeyCode::BackTab => self.mode = (self.mode + 1) % 2,
            KeyCode::Up | KeyCode::Down if self.completion_open() => {
                let len = self.suggestions().len();
                if len > 0 {
                    self.selected = if key.code == KeyCode::Down {
                        (self.selected + 1) % len
                    } else {
                        (self.selected + len - 1) % len
                    };
                }
            }
            KeyCode::Up | KeyCode::Down
                if key.modifiers.is_empty() && self.history_boundary(key.code) =>
            {
                self.recall_history(key.code == KeyCode::Up);
            }
            _ => {
                if let Err(error) = self.input.key_at_width(key, self.editor_width) {
                    self.notice = error;
                }
                self.menu_dismissed = false;
                self.selected = 0;
            }
        }
    }
    fn history_boundary(&self, key: KeyCode) -> bool {
        let rows = layout_rows(&self.input.text, self.editor_width);
        let index = rows
            .iter()
            .rposition(|r| r.start <= self.input.cursor)
            .unwrap_or(0);
        if key == KeyCode::Up {
            index == 0
        } else {
            index + 1 == rows.len()
        }
    }
    fn recall_history(&mut self, older: bool) {
        let len = self.history.len();
        if older {
            if self.history_index == 0 {
                return;
            }
            if self.history_index == len {
                // Preserve cursor, attachments, selection and undo in the unsent draft.
                self.history_draft = Some(self.input.clone());
            }
            self.history_index -= 1;
        } else {
            if self.history_index >= len {
                return;
            }
            self.history_index += 1;
        }
        if self.history_index == len {
            if let Some(draft) = self.history_draft.take() {
                self.input = draft;
            }
        } else {
            self.input = self.history[self.history_index].clone();
            self.input.cursor = self.input.text.len();
        }
        self.menu_dismissed = true;
        self.selected = 0;
    }
    fn resolve_approval(&mut self, allow: bool) {
        self.approval = false;
        self.add(Entry::new(
            EntryKind::Note,
            if allow {
                "Allowed once [demo]"
            } else {
                "Declined [demo]"
            },
            &["Preview decision recorded. No command ran and no files changed."],
        ));
    }
    fn toggle_theme(&mut self) {
        self.preferences.values.theme = if self.theme.mode == ThemeMode::Dark {
            ThemeMode::Terminal
        } else {
            ThemeMode::Dark
        };
        self.preferences
            .commit(crate::preferences::PreferenceField::Theme);
        crate::settings::sync_effective(self);
    }
    fn activate(&mut self) {
        match self.screen {
            Screen::Preview => {
                let name = SCENARIOS[self.selected].0;
                self.load_scenario(name).expect("known scenario");
            }
            Screen::Models => {
                if self.selected == self.model_rows().len() {
                    self.open(Screen::ProviderPicker);
                    return;
                }
                if let Some((p, model)) = self.model_rows().get(self.selected).cloned() {
                    if let Some(model) = model {
                        self.effort = self.picker_effort();
                        self.active_provider = p;
                        self.active_model = Some(model);
                        self.close();
                    } else {
                        self.start_provider_edit(p);
                    }
                }
            }
            Screen::ProviderPicker => {
                let chosen = self.selected;
                self.close();
                self.model_provider = chosen;
                self.selected = 0;
            }
            Screen::Providers => {
                if self.selected == self.providers.len() {
                    self.providers.push(ProviderDescriptor {
                        id: format!("custom-{}", self.providers.len()),
                        label: "Custom provider".into(),
                        endpoint: String::new(),
                        credential_source: String::new(),
                        models: vec![],
                        configured: false,
                    });
                }
                self.start_provider_edit(self.selected);
            }
            Screen::Sessions => {
                if let Some(index) = self.filtered_sessions().get(self.selected).copied() {
                    self.home();
                    self.entries = conversation_fixture();
                    self.agents = crate::agents::AgentActivity::default();
                    self.view_selection = None;
                    self.active_provider = index.min(3);
                    self.active_model = Some("Demo model".into());
                    self.notice = format!(
                        "Loaded demo session: {}. Nothing was read from disk.",
                        SESSIONS[index].0
                    );
                }
            }
            _ => {}
        }
    }
}

pub fn conversation_fixture() -> Vec<Entry> {
    vec![
        Entry::new(
            EntryKind::User,
            "you · demo",
            &["Extract the shared request validation into a helper."],
        ),
        Entry::new(
            EntryKind::Tool,
            "Read · src/validation.rs · complete [demo]",
            &[
                "Two handlers repeat the same input guard.",
                "The error types already match.",
                "A shared helper keeps the handlers short.",
                "Demo detail: malformed requests return an explicit error.",
                "Demo detail: empty input is rejected.",
                "Demo detail: no real source files were opened.",
            ],
        ),
        Entry::new(
            EntryKind::Tool,
            "Patch · 2 files · proposed [demo]",
            &[
                "src/validation.rs  +8 -3",
                "tests/validation.rs  +6 -0",
                "Open /diff to inspect the sample patch.",
            ],
        ),
        Entry::new(
            EntryKind::Assistant,
            "polaris · demo response",
            &[
                "I would extract the shared guard and keep the handlers focused on their work.",
                "",
                "The sample patch adds a **shared helper** and covers empty input.",
                "",
                "```rust",
                "fn require_input(input: &str) -> Result<(), Error> {",
                "    if input.trim().is_empty() { return Err(Error::EmptyInput); }",
                "    Ok(())",
                "}",
                "```",
                "No files have been read, edited, or tested by this preview.",
                "",
                "Use /diff to review the fixture, or Ctrl+O for full tool output.",
            ],
        ),
    ]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    File,
    Command,
    Directive,
    Setting,
}
#[derive(Clone, Debug)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    pub kind: SuggestionKind,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Submission {
    pub planning: bool,
    pub goal: bool,
    pub settings: crate::prompt_settings::PromptSettings,
    pub settings_errors: Vec<String>,
    pub settings_only: bool,
    pub text: String,
    pub files: Vec<String>,
    pub attachment_ids: Vec<u64>,
}
#[derive(Clone, Debug)]
pub enum ClipboardAction {
    Paste(u64),
    Copy(u64, String),
}
#[derive(Clone, Debug)]
pub enum ClipboardPurpose {
    Paste,
    Copy(usize),
}

impl App {
    pub fn start_index(&mut self) {
        if self.file_index.is_none() {
            self.file_index = Some(FileIndex::start(self.workspace.clone().into()));
        }
    }
    pub fn poll_index(&mut self) -> bool {
        self.file_index.as_mut().is_some_and(FileIndex::poll)
    }
    pub fn suggestions(&self) -> Vec<Suggestion> {
        if self.screen != Screen::Home
            || self.menu_dismissed
            || self.approval
            || self.preview.is_some()
        {
            return vec![];
        }
        let Some(t) = active_token(&self.input.text, self.input.cursor) else {
            return vec![];
        };
        if self
            .input
            .spans
            .iter()
            .any(|s| s.range.contains(&t.range.start))
        {
            return vec![];
        }
        match t.kind {
            CompletionKind::Mention => self
                .file_index
                .as_ref()
                .map(|i| i.query(&t.query))
                .unwrap_or_default()
                .into_iter()
                .map(|label| Suggestion {
                    label,
                    detail: "workspace file".into(),
                    kind: SuggestionKind::File,
                })
                .collect(),
            CompletionKind::Setting => {
                let prefix = format!("{}{}", char::from(36), t.query);
                crate::prompt_settings::SETTINGS
                    .iter()
                    .filter(|(name, _)| name.starts_with(&prefix))
                    .map(|(name, detail)| Suggestion {
                        label: (*name).into(),
                        detail: self.prompt_settings.completion_detail(name, detail),
                        kind: SuggestionKind::Setting,
                    })
                    .collect()
            }
            CompletionKind::Slash => {
                let prefix = format!("/{}", t.query);
                let mut out = vec![];
                if t.range.start == 0 {
                    out.extend(COMMANDS.iter().filter(|(n, _)| n.starts_with(&prefix)).map(
                        |(n, d)| Suggestion {
                            label: (*n).into(),
                            detail: (*d).into(),
                            kind: SuggestionKind::Command,
                        },
                    ));
                }
                out.extend(
                    [
                        ("/plan", "Plan this request"),
                        ("/goal", "Mark this request as a goal"),
                    ]
                    .into_iter()
                    .filter(|(n, _)| n.starts_with(&prefix))
                    .map(|(n, d)| Suggestion {
                        label: n.into(),
                        detail: d.into(),
                        kind: SuggestionKind::Directive,
                    }),
                );
                out
            }
        }
    }
    pub fn completion_open(&self) -> bool {
        self.screen == Screen::Home
            && !self.menu_dismissed
            && !self.approval
            && self.preview.is_none()
            && active_token(&self.input.text, self.input.cursor).is_some()
    }
    pub fn accept_completion(&mut self, index: usize) {
        let Some(t) = active_token(&self.input.text, self.input.cursor) else {
            return;
        };
        let Some(item) = self.suggestions().get(index).cloned() else {
            return;
        };
        let r = match item.kind {
            SuggestionKind::File => {
                if item.label.contains(char::is_whitespace) {
                    format!("@\"{}\" ", item.label)
                } else {
                    format!("@{} ", item.label)
                }
            }
            SuggestionKind::Setting => {
                if self.input.text[t.range.end..].starts_with('=') {
                    item.label
                } else if matches!(item.label.as_str(), "$websearch" | "$subagents") {
                    format!("{} ", item.label)
                } else {
                    format!("{}=[", item.label)
                }
            }
            SuggestionKind::Directive => format!("{} ", item.label),
            SuggestionKind::Command => item.label,
        };
        if let Err(e) = self.input.replace(t.range, &r) {
            self.notice = e;
        }
        self.selected = 0;
    }
    pub fn paste_text(&mut self, text: &str) {
        self.agents.focused = false;
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if text.len() > crate::composer::MAX_TEXT {
            self.notice = "Paste exceeds the 1 MiB limit; nothing was inserted.".into();
            return;
        }
        if text.split('\n').count() > 5 || text.chars().count() > 500 {
            self.add_attachment(AttachmentPayload::Text(text));
        } else if let Err(e) = self.input.try_insert(&text) {
            self.notice = e;
        }
    }
    pub fn paste_image(&mut self, width: usize, height: usize, rgba: Vec<u8>) {
        self.agents.focused = false;
        if width == 0
            || height == 0
            || width.checked_mul(height).and_then(|x| x.checked_mul(4)) != Some(rgba.len())
            || rgba.len() > 64 * 1024 * 1024
        {
            self.notice = "Image dimensions or size are invalid; nothing was inserted.".into();
            return;
        }
        self.add_attachment(AttachmentPayload::Image {
            width,
            height,
            rgba,
        });
    }
    fn add_attachment(&mut self, payload: AttachmentPayload) {
        let size = match &payload {
            AttachmentPayload::Text(s) => s.len(),
            AttachmentPayload::Image { rgba, .. } => rgba.len(),
        };
        if self.attachments.values().map(|a| a.bytes()).sum::<usize>() + size > 128 * 1024 * 1024 {
            self.notice = "Session attachments exceed 128 MiB; nothing was inserted.".into();
            return;
        }
        let label = match payload {
            AttachmentPayload::Text(ref text) => {
                format!("[Pasted Content {} chars]", text.chars().count())
            }
            _ => format!("[Image {}]", self.next_image),
        };
        let a = Arc::new(Attachment {
            id: self.next_attachment,
            label,
            payload,
        });
        if let Err(e) = self.input.insert_attachment(a.clone()) {
            self.notice = e;
            return;
        }
        if matches!(a.payload, AttachmentPayload::Image { .. }) {
            self.next_image += 1;
        }
        self.next_attachment += 1;
        self.attachments.insert(a.id, a);
    }
    pub fn open_attachment(&mut self, id: u64) {
        if let Some(a) = self.attachments.get(&id) {
            self.preview = Some(a.clone());
            self.preview_scroll = 0;
            self.view_selection = None;
        }
    }
    pub fn expand_preview(&mut self) {
        if let Some(a) = self.preview.clone()
            && let AttachmentPayload::Text(text) = &a.payload
        {
            let r = if self.input.spans.iter().any(|s| s.attachment.id == a.id) {
                self.input.expand_attachment(a.id)
            } else {
                self.input.try_insert(text)
            };
            if let Err(e) = r {
                self.notice = e;
            } else {
                self.preview = None;
                self.view_selection = None;
            }
        }
    }
    pub fn toast_text(&self) -> Option<&str> {
        self.toast
            .as_ref()
            .filter(|(_, until)| *until > crate::frame::now())
            .map(|(s, _)| s.as_str())
    }
    pub fn set_toast(&mut self, text: String) {
        self.toast = Some((text, Instant::now() + Duration::from_secs(2)));
    }
    pub fn request_copy(&mut self, text: String) {
        use unicode_segmentation::UnicodeSegmentation;
        if text.is_empty() {
            return;
        }
        let token = self.next_clipboard;
        self.next_clipboard += 1;
        self.clipboard_pending
            .insert(token, ClipboardPurpose::Copy(text.unicode_words().count()));
        self.clipboard_actions
            .push(ClipboardAction::Copy(token, text));
    }
    pub fn request_paste(&mut self) {
        self.agents.focused = false;
        let t = self.next_clipboard;
        self.next_clipboard += 1;
        self.input.bookmark(t);
        self.clipboard_pending.insert(t, ClipboardPurpose::Paste);
        self.clipboard_actions.push(ClipboardAction::Paste(t));
    }
    pub fn clipboard_result(&mut self, result: crate::clipboard::ClipboardResult) {
        let purpose = self.clipboard_pending.remove(&result.token);
        match result.result {
            Err(e) => {
                self.input.take_bookmark(result.token);
                self.set_toast(format!("Clipboard: {e}"));
            }
            Ok(crate::clipboard::ClipboardOperationResult::Copy) => {
                if let Some(ClipboardPurpose::Copy(n)) = purpose {
                    self.set_toast(format!(
                        "Copied {n} {}",
                        if n == 1 { "word" } else { "words" }
                    ));
                }
            }
            Ok(crate::clipboard::ClipboardOperationResult::Paste(p)) => {
                let Some(range) = self.input.take_bookmark(result.token) else {
                    self.set_toast("Paste canceled because the draft changed.".into());
                    return;
                };
                let old_cursor = self.input.cursor;
                let old_len = self.input.text.len();
                let end = range.end;
                let start = range.start;
                self.input.anchor = Some(start);
                self.input.cursor = end;
                match p {
                    crate::clipboard::ClipboardPayload::Text(text) => self.paste_text(&text),
                    crate::clipboard::ClipboardPayload::Image {
                        width,
                        height,
                        rgba,
                    } => self.paste_image(width, height, rgba),
                }
                let delta = self.input.text.len() as isize - old_len as isize;
                if old_cursor > end {
                    self.input.cursor = self
                        .input
                        .snap((old_cursor as isize + delta).max(0) as usize);
                } else if old_cursor < start {
                    self.input.cursor = old_cursor;
                }
            }
        }
    }
    fn rich_key(&mut self, key: KeyEvent) -> bool {
        if self.screen == Screen::Home {
            self.composer_scroll = 0;
        }
        if let Some(p) = self.preview.clone() {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.preview = None;
                    self.view_selection = None;
                }
                KeyCode::Char('e') => self.expand_preview(),
                KeyCode::Char('c') => {
                    if let AttachmentPayload::Text(t) = &p.payload {
                        self.request_copy(t.clone());
                    }
                }
                KeyCode::Up => self.preview_scroll = self.preview_scroll.saturating_sub(1),
                KeyCode::Down => self.preview_scroll = self.preview_scroll.saturating_add(1),
                KeyCode::PageUp => self.preview_scroll = self.preview_scroll.saturating_sub(10),
                KeyCode::PageDown => self.preview_scroll = self.preview_scroll.saturating_add(10),
                _ => {}
            }
            return true;
        }
        if self.screen == Screen::Home && key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('v') => {
                    self.request_paste();
                    return true;
                }
                KeyCode::Char('c' | 'x') if self.input.selection().is_some() => {
                    self.request_copy(self.input.selected_text());
                    if key.code == KeyCode::Char('x') {
                        self.input.delete();
                    }
                    return true;
                }
                _ => {}
            }
        }
        if key.code == KeyCode::Insert
            && key.modifiers.contains(KeyModifiers::SHIFT)
            && self.screen == Screen::Home
        {
            self.request_paste();
            return true;
        }
        if self.input.selection().is_some()
            && key.code == KeyCode::Esc
            && self.screen == Screen::Home
        {
            self.input.anchor = None;
            return true;
        }
        if self.view_selection.is_some() && key.code == KeyCode::Esc {
            self.view_selection = None;
            return true;
        }
        false
    }
}
pub fn parse_submission(draft: &Composer) -> Submission {
    let parsed_settings = crate::prompt_settings::parse(draft);
    let mut r = Submission {
        settings_only: !parsed_settings.ranges.is_empty()
            && draft.text.char_indices().all(|(i, ch)| {
                ch.is_whitespace() || parsed_settings.ranges.iter().any(|r| r.contains(&i))
            }),
        settings: parsed_settings.settings,
        settings_errors: parsed_settings.errors,
        text: draft.expanded_text(),
        attachment_ids: draft.spans.iter().map(|s| s.attachment.id).collect(),
        ..Submission::default()
    };
    let mut code = false;
    let mut token_offset = 0;
    for token in draft.text.split_whitespace() {
        let offset = token_offset + draft.text[token_offset..].find(token).unwrap();
        token_offset = offset + token.len();
        if parsed_settings.ranges.iter().any(|r| r.contains(&offset)) {
            continue;
        }
        if token.starts_with(char::from(96)) {
            code = !code;
            if token.ends_with(char::from(96)) && token.len() > 1 {
                code = false;
            }
            continue;
        }
        if code {
            if token.ends_with(char::from(96)) {
                code = false;
            }
            continue;
        }
        match token.trim_end_matches([',', '.', ':', '!', '?']) {
            "/plan" => r.planning = true,
            "/goal" => r.goal = true,
            _ => {}
        }
    }
    for (i, ch) in draft.text.char_indices() {
        if ch == '@' && !parsed_settings.ranges.iter().any(|r| r.contains(&i)) {
            let end = draft.text[i..]
                .find(char::is_whitespace)
                .map_or(draft.text.len(), |n| i + n);
            let end = if draft.text[i..].starts_with("@\"") {
                draft.text[i + 2..].find('"').map_or(end, |n| i + 2 + n)
            } else {
                end
            };
            if let Some(t) = active_token(&draft.text, end)
                && t.kind == CompletionKind::Mention
            {
                r.files.push(t.query);
            }
        }
    }
    r
}
