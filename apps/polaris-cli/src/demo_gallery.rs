//! Typed, deterministic local screens used by the isolated CLI demo.
//! All values are fixtures. This module has no filesystem, network, or provider access.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use polaris_sdk::effort::Effort;
use polaris_ui_cli::screen::RenderOptions;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Wrap},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GalleryPage {
    Welcome,
    Preview,
    Model,
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
    Approval,
    Help,
    Settings,
    Agents,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelSelection {
    pub id: &'static str,
    pub name: &'static str,
    pub provider: &'static str,
    pub effort: Effort,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderDraft {
    pub label: String,
    pub endpoint: String,
    pub credential_source: String,
    pub model_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GalleryEffect {
    Consumed,
    Close,
    LoadScenario(&'static str),
    ModelSelected(ModelSelection),
    SettingSelected {
        key: &'static str,
        value: &'static str,
    },
    ProviderSaved(ProviderDraft),
    ApprovalDecision(bool),
    ViewedConversation {
        scenario: &'static str,
        title: &'static str,
        owner: &'static str,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Lifecycle state shown by a deterministic local agent fixture.
pub enum AgentStatus {
    /// The task has not started.
    Queued,
    /// The task is actively progressing.
    Working,
    /// The task is waiting on an external decision or dependency.
    Waiting,
    /// The task is paused.
    Paused,
    /// The task completed successfully.
    Completed,
    /// The task failed.
    Failed,
    /// The task was cancelled.
    Cancelled,
}
impl AgentStatus {
    /// Returns the stable display label for this status.
    pub const fn label(self) -> &'static str {
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
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// One static agent and conversation record in a demo scenario.
pub struct AgentFixtureRow {
    /// Stable fixture-local identifier.
    pub id: u64,
    /// Agent display name.
    pub name: &'static str,
    /// Conversation task summary.
    pub task: &'static str,
    /// Current fixture lifecycle state.
    pub status: AgentStatus,
    /// Elapsed fixture time in seconds.
    pub elapsed_seconds: u64,
    /// Token usage, or `None` when the fixture does not report it.
    pub tokens: Option<u64>,
    /// Model display name.
    pub model: &'static str,
    /// Effort display label.
    pub effort: &'static str,
    /// Scripted conversation summary for this fixture.
    pub transcript_summary: &'static str,
    /// Labeled fixture failure detail, when the row represents a failure.
    pub failure_note: Option<&'static str>,
}
const fn agent_row(
    id: u64,
    name: &'static str,
    task: &'static str,
    status: AgentStatus,
    elapsed_seconds: u64,
    tokens: Option<u64>,
) -> AgentFixtureRow {
    AgentFixtureRow {
        id,
        name,
        task,
        status,
        elapsed_seconds,
        tokens,
        model: "Nemotron Super 49B v1.5",
        effort: "high",
        transcript_summary: "The registry should expose typed entries and validate names at its boundary. Suggested checks: duplicate names return a clear error; unknown entries remain distinguishable from empty ones.",
        failure_note: if matches!(status, AgentStatus::Failed) {
            Some("This is a labeled failure fixture; no backend request failed.")
        } else {
            None
        },
    }
}
const AGENT_WORKING_ROWS: [AgentFixtureRow; 2] = [
    agent_row(
        1,
        "Registry",
        "Implement storage registry",
        AgentStatus::Working,
        12,
        Some(12_400),
    ),
    agent_row(
        2,
        "RTK tools",
        "Implement tool registry",
        AgentStatus::Working,
        15,
        None,
    ),
];
const AGENT_MIXED_ROWS: [AgentFixtureRow; 7] = [
    agent_row(
        1,
        "Registry",
        "Implement storage registry",
        AgentStatus::Working,
        12,
        Some(12_400),
    ),
    agent_row(
        2,
        "RTK tools",
        "Implement tool registry",
        AgentStatus::Waiting,
        15,
        None,
    ),
    agent_row(
        3,
        "Review 1",
        "Review implementation area 1",
        AgentStatus::Paused,
        18,
        Some(18_600),
    ),
    agent_row(
        4,
        "Review 2",
        "Review implementation area 2",
        AgentStatus::Queued,
        21,
        Some(21_700),
    ),
    agent_row(
        5,
        "Review 3",
        "Review implementation area 3",
        AgentStatus::Completed,
        24,
        Some(24_800),
    ),
    agent_row(
        6,
        "Review 4",
        "Review implementation area 4",
        AgentStatus::Failed,
        27,
        Some(27_900),
    ),
    agent_row(
        7,
        "Review 5",
        "Review implementation area 5",
        AgentStatus::Cancelled,
        30,
        Some(31_000),
    ),
];
const AGENT_COMPLETED_ROWS: [AgentFixtureRow; 2] = [
    agent_row(
        1,
        "Registry",
        "Implement storage registry",
        AgentStatus::Completed,
        12,
        Some(12_400),
    ),
    agent_row(
        2,
        "RTK tools",
        "Implement tool registry",
        AgentStatus::Completed,
        15,
        None,
    ),
];
const AGENT_MANY_ROWS: [AgentFixtureRow; 9] = [
    agent_row(
        1,
        "Registry",
        "Implement storage registry",
        AgentStatus::Working,
        12,
        Some(12_400),
    ),
    agent_row(
        2,
        "RTK tools",
        "Implement tool registry",
        AgentStatus::Working,
        15,
        None,
    ),
    agent_row(
        3,
        "Review 1",
        "Review implementation area 1",
        AgentStatus::Working,
        18,
        Some(18_600),
    ),
    agent_row(
        4,
        "Review 2",
        "Review implementation area 2",
        AgentStatus::Working,
        21,
        Some(21_700),
    ),
    agent_row(
        5,
        "Review 3",
        "Review implementation area 3",
        AgentStatus::Working,
        24,
        Some(24_800),
    ),
    agent_row(
        6,
        "Review 4",
        "Review implementation area 4",
        AgentStatus::Working,
        27,
        Some(27_900),
    ),
    agent_row(
        7,
        "Review 5",
        "Review implementation area 5",
        AgentStatus::Working,
        30,
        Some(31_000),
    ),
    agent_row(
        8,
        "Review 6",
        "Review implementation area 6",
        AgentStatus::Working,
        33,
        Some(34_100),
    ),
    agent_row(
        9,
        "Review 7",
        "Review implementation area 7",
        AgentStatus::Working,
        36,
        Some(37_200),
    ),
];
impl GalleryEffect {
    pub const fn is_consumed(&self) -> bool {
        !matches!(self, Self::Close)
    }
}
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
    ("questions", "Interactive local questionnaire"),
    ("editor", "Editable sample patch"),
];
const MODELS: &[(&str, &str, &str)] = &[
    (
        "nvidia/llama-3.3-nemotron-super-49b-v1.5",
        "Nemotron Super 49B v1.5",
        "NVIDIA NIM",
    ),
    (
        "nvidia/nvidia-nemotron-nano-9b-v2",
        "Nemotron Nano 9B v2",
        "NVIDIA NIM",
    ),
    (
        "nvidia/llama-3.1-nemotron-nano-8b-v1",
        "Nemotron Nano 8B v1",
        "NVIDIA NIM",
    ),
];
const MAX_PROVIDER_FIELD_CHARS: usize = 256;
const MAX_SESSION_QUERY_CHARS: usize = 128;
fn append_sanitized(target: &mut String, input: &str, max_chars: usize) {
    let remaining = max_chars.saturating_sub(target.chars().count());
    for ch in input.chars().filter(|ch| !ch.is_control()).take(remaining) {
        target.push(ch);
    }
}
const SESSIONS: &[(&str, &str, &str)] = &[
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
const PROVIDERS: &[(&str, &str, &str)] = &[
    (
        "NVIDIA NIM",
        "https://integrate.api.nvidia.com/v1",
        "Credential source: environment / local key store",
    ),
    (
        "Anthropic",
        "https://api.anthropic.com",
        "Credential source: environment / local key store",
    ),
    (
        "OpenAI",
        "https://api.openai.com/v1",
        "Credential source: environment / local key store",
    ),
    (
        "Local",
        "http://localhost:11434/v1",
        "Credential source: none required",
    ),
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageBarStyle {
    Ticks,
    Strip,
    Segments,
}
impl UsageBarStyle {
    pub const ALL: [Self; 3] = [Self::Ticks, Self::Strip, Self::Segments];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ticks => "Ticks",
            Self::Strip => "Strip",
            Self::Segments => "Segments",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatsSummary {
    pub sessions: usize,
    pub active_days: usize,
    pub tokens: u64,
    pub model_tokens: [u64; 3],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StatsDay {
    sessions: usize,
    models: [u64; 3],
}
fn stats_day(index: usize) -> StatsDay {
    let mut row = StatsDay {
        sessions: 0,
        models: [0; 3],
    };
    if (index
        .saturating_mul(17)
        .saturating_add(index / 7 * 3)
        .saturating_add(11))
        % 13
        < 4
    {
        return row;
    }
    let session_count = 1 + (index.saturating_mul(7).saturating_add(3)) % 6;
    for session in 0..session_count {
        row.sessions = row.sessions.saturating_add(1);
        for model in 0..3 {
            if (index + session + model).is_multiple_of(7) {
                continue;
            }
            let wave = (index
                .saturating_mul(model + 3)
                .saturating_add(session.saturating_mul(11))
                .saturating_add(model.saturating_mul(7)))
                % 19;
            let gain = match model {
                0 => {
                    if index % 41 < 12 {
                        17u64
                    } else {
                        5
                    }
                }
                1 => {
                    if index % 67 > 40 {
                        12
                    } else {
                        2
                    }
                }
                _ => {
                    if index % 29 < 5 {
                        18
                    } else {
                        1
                    }
                }
            };
            let base = u64::try_from(wave.saturating_add(1))
                .unwrap_or(0)
                .saturating_mul(gain)
                .saturating_mul(80);
            let multiplier = u64::try_from(1 + index % 9).unwrap_or(1);
            let value = base
                .saturating_add(base / 3)
                .saturating_add(u64::try_from(session.saturating_mul(11)).unwrap_or(0))
                .saturating_add(base.saturating_mul(multiplier))
                .saturating_add(base / 5);
            if let Some(total) = row.models.get_mut(model) {
                *total = total.saturating_add(value);
            }
        }
    }
    row
}
fn stats_range_bounds(range: usize) -> (usize, usize) {
    let days = match range {
        1 => 7,
        2 => 30,
        _ => 365,
    };
    (365usize.saturating_sub(days), 365)
}
fn stats_date(index: usize) -> String {
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut year = 2025usize;
    let mut month = 10usize;
    let mut day = 19usize;
    let mut remaining = index;
    loop {
        let month_days: usize = match month {
            2 if year.is_multiple_of(4)
                && (!year.is_multiple_of(100) || year.is_multiple_of(400)) =>
            {
                29
            }
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        };
        let days_left = month_days.saturating_sub(day).saturating_add(1);
        if remaining < days_left {
            day = day.saturating_add(remaining);
            break;
        }
        remaining = remaining.saturating_sub(days_left);
        day = 1;
        month = month.saturating_add(1);
        if month > 12 {
            month = 1;
            year = year.saturating_add(1);
        }
    }
    let month_name = months
        .get(month.saturating_sub(1))
        .copied()
        .map_or("???", |v| v);
    format!("{month_name} {day}, {year}")
}
fn stats_summary(range: usize) -> StatsSummary {
    let (start, end) = stats_range_bounds(range);
    let mut out = StatsSummary {
        sessions: 0,
        active_days: 0,
        tokens: 0,
        model_tokens: [0; 3],
    };
    for day in start..end {
        let row = stats_day(day);
        out.sessions = out.sessions.saturating_add(row.sessions);
        if row.sessions > 0 {
            out.active_days = out.active_days.saturating_add(1);
        }
        for model in 0..3 {
            if let (Some(total), Some(value)) =
                (out.model_tokens.get_mut(model), row.models.get(model))
            {
                *total = total.saturating_add(*value);
                out.tokens = out.tokens.saturating_add(*value);
            }
        }
    }
    out
}
fn stats_stack_row(points: &[[u64; 3]], scale: u64, row: usize, height: usize) -> String {
    let level = height.saturating_sub(row.saturating_add(1));
    let denominator = u128::from(scale.max(1));
    let height_u128 = u128::try_from(height).unwrap_or(0);
    points
        .iter()
        .map(|point| {
            let mut lower = 0u128;
            for (index, value) in point.iter().copied().enumerate() {
                let upper = lower
                    .saturating_add(u128::from(value))
                    .saturating_mul(height_u128)
                    / denominator;
                let glyph = if u128::try_from(level)
                    .is_ok_and(|v| v >= lower * height_u128 / denominator && v < upper)
                {
                    char::from_digit(u32::try_from(index + 1).unwrap_or(0), 10).unwrap_or('?')
                } else {
                    ' '
                };
                lower = lower.saturating_add(u128::from(value));
                if glyph != ' ' {
                    return glyph;
                }
            }
            ' '
        })
        .collect()
}
fn stats_sparkline(points: &[[u64; 3]], model: usize, scale: u64, ascii: bool) -> String {
    let glyphs = if ascii {
        ['.', '-', '+', '=', '*', '#', '%', '@']
    } else {
        ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']
    };
    points
        .iter()
        .map(|point| {
            let value = point.get(model).copied().unwrap_or(0);
            let slot =
                usize::try_from(u128::from(value).saturating_mul(7) / u128::from(scale.max(1)))
                    .unwrap_or(0)
                    .min(7);
            glyphs.get(slot).copied().unwrap_or('*')
        })
        .collect()
}
const SETTINGS_CATEGORIES: [&str; 6] = [
    "Appearance",
    "Usage",
    "Statistics",
    "Interaction",
    "Accessibility",
    "Providers",
];
fn usage_bar(style: UsageBarStyle, used: u64, capacity: u64, width: usize, ascii: bool) -> String {
    let w = width.min(55);
    if w == 0 {
        return String::new();
    }
    if capacity == 0 {
        return "Unavailable".to_owned();
    }
    let fill_width = match style {
        UsageBarStyle::Ticks => w.saturating_sub(1),
        UsageBarStyle::Strip => w,
        UsageBarStyle::Segments => w.div_ceil(2),
    };
    let fill = usize::try_from(
        u128::from(used.min(capacity)).saturating_mul(u128::try_from(fill_width).unwrap_or(0))
            / u128::from(capacity),
    )
    .unwrap_or(0);
    (0..w)
        .map(|i| {
            let active = match style {
                UsageBarStyle::Ticks => i <= fill.min(w.saturating_sub(1)),
                UsageBarStyle::Strip => i < fill,
                UsageBarStyle::Segments => i / 2 < fill,
            };
            match style {
                UsageBarStyle::Ticks => {
                    if i == fill.min(w.saturating_sub(1)) {
                        if ascii { '+' } else { '╋' }
                    } else if ascii {
                        if active { '#' } else { '|' }
                    } else if active {
                        '┬'
                    } else {
                        '─'
                    }
                }
                UsageBarStyle::Strip => {
                    if ascii {
                        if active { '#' } else { '-' }
                    } else if active {
                        '▄'
                    } else {
                        '·'
                    }
                }
                UsageBarStyle::Segments => {
                    if i % 2 == 1 {
                        ' '
                    } else if ascii {
                        if active { '#' } else { '-' }
                    } else if active {
                        '▮'
                    } else {
                        '▯'
                    }
                }
            }
        })
        .collect()
}

fn usage_number(value: u64) -> String {
    let digits = value.to_string();
    let length = digits.len();
    let mut output = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && length.saturating_sub(index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(digit);
    }
    output
}

fn usage_bar_colors(options: RenderOptions) -> (Color, Color, Color) {
    match options.colors {
        polaris_ui_cli::screen::ColorMode::TrueColor => (
            Color::Rgb(118, 185, 0),
            Color::Rgb(192, 229, 140),
            Color::Rgb(52, 52, 52),
        ),
        polaris_ui_cli::screen::ColorMode::Terminal | polaris_ui_cli::screen::ColorMode::Ansi => {
            (Color::Green, Color::LightGreen, Color::DarkGray)
        }
        polaris_ui_cli::screen::ColorMode::NoColor => (Color::Reset, Color::Reset, Color::Reset),
    }
}

fn usage_bar_line(
    style: UsageBarStyle,
    used: u64,
    capacity: u64,
    width: u16,
    options: RenderOptions,
) -> Line<'static> {
    let width = usize::from(width.min(55));
    if width == 0 {
        return Line::default();
    }
    if capacity == 0 {
        return Line::styled(
            "Unavailable",
            Style::default().fg(usage_bar_colors(options).2),
        );
    }
    let (filled_color, marker_color, empty_color) = usage_bar_colors(options);
    let capacity_width = match style {
        UsageBarStyle::Ticks => width.saturating_sub(1),
        UsageBarStyle::Strip => width,
        UsageBarStyle::Segments => width.div_ceil(2),
    };
    let fill = usize::try_from(
        u128::from(used.min(capacity)).saturating_mul(u128::try_from(capacity_width).unwrap_or(0))
            / u128::from(capacity),
    )
    .unwrap_or(0)
    .min(capacity_width);
    let mut spans = Vec::new();
    for index in 0..width {
        let (glyph, active, marker) = match style {
            UsageBarStyle::Ticks => {
                let position = fill.min(width.saturating_sub(1));
                let tick = index == 0
                    || index == width.saturating_sub(1)
                    || [1usize, 2, 3].iter().any(|quarter| {
                        index == width.saturating_sub(1).saturating_mul(*quarter) / 4
                    });
                let glyph = if index == position {
                    if options.ascii { '+' } else { '╋' }
                } else if options.ascii {
                    if index < position { '#' } else { '|' }
                } else if tick {
                    '┬'
                } else if index < position {
                    '━'
                } else {
                    '─'
                };
                (glyph, index <= position, index == position)
            }
            UsageBarStyle::Strip => {
                let active = index < fill;
                let glyph = if options.ascii {
                    if active { '#' } else { '-' }
                } else if active {
                    '▄'
                } else {
                    '·'
                };
                (glyph, active, active && index.saturating_add(1) == fill)
            }
            UsageBarStyle::Segments => {
                let active = index / 2 < fill;
                let glyph = if index % 2 == 1 {
                    ' '
                } else if options.ascii {
                    if active { '#' } else { '-' }
                } else if active {
                    '▮'
                } else {
                    '▯'
                };
                (glyph, active, active && index / 2 + 1 == fill)
            }
        };
        let color = if marker {
            marker_color
        } else if active {
            filled_color
        } else {
            empty_color
        };
        spans.push(Span::styled(glyph.to_string(), Style::default().fg(color)));
    }
    Line::from(spans)
}

fn usage_pair(label: &str, value: &str, base: Style, dim: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<13}"), dim),
        Span::styled(value.to_owned(), base),
    ])
}

fn usage_gauge_lines(
    label: &str,
    used: Option<u64>,
    capacity: Option<u64>,
    unit: &str,
    reset: Option<&str>,
    style: UsageBarStyle,
    width: u16,
    options: RenderOptions,
    base: Style,
    dim: Style,
) -> Vec<Line<'static>> {
    let valid_capacity = capacity.filter(|value| *value > 0);
    let ratio = used.zip(valid_capacity);
    let percentage = match ratio {
        Some((amount, maximum)) if amount > 0 && u128::from(amount) * 100 < u128::from(maximum) => {
            "<1%".to_owned()
        }
        Some((amount, maximum)) => {
            format!("{}%", u128::from(amount) * 100 / u128::from(maximum))
        }
        None => "Unavailable".to_owned(),
    };
    let header_width = usize::from(width.min(55));
    let spacing = header_width
        .saturating_sub(label.chars().count().saturating_add(percentage.len()))
        .max(2);
    let header = Line::from(vec![
        Span::styled(label.to_owned(), base),
        Span::raw(" ".repeat(spacing)),
        Span::styled(
            if ratio.is_some() {
                format!("{percentage} used")
            } else {
                percentage
            },
            if ratio.is_some() {
                Style::default().fg(usage_bar_colors(options).0)
            } else {
                dim
            },
        ),
    ]);
    let mut lines = vec![header];
    if let Some((amount, maximum)) = ratio {
        lines.push(usage_bar_line(style, amount, maximum, width, options));
        lines.push(Line::styled(
            format!(
                "{} / {} {unit}",
                usage_number(amount),
                usage_number(maximum)
            ),
            dim,
        ));
        if amount >= maximum {
            lines.push(Line::styled(
                if amount == maximum {
                    "Limit reached"
                } else {
                    "Over limit"
                },
                dim,
            ));
        } else if u128::from(amount) * 100 >= u128::from(maximum) * 80 {
            lines.push(Line::styled("Near limit", dim));
        }
    } else if let Some(amount) = used {
        lines.push(Line::styled(
            format!("{} {unit} used; capacity unavailable", usage_number(amount)),
            dim,
        ));
    } else if let Some(maximum) = valid_capacity {
        lines.push(Line::styled(
            format!(
                "Capacity {} {unit}; usage unavailable",
                usage_number(maximum)
            ),
            dim,
        ));
    }
    if let Some(reset) = reset {
        lines.push(Line::styled(reset.to_owned(), dim));
    }
    lines
}

fn usage_left_lines(
    demo: bool,
    style: UsageBarStyle,
    width: u16,
    options: RenderOptions,
    base: Style,
    dim: Style,
) -> Vec<Line<'static>> {
    let mut lines = if demo {
        vec![
            Line::styled("Session", base),
            usage_pair("Total cost", "USD 0.42", base, dim),
            usage_pair("Tokens", "15,580 total / 12,400 in / 3,180 out", base, dim),
            usage_pair("Duration", "2m 14s wall / 1m 56s API", base, dim),
            usage_pair("Changes", "2 files / +14 -3", base, dim),
            Line::default(),
        ]
    } else {
        vec![
            Line::styled("Session", base),
            usage_pair("Total cost", "Unavailable", base, dim),
            usage_pair("Tokens", "Unavailable", base, dim),
            usage_pair("Duration", "Unavailable", base, dim),
            usage_pair("Changes", "Unavailable", base, dim),
            Line::default(),
        ]
    };
    lines.extend(usage_gauge_lines(
        "Context",
        demo.then_some(25_600),
        demo.then_some(128_000),
        "tokens",
        None,
        style,
        width,
        options,
        base,
        dim,
    ));
    lines.push(Line::default());
    lines.push(Line::styled(
        if demo {
            "Limits / sample policy"
        } else {
            "Provider limits"
        },
        base,
    ));
    for (label, used, capacity, reset) in [
        (
            "Current window",
            demo.then_some(420),
            demo.then_some(1_000),
            demo.then_some("Resets in 2h 30m"),
        ),
        (
            "Weekly allowance",
            demo.then_some(2_900),
            demo.then_some(5_000),
            demo.then_some("Resets in 4d"),
        ),
    ] {
        lines.extend(usage_gauge_lines(
            label, used, capacity, "units", reset, style, width, options, base, dim,
        ));
        lines.push(Line::default());
    }
    lines
}

fn usage_right_lines(demo: bool, details: bool, base: Style, dim: Style) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled("Models", base)];
    if demo {
        lines.extend([
            Line::styled("Nemotron Super 49B v1.5", base),
            Line::styled("  10,000 in / 2,500 out / USD 0.35", dim),
            Line::styled("Nemotron Nano 9B v2", base),
            Line::styled("  2,400 in / 680 out / USD 0.07", dim),
        ]);
    } else {
        lines.push(Line::styled("Usage unavailable", dim));
    }
    if !details {
        return lines;
    }
    lines.push(Line::default());
    lines.push(Line::styled("Cache", base));
    lines.push(usage_pair(
        "Read / write",
        if demo {
            "100,000 / 2,000 tokens"
        } else {
            "Unavailable"
        },
        base,
        dim,
    ));
    lines.push(usage_pair(
        "Hit rate",
        if demo {
            "90% / 90 of 100 requests"
        } else {
            "Unavailable"
        },
        base,
        dim,
    ));
    lines.push(Line::default());
    lines.push(Line::styled("Subagents", base));
    if demo {
        lines.extend([
            Line::styled("General purpose / 2 calls / 2,200 tokens", base),
            Line::styled("  Nemotron Nano 9B v2", dim),
            Line::styled("Explore / 1 call / 880 tokens", base),
            Line::styled("  Nemotron Nano 9B v2", dim),
            Line::styled("Included in session/model totals.", dim),
        ]);
    } else {
        lines.push(Line::styled("Usage unavailable", dim));
    }
    lines
}

fn usage_join_columns(
    left: Vec<Line<'static>>,
    right: Vec<Line<'static>>,
    left_width: u16,
) -> Vec<Line<'static>> {
    let count = left.len().max(right.len());
    let mut output = Vec::with_capacity(count);
    for index in 0..count {
        let left_line = left
            .get(index)
            .map_or_else(Line::default, |line| line.clone());
        let right_line = right
            .get(index)
            .map_or_else(Line::default, |line| line.clone());
        let left_used = left_line.width();
        let mut spans = left_line.spans;
        let padding = usize::from(left_width)
            .saturating_add(4)
            .saturating_sub(left_used);
        spans.push(Span::raw(" ".repeat(padding)));
        spans.extend(right_line.spans);
        output.push(Line::from(spans));
    }
    output
}

fn usage_fixture_lines(
    body: Rect,
    demo: bool,
    details: bool,
    style: UsageBarStyle,
    options: RenderOptions,
    base: Style,
    dim: Style,
) -> Vec<Line<'static>> {
    let badge_color = match options.colors {
        polaris_ui_cli::screen::ColorMode::TrueColor => Color::Rgb(229, 180, 84),
        polaris_ui_cli::screen::ColorMode::Terminal | polaris_ui_cli::screen::ColorMode::Ansi => {
            Color::Yellow
        }
        polaris_ui_cli::screen::ColorMode::NoColor => Color::Reset,
    };
    let badge = if demo {
        "DEMO DATA · sample figures, not account telemetry"
    } else {
        "UNAVAILABLE · missing is not zero"
    };
    let control = if details {
        "[ Ctrl+O Less ]"
    } else {
        "[ Ctrl+O Details ]"
    };
    let badge_width = badge.chars().count();
    let control_width = control.chars().count();
    let gap = usize::from(body.width)
        .saturating_sub(badge_width.saturating_add(control_width))
        .max(1);
    let mut output = vec![Line::from(vec![
        Span::styled(badge.to_owned(), Style::default().fg(badge_color)),
        Span::raw(" ".repeat(gap)),
        Span::styled(control.to_owned(), dim),
    ])];
    let inner_width = body.width.saturating_sub(2);
    let left_width = inner_width.saturating_sub(4) / 2;
    let left = usage_left_lines(demo, style, left_width, options, base, dim);
    let right = usage_right_lines(demo, details, base, dim);
    if inner_width >= 104 {
        output.extend(usage_join_columns(left, right, left_width));
    } else {
        output.extend(left);
        output.extend(right);
    }
    output
}

#[derive(Clone, Copy, Debug)]
struct Hit {
    rect: Rect,
    action: HitAction,
}
#[derive(Clone, Copy, Debug)]
enum HitAction {
    Select(usize),
    ToggleDemo,
    ToggleDetails,
    Tab(usize),
    StatsRange(usize),
    StatsBucket(usize),
    SettingValue(usize),
    Approval(bool),
    Open(GalleryPage),
}
#[derive(Clone, Debug)]
pub struct DemoGallery {
    page: GalleryPage,
    selected: usize,
    model: usize,
    effort: usize,
    stats_tab: usize,
    stats_demo: bool,
    stats_range: usize,
    stats_chart: usize,
    stats_details: bool,
    stats_bucket: Option<usize>,
    agent_scenario: Option<&'static str>,
    usage_demo: bool,
    usage_details: bool,
    usage_style: usize,
    onboarding_step: usize,
    query: String,
    provider_draft: ProviderDraft,
    configured_providers: Vec<ProviderDraft>,
    provider_field: usize,
    setting_theme: usize,
    setting_ascii: bool,
    setting_color: usize,
    setting_motion: bool,
    settings_category: usize,
    model_provider: usize,
    settings_row: usize,
    settings_value: usize,
    settings_focused: bool,
    transcript_expanded: bool,
    navigation: Vec<(GalleryPage, usize)>,
    hits: Vec<Hit>,
    hits_ready: bool,
}
impl Default for DemoGallery {
    fn default() -> Self {
        Self::new()
    }
}
impl DemoGallery {
    pub fn new() -> Self {
        Self {
            page: GalleryPage::Welcome,
            selected: 0,
            model: 0,
            effort: 2,
            stats_tab: 0,
            stats_demo: true,
            stats_range: 0,
            stats_chart: 0,
            stats_details: false,
            stats_bucket: None,
            agent_scenario: None,
            usage_demo: true,
            usage_details: false,
            usage_style: 1,
            onboarding_step: 0,
            query: String::new(),
            provider_draft: ProviderDraft {
                label: String::new(),
                endpoint: String::new(),
                credential_source: String::new(),
                model_id: String::new(),
            },
            configured_providers: Vec::new(),
            provider_field: 0,
            setting_theme: 0,
            setting_ascii: false,
            setting_color: 0,
            setting_motion: false,
            settings_category: 0,
            model_provider: 0,
            settings_row: 0,
            settings_value: 0,
            settings_focused: true,
            transcript_expanded: false,
            navigation: Vec::new(),
            hits: Vec::new(),
            hits_ready: false,
        }
    }
    pub fn open(&mut self, page: GalleryPage) {
        self.navigation.clear();
        if page == GalleryPage::Agents {
            self.agent_scenario = None;
        }
        if page == GalleryPage::Settings {
            self.settings_category = 0;
            self.settings_row = 0;
            self.settings_focused = true;
        }
        self.open_page(page);
    }
    /// Applies the local view state for a known preview scenario.
    ///
    /// Returns `false` without changing state when `name` is not in [`SCENARIOS`].
    /// Agent and conversation fixtures remain represented by [`GalleryEffect::LoadScenario`]
    /// so the caller can route them to the corresponding application state.
    pub fn load_scenario(&mut self, name: &str) -> bool {
        let Some((scenario, _)) = SCENARIOS.iter().find(|(scenario, _)| *scenario == name) else {
            return false;
        };
        let page = match *scenario {
            "welcome" => GalleryPage::Welcome,
            "conversation" | "working" | "editor" | "questions" => self.page,
            "messages" | "transcript" => GalleryPage::Transcript,
            "error" | "status" => GalleryPage::Status,
            "approval" => GalleryPage::Approval,
            "diff" => GalleryPage::Diff,
            "onboarding" => GalleryPage::Onboarding,
            "models" => GalleryPage::Model,
            "providers" => GalleryPage::Providers,
            "settings" => GalleryPage::Settings,
            "sessions" => GalleryPage::Sessions,
            "usage" | "usage-demo" => GalleryPage::Usage,
            "stats" | "stats-stacked" | "stats-lines" | "stats-trends" | "stats-details"
            | "stats-demo" => GalleryPage::Stats,
            "agents-working" | "agents-mixed" | "agents-completed" | "agents-many" => {
                GalleryPage::Agents
            }
            "help" => GalleryPage::Help,
            _ => self.page,
        };
        self.agent_scenario = None;
        self.open(page);
        match *scenario {
            "stats" => {
                self.stats_demo = false;
                self.stats_tab = 0;
                self.stats_range = 0;
                self.stats_chart = 0;
                self.stats_details = false;
                self.stats_bucket = None;
            }
            "stats-demo" => {
                self.stats_demo = true;
                self.stats_tab = 0;
                self.stats_range = 0;
                self.stats_chart = 0;
                self.stats_details = false;
                self.stats_bucket = None;
            }
            "stats-stacked" | "stats-lines" | "stats-trends" | "stats-details" => {
                self.stats_demo = true;
                self.stats_tab = if *scenario == "stats-details" { 0 } else { 1 };
                self.stats_range = 0;
                self.stats_chart = match *scenario {
                    "stats-lines" => 1,
                    "stats-trends" => 2,
                    _ => 0,
                };
                self.stats_details = *scenario == "stats-details";
                self.stats_bucket = None;
            }
            "usage" => {
                self.usage_demo = false;
                self.usage_details = false;
            }
            "usage-demo" => {
                self.usage_demo = true;
                self.usage_details = false;
            }
            "agents-working" | "agents-mixed" | "agents-completed" | "agents-many" => {
                self.agent_scenario = Some(*scenario);
            }
            _ => {}
        }
        self.changed();
        true
    }
    fn open_child(&mut self, page: GalleryPage) {
        self.navigation.push((self.page, self.selected));
        self.open_page(page);
    }
    fn open_page(&mut self, page: GalleryPage) {
        self.page = page;
        self.selected = 0;
        self.query.clear();
        self.invalidate_hits();
    }
    fn go_back(&mut self) -> GalleryEffect {
        if let Some((page, selected)) = self.navigation.pop() {
            self.page = page;
            self.selected = selected;
            self.invalidate_hits();
            GalleryEffect::Consumed
        } else {
            self.invalidate_hits();
            GalleryEffect::Close
        }
    }
    pub const fn page(&self) -> GalleryPage {
        self.page
    }
    pub fn selected_model(&self) -> Option<ModelSelection> {
        if self.model_provider != 0 {
            return None;
        }
        let (id, name, provider) = MODELS.get(self.model).copied()?;
        let effort = Effort::ALL.get(self.effort).copied()?;
        Some(ModelSelection {
            id,
            name,
            provider,
            effort,
        })
    }
    pub fn stats_tab_label(&self) -> &'static str {
        ["Overview", "Models"]
            .get(self.stats_tab)
            .copied()
            .map_or("Overview", |v| v)
    }
    pub fn stats_range_label(&self) -> &'static str {
        ["All time", "Last 7 days", "Last 30 days"]
            .get(self.stats_range)
            .copied()
            .map_or("All time", |v| v)
    }
    pub fn stats_chart_label(&self) -> &'static str {
        ["Stacked", "Lines", "Trends"]
            .get(self.stats_chart)
            .copied()
            .map_or("Stacked", |v| v)
    }
    pub fn stats_summary(&self) -> StatsSummary {
        stats_summary(self.stats_range)
    }
    pub fn usage_bar_style(&self) -> UsageBarStyle {
        UsageBarStyle::ALL
            .get(self.usage_style)
            .copied()
            .unwrap_or(UsageBarStyle::Strip)
    }
    pub fn settings_categories(&self) -> &'static [&'static str] {
        &SETTINGS_CATEGORIES
    }
    pub fn settings_category_label(&self) -> &'static str {
        SETTINGS_CATEGORIES
            .get(self.settings_category)
            .copied()
            .map_or("Appearance", |v| v)
    }
    pub const fn stats_details_visible(&self) -> bool {
        self.stats_details
    }
    pub const fn stats_bucket(&self) -> Option<usize> {
        self.stats_bucket
    }
    pub const fn usage_demo_visible(&self) -> bool {
        self.usage_demo
    }
    pub const fn usage_details_visible(&self) -> bool {
        self.usage_details
    }
    #[cfg(test)]
    pub const fn stats_demo_visible(&self) -> bool {
        self.stats_demo
    }
    /// Returns the rows for the currently loaded agent demo, when one is active.
    pub fn agent_fixture_rows(&self) -> &'static [AgentFixtureRow] {
        match self.agent_scenario {
            Some("agents-working") => &AGENT_WORKING_ROWS,
            Some("agents-mixed") => &AGENT_MIXED_ROWS,
            Some("agents-completed") => &AGENT_COMPLETED_ROWS,
            Some("agents-many") => &AGENT_MANY_ROWS,
            _ => &[],
        }
    }
    /// Applies sanitized, bounded pasted text to the active editable field.
    pub fn handle_paste(&mut self, pasted: &str) -> GalleryEffect {
        match self.page {
            GalleryPage::Sessions => {
                append_sanitized(&mut self.query, pasted, MAX_SESSION_QUERY_CHARS);
                self.changed();
            }
            GalleryPage::ProviderEdit if self.provider_field < 4 => {
                let target = match self.provider_field {
                    0 => Some(&mut self.provider_draft.label),
                    1 => Some(&mut self.provider_draft.endpoint),
                    2 => Some(&mut self.provider_draft.credential_source),
                    3 => Some(&mut self.provider_draft.model_id),
                    _ => None,
                };
                if let Some(text) = target {
                    append_sanitized(text, pasted, MAX_PROVIDER_FIELD_CHARS);
                    self.changed();
                }
            }
            _ => {}
        }
        GalleryEffect::Consumed
    }
    fn append_provider_char(&mut self, ch: char) {
        let target = match self.provider_field {
            0 => Some(&mut self.provider_draft.label),
            1 => Some(&mut self.provider_draft.endpoint),
            2 => Some(&mut self.provider_draft.credential_source),
            3 => Some(&mut self.provider_draft.model_id),
            _ => None,
        };
        if let Some(text) = target {
            if !ch.is_control() && text.chars().count() < MAX_PROVIDER_FIELD_CHARS {
                text.push(ch);
            }
            self.changed();
        }
    }
    fn invalidate_hits(&mut self) {
        self.hits.clear();
        self.hits_ready = false;
    }
    fn changed(&mut self) {
        self.invalidate_hits();
    }
    pub fn hit_region(&self, index: usize) -> Option<Rect> {
        self.hits.get(index).map(|h| h.rect)
    }
    pub fn prepare_hits(&mut self, viewport: Rect, _options: RenderOptions) {
        let _ = self.layout(viewport);
    }
    fn line_count(&self) -> usize {
        match self.page {
            GalleryPage::Preview => SCENARIOS.len(),
            GalleryPage::Sessions => self.session_count(),
            GalleryPage::Model => {
                if self.model_provider == 0 {
                    MODELS.len().saturating_add(1)
                } else {
                    1
                }
            }
            GalleryPage::Providers => PROVIDERS
                .len()
                .saturating_add(self.configured_providers.len())
                .saturating_add(1),
            GalleryPage::ProviderPicker => PROVIDERS.len(),
            GalleryPage::Settings => SETTINGS_CATEGORIES.len(),
            GalleryPage::Stats => 2,
            GalleryPage::Agents => {
                if self.agent_scenario.is_some() {
                    self.agent_fixture_rows().len()
                } else {
                    4
                }
            }
            GalleryPage::Approval => 2,
            _ => 0,
        }
    }
    fn layout(&mut self, area: Rect) -> (Rect, usize) {
        self.hits.clear();
        self.hits_ready = false;
        if area.width < 12 || area.height < 5 {
            return (Rect::default(), 0);
        }
        let body = Rect::new(
            area.x.saturating_add(1),
            area.y.saturating_add(2),
            area.width.saturating_sub(2),
            area.height.saturating_sub(2),
        );
        let header = matches!(
            self.page,
            GalleryPage::Preview
                | GalleryPage::Sessions
                | GalleryPage::Providers
                | GalleryPage::ProviderPicker
                | GalleryPage::Settings
                | GalleryPage::Agents
        );
        let offset = usize::from(header);
        let footer = usize::from(self.page == GalleryPage::Agents && self.agent_scenario.is_some());
        let available = usize::from(body.height)
            .saturating_sub(offset)
            .saturating_sub(footer);
        let count = self.line_count();
        let start = self.selected.min(count.saturating_sub(available));
        if self.page == GalleryPage::Model {
            let rows = if self.model_provider == 0 {
                MODELS.len().saturating_add(1)
            } else {
                1
            };
            for idx in 0..rows {
                let row = if self.model_provider == 0 {
                    1usize.saturating_add(idx.saturating_mul(2))
                } else {
                    1
                };
                if row >= usize::from(body.height) {
                    continue;
                }
                let y = body
                    .y
                    .saturating_add(u16::try_from(row).unwrap_or(u16::MAX));
                self.hits.push(Hit {
                    rect: Rect::new(body.x, y, body.width, 1),
                    action: HitAction::Select(idx),
                });
            }
        }
        for row in 0..available {
            let idx = start.saturating_add(row);
            if idx >= count {
                break;
            }
            let y = body
                .y
                .saturating_add(u16::try_from(offset.saturating_add(row)).unwrap_or(u16::MAX));
            let action = match self.page {
                GalleryPage::Preview
                | GalleryPage::Sessions
                | GalleryPage::Providers
                | GalleryPage::ProviderPicker
                | GalleryPage::Settings
                | GalleryPage::Agents => HitAction::Select(idx),
                _ => continue,
            };
            self.hits.push(Hit {
                rect: Rect::new(body.x, y, body.width, 1),
                action,
            });
        }
        if self.page == GalleryPage::Settings {
            let row_start = 1usize.saturating_add(SETTINGS_CATEGORIES.len());
            let row_count = if matches!(self.settings_category, 1 | 2) {
                self.settings_row_count()
            } else {
                1
            };
            for index in 0..row_count {
                let row = row_start.saturating_add(index);
                if row < usize::from(body.height) {
                    let y = body
                        .y
                        .saturating_add(u16::try_from(row).unwrap_or(u16::MAX));
                    self.hits.push(Hit {
                        rect: Rect::new(body.x, y, body.width, 1),
                        action: HitAction::SettingValue(index),
                    });
                }
            }
        }
        if self.page == GalleryPage::Approval {
            for (index, approved) in [true, false].iter().copied().enumerate() {
                let row = 3usize.saturating_add(index);
                if row < usize::from(body.height) {
                    let y = body
                        .y
                        .saturating_add(u16::try_from(row).unwrap_or(u16::MAX));
                    self.hits.push(Hit {
                        rect: Rect::new(body.x, y, body.width, 1),
                        action: HitAction::Approval(approved),
                    });
                }
            }
        }
        if self.page == GalleryPage::Stats && body.height > 0 {
            let tab_width = body.width.min(12);
            self.hits.push(Hit {
                rect: Rect::new(body.x, body.y, tab_width, 1),
                action: HitAction::Tab(0),
            });
            let second_x = body.x.saturating_add(tab_width);
            let remaining = body.width.saturating_sub(tab_width);
            if remaining > 0 {
                self.hits.push(Hit {
                    rect: Rect::new(second_x, body.y, remaining.min(12), 1),
                    action: HitAction::Tab(1),
                });
            }
            for (index, (x_offset, width)) in
                [(20u16, 10u16), (31, 13), (45, 14)].iter().enumerate()
            {
                let x = body.x.saturating_add(*x_offset);
                let available_width = body
                    .x
                    .saturating_add(body.width)
                    .saturating_sub(x)
                    .min(*width);
                if available_width > 0 {
                    self.hits.push(Hit {
                        rect: Rect::new(x, body.y, available_width, 1),
                        action: HitAction::StatsRange(index),
                    });
                }
            }
            let (range_start, range_end) = stats_range_bounds(self.stats_range);
            for day in range_start..range_end {
                if self.stats_tab != 0 || !self.stats_demo {
                    break;
                }
                let weekday = day.saturating_add(6) % 7;
                let column = day / 7;
                let x = body
                    .x
                    .saturating_add(4)
                    .saturating_add(u16::try_from(column).unwrap_or(u16::MAX));
                let y = body
                    .y
                    .saturating_add(4)
                    .saturating_add(u16::try_from(weekday).unwrap_or(u16::MAX));
                if x < body.x.saturating_add(body.width) && y < body.y.saturating_add(body.height) {
                    self.hits.push(Hit {
                        rect: Rect::new(x, y, 1, 1),
                        action: HitAction::StatsBucket(day),
                    });
                }
            }
        }
        if self.page == GalleryPage::Usage {
            if body.height > 0 {
                self.hits.push(Hit {
                    rect: Rect::new(body.x, body.y, body.width, 1),
                    action: HitAction::ToggleDetails,
                });
            }
            if body.height > 1 {
                self.hits.push(Hit {
                    rect: Rect::new(body.x, body.y.saturating_add(1), body.width, 1),
                    action: HitAction::ToggleDemo,
                });
            }
        }
        if self.page == GalleryPage::Welcome && body.height > 0 {
            self.hits.push(Hit {
                rect: Rect::new(body.x, body.y, body.width, 1),
                action: HitAction::Open(GalleryPage::Preview),
            });
        }
        self.hits_ready = true;
        (body, start)
    }
    pub fn handle_mouse(&mut self, event: MouseEvent) -> GalleryEffect {
        if !self.hits_ready || event.kind != MouseEventKind::Down(MouseButton::Left) {
            return GalleryEffect::Consumed;
        }
        let action = self
            .hits
            .iter()
            .find(|h| {
                event.column >= h.rect.x
                    && event.column < h.rect.x.saturating_add(h.rect.width)
                    && event.row >= h.rect.y
                    && event.row < h.rect.y.saturating_add(h.rect.height)
            })
            .map(|h| h.action);
        match action {
            Some(HitAction::Select(i))
                if self.page == GalleryPage::Agents && self.agent_scenario.is_some() =>
            {
                self.selected = i.min(self.agent_fixture_rows().len().saturating_sub(1));
                self.changed();
                self.activate_selected()
            }
            Some(HitAction::Select(i)) if self.page == GalleryPage::Settings => {
                self.settings_category = i.min(SETTINGS_CATEGORIES.len().saturating_sub(1));
                self.settings_focused = true;
                self.settings_row = 0;
                self.changed();
                GalleryEffect::Consumed
            }
            Some(HitAction::Select(i)) if self.page == GalleryPage::Model => {
                if self.model_provider != 0 {
                    if let Some((label, endpoint, credential)) = PROVIDERS.get(self.model_provider)
                    {
                        self.provider_draft = ProviderDraft {
                            label: (*label).to_owned(),
                            endpoint: (*endpoint).to_owned(),
                            credential_source: (*credential).to_owned(),
                            model_id: String::new(),
                        };
                    }
                    self.open_child(GalleryPage::ProviderEdit);
                    GalleryEffect::Consumed
                } else if i >= MODELS.len() {
                    self.open_child(GalleryPage::ProviderPicker);
                    GalleryEffect::Consumed
                } else {
                    self.model = i;
                    self.changed();
                    self.activate_selected()
                }
            }
            Some(HitAction::Select(i)) if self.page == GalleryPage::ProviderPicker => {
                self.selected = i;
                self.model_provider = i;
                self.open_child(GalleryPage::Model);
                GalleryEffect::Consumed
            }
            Some(HitAction::Select(i)) => {
                self.selected = i;
                self.changed();
                self.activate_selected()
            }
            Some(HitAction::SettingValue(index)) => {
                if matches!(self.settings_category, 1 | 2) {
                    self.settings_row = index.min(self.settings_row_count().saturating_sub(1));
                    self.settings_focused = false;
                    self.settings_value = 0;
                } else {
                    self.settings_focused = false;
                    self.settings_value = self.settings_current_value();
                }
                self.activate_settings()
            }
            Some(HitAction::Approval(approved)) => GalleryEffect::ApprovalDecision(approved),
            Some(HitAction::ToggleDemo) => {
                self.usage_demo = !self.usage_demo;
                self.changed();
                GalleryEffect::Consumed
            }
            Some(HitAction::ToggleDetails) => {
                if self.page == GalleryPage::Usage {
                    self.usage_details = !self.usage_details;
                } else {
                    self.stats_details = !self.stats_details;
                }
                self.changed();
                GalleryEffect::Consumed
            }
            Some(HitAction::Tab(i)) => {
                self.stats_tab = i.min(1);
                self.changed();
                GalleryEffect::Consumed
            }
            Some(HitAction::StatsRange(i)) => {
                self.stats_range = i.min(2);
                self.stats_bucket = None;
                self.changed();
                GalleryEffect::Consumed
            }
            Some(HitAction::StatsBucket(day)) => {
                let (start, end) = stats_range_bounds(self.stats_range);
                if (start..end).contains(&day) {
                    self.stats_bucket = Some(day.saturating_sub(start));
                    self.changed();
                }
                GalleryEffect::Consumed
            }
            Some(HitAction::Open(page)) => {
                self.open_child(page);
                GalleryEffect::Consumed
            }
            None => GalleryEffect::Consumed,
        }
    }
    fn visible_session(
        &self,
        position: usize,
    ) -> Option<&'static (&'static str, &'static str, &'static str)> {
        let query = self.query.to_lowercase();
        SESSIONS
            .iter()
            .filter(|(title, owner, date)| {
                query.is_empty()
                    || title.to_lowercase().contains(&query)
                    || owner.to_lowercase().contains(&query)
                    || date.to_lowercase().contains(&query)
            })
            .nth(position)
    }
    fn session_count(&self) -> usize {
        let query = self.query.to_lowercase();
        SESSIONS
            .iter()
            .filter(|(title, owner, date)| {
                query.is_empty()
                    || title.to_lowercase().contains(&query)
                    || owner.to_lowercase().contains(&query)
                    || date.to_lowercase().contains(&query)
            })
            .count()
    }
    fn settings_row_count(&self) -> usize {
        match self.settings_category {
            0 | 4 => 2,
            1 | 2 => 3,
            3 => 1,
            _ => 1,
        }
    }
    fn settings_value_count(&self) -> usize {
        match (self.settings_category, self.settings_row) {
            (0, _) => 2,
            (1, _) | (2, _) => 1,
            (3, _) => 2,
            (4, 0) => 3,
            (4, _) => 2,
            _ => 1,
        }
    }
    fn settings_current_value(&self) -> usize {
        match (self.settings_category, self.settings_row) {
            (0, 0) => self.setting_theme,
            (0, _) => usize::from(self.setting_ascii),
            (1, _) | (2, _) => 0,
            (3, _) => self.settings_value % 2,
            (4, 0) => self.setting_color,
            (4, _) => usize::from(self.setting_motion),
            _ => 0,
        }
    }
    fn activate_settings(&mut self) -> GalleryEffect {
        if self.settings_focused {
            if self.settings_category == 5 {
                self.open_child(GalleryPage::Providers);
                return GalleryEffect::Consumed;
            }
            self.settings_focused = false;
            self.settings_row = match self.settings_category {
                1 => self.usage_style,
                2 => self.stats_chart,
                _ => 0,
            };
            self.settings_value = self.settings_current_value();
            self.changed();
            return GalleryEffect::Consumed;
        }
        let value = self
            .settings_value
            .min(self.settings_value_count().saturating_sub(1));
        let (key, label) = match (self.settings_category, self.settings_row) {
            (0, 0) => {
                self.setting_theme = value;
                (
                    "theme",
                    ["dark", "terminal"]
                        .get(value)
                        .copied()
                        .map_or("dark", |v| v),
                )
            }
            (0, _) => {
                self.setting_ascii = value == 1;
                ("ascii", if self.setting_ascii { "true" } else { "false" })
            }
            (1, _) => {
                self.usage_style = self
                    .settings_row
                    .min(UsageBarStyle::ALL.len().saturating_sub(1));
                ("usage_bar_style", self.usage_bar_style().label())
            }
            (2, _) => {
                self.stats_chart = self.settings_row.min(2);
                ("stats_chart_style", self.stats_chart_label())
            }
            (3, _) => {
                self.settings_value = value;
                (
                    "interaction_mode",
                    ["auto", "plan"].get(value).copied().map_or("auto", |v| v),
                )
            }
            (4, 0) => {
                self.setting_color = value;
                (
                    "color_policy",
                    ["auto", "always", "never"]
                        .get(value)
                        .copied()
                        .map_or("auto", |v| v),
                )
            }
            (4, _) => {
                self.setting_motion = value == 1;
                (
                    "reduce_motion",
                    if self.setting_motion { "true" } else { "false" },
                )
            }
            _ => return GalleryEffect::Consumed,
        };
        self.changed();
        GalleryEffect::SettingSelected { key, value: label }
    }
    fn activate_selected(&mut self) -> GalleryEffect {
        match self.page {
            GalleryPage::Preview => SCENARIOS
                .get(self.selected)
                .map(|(name, _)| GalleryEffect::LoadScenario(name))
                .unwrap_or(GalleryEffect::Consumed),
            GalleryPage::Model => self
                .selected_model()
                .map(GalleryEffect::ModelSelected)
                .unwrap_or(GalleryEffect::Consumed),
            GalleryPage::Sessions => self
                .visible_session(self.selected)
                .map(|(title, owner, _)| GalleryEffect::ViewedConversation {
                    scenario: "conversation",
                    title,
                    owner,
                })
                .unwrap_or(GalleryEffect::Consumed),
            GalleryPage::Agents => {
                if let Some(row) = self.agent_fixture_rows().get(self.selected) {
                    GalleryEffect::ViewedConversation {
                        scenario: "conversation",
                        title: row.task,
                        owner: row.name,
                    }
                } else {
                    let pair = match self.selected {
                        0 => ("agents-working", "Parallel implementation", "agent-1"),
                        1 => ("agents-mixed", "Validation review", "agent-2"),
                        2 => ("agents-completed", "Documentation pass", "agent-3"),
                        _ => ("agents-many", "Agent roster", "demo"),
                    };
                    GalleryEffect::LoadScenario(pair.0)
                }
            }
            GalleryPage::Settings => self.activate_settings(),
            GalleryPage::ProviderPicker => {
                self.model_provider = self.selected.min(PROVIDERS.len().saturating_sub(1));
                self.open_child(GalleryPage::Model);
                GalleryEffect::Consumed
            }
            GalleryPage::Providers => {
                let add_index = PROVIDERS
                    .len()
                    .saturating_add(self.configured_providers.len());
                if self.selected == add_index {
                    self.provider_draft = ProviderDraft {
                        label: String::new(),
                        endpoint: String::new(),
                        credential_source: String::new(),
                        model_id: String::new(),
                    };
                    self.provider_field = 0;
                    self.open_child(GalleryPage::ProviderEdit);
                    return GalleryEffect::Consumed;
                }
                if let Some((label, endpoint, credential)) = PROVIDERS.get(self.selected) {
                    self.provider_draft = ProviderDraft {
                        label: (*label).to_owned(),
                        endpoint: (*endpoint).to_owned(),
                        credential_source: (*credential).to_owned(),
                        model_id: String::new(),
                    };
                    self.provider_field = 0;
                    self.open_child(GalleryPage::ProviderEdit);
                } else if let Some(provider) = self
                    .configured_providers
                    .get(self.selected.saturating_sub(PROVIDERS.len()))
                {
                    self.provider_draft = provider.clone();
                    self.provider_field = 0;
                    self.open_child(GalleryPage::ProviderEdit);
                }
                GalleryEffect::Consumed
            }
            GalleryPage::Onboarding => {
                self.onboarding_step = self.onboarding_step.saturating_add(1).min(3);
                self.changed();
                GalleryEffect::Consumed
            }
            GalleryPage::Transcript => {
                self.transcript_expanded = !self.transcript_expanded;
                self.changed();
                GalleryEffect::Consumed
            }
            _ => GalleryEffect::Consumed,
        }
    }
    pub fn handle_key(&mut self, key: KeyEvent) -> GalleryEffect {
        if key.code == KeyCode::Esc {
            return self.go_back();
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            if self.page == GalleryPage::Usage
                && key.modifiers.contains(KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('o')
            {
                self.usage_details = !self.usage_details;
                self.changed();
            } else if self.page == GalleryPage::Stats
                && key.modifiers.contains(KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('o')
            {
                self.stats_details = !self.stats_details;
                self.changed();
            } else {
                return GalleryEffect::Consumed;
            }
            return GalleryEffect::Consumed;
        }
        match self.page {
            GalleryPage::Model => match key.code {
                KeyCode::Up => {
                    self.model = self.model.saturating_sub(1);
                    self.changed()
                }
                KeyCode::Down => {
                    self.model = self.model.saturating_add(1).min(MODELS.len());
                    self.changed()
                }
                KeyCode::Left => {
                    self.effort = self.effort.saturating_sub(1);
                    self.changed()
                }
                KeyCode::Right => {
                    self.effort = self
                        .effort
                        .saturating_add(1)
                        .min(Effort::ALL.len().saturating_sub(1));
                    self.changed()
                }
                KeyCode::Char('p') => self.open_child(GalleryPage::ProviderPicker),
                KeyCode::Enter => {
                    if self.model_provider != 0 {
                        if let Some((label, endpoint, credential)) =
                            PROVIDERS.get(self.model_provider)
                        {
                            self.provider_draft.label = (*label).to_owned();
                            self.provider_draft.endpoint = (*endpoint).to_owned();
                            self.provider_draft.credential_source = (*credential).to_owned();
                            self.provider_draft.model_id.clear();
                        }
                        self.open_child(GalleryPage::ProviderEdit);
                        return GalleryEffect::Consumed;
                    }
                    if self.model >= MODELS.len() {
                        self.open_child(GalleryPage::ProviderPicker);
                        return GalleryEffect::Consumed;
                    }
                    return self.activate_selected();
                }
                _ => {}
            },
            GalleryPage::Preview
            | GalleryPage::Sessions
            | GalleryPage::Providers
            | GalleryPage::ProviderPicker
            | GalleryPage::Agents => match key.code {
                KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    self.changed()
                }
                KeyCode::Down => {
                    self.selected = self
                        .selected
                        .saturating_add(1)
                        .min(self.line_count().saturating_sub(1));
                    self.changed()
                }
                KeyCode::Enter => return self.activate_selected(),
                KeyCode::Char(c) if self.page == GalleryPage::Sessions => {
                    if !c.is_control() && self.query.chars().count() < MAX_SESSION_QUERY_CHARS {
                        self.query.push(c);
                    }
                    self.selected = 0;
                    self.changed()
                }
                KeyCode::Backspace if self.page == GalleryPage::Sessions => {
                    self.query.pop();
                    self.selected = 0;
                    self.changed()
                }
                _ => {}
            },
            GalleryPage::Stats => match key.code {
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    self.stats_tab = (self.stats_tab + 1) % 2;
                    self.stats_bucket = None;
                    self.changed()
                }
                KeyCode::Char('r') => {
                    self.stats_range = (self.stats_range + 1) % 3;
                    self.stats_bucket = None;
                    self.changed()
                }
                KeyCode::Char('d') => {
                    self.stats_demo = !self.stats_demo;
                    self.stats_bucket = None;
                    self.changed()
                }
                KeyCode::Char('c') if self.stats_tab == 1 => {
                    self.stats_chart = (self.stats_chart + 1) % 3;
                    self.changed()
                }
                KeyCode::Char(']') => {
                    let count = stats_range_bounds(self.stats_range)
                        .1
                        .saturating_sub(stats_range_bounds(self.stats_range).0);
                    self.stats_bucket =
                        Some(self.stats_bucket.map_or(count.saturating_sub(1), |i| {
                            i.saturating_add(1).min(count.saturating_sub(1))
                        }));
                    self.changed()
                }
                KeyCode::Char('[') => {
                    self.stats_bucket = Some(self.stats_bucket.map_or(0, |i| i.saturating_sub(1)));
                    self.changed()
                }
                KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    self.changed()
                }
                KeyCode::Down => {
                    self.selected = self.selected.saturating_add(1).min(2);
                    self.changed()
                }
                KeyCode::Enter => return GalleryEffect::Close,
                _ => {}
            },
            GalleryPage::Usage => match key.code {
                KeyCode::Enter => return GalleryEffect::Close,
                KeyCode::Char('d') => {
                    self.usage_demo = !self.usage_demo;
                    self.changed()
                }
                KeyCode::Char(' ') => {
                    self.usage_details = !self.usage_details;
                    self.changed()
                }
                KeyCode::Up => {
                    self.selected = self.selected.saturating_sub(1);
                    self.changed()
                }
                KeyCode::Down => {
                    self.selected = self.selected.saturating_add(1).min(2);
                    self.changed()
                }
                _ => {}
            },
            GalleryPage::Settings => match key.code {
                KeyCode::Up | KeyCode::Down if self.settings_focused => {
                    let n = SETTINGS_CATEGORIES.len();
                    self.settings_category = if key.code == KeyCode::Up {
                        self.settings_category.saturating_add(n).saturating_sub(1) % n
                    } else {
                        (self.settings_category + 1) % n
                    };
                    self.selected = self.settings_category;
                    self.changed()
                }
                KeyCode::Tab | KeyCode::Right if self.settings_focused => {
                    self.settings_focused = false;
                    self.settings_row = 0;
                    self.settings_value = self.settings_current_value();
                    self.changed()
                }
                KeyCode::Enter => return self.activate_settings(),
                KeyCode::Left | KeyCode::BackTab if !self.settings_focused => {
                    self.settings_focused = true;
                    self.changed()
                }
                KeyCode::Up | KeyCode::Down if !self.settings_focused => {
                    let n = self.settings_row_count();
                    self.settings_row = if key.code == KeyCode::Up {
                        self.settings_row.saturating_add(n).saturating_sub(1) % n
                    } else {
                        (self.settings_row + 1) % n
                    };
                    self.settings_value = self.settings_current_value();
                    self.changed()
                }
                KeyCode::Left | KeyCode::Right if !self.settings_focused => {
                    let count = self.settings_value_count();
                    self.settings_value = if key.code == KeyCode::Left {
                        self.settings_value.saturating_add(count).saturating_sub(1) % count
                    } else {
                        (self.settings_value + 1).min(count.saturating_sub(1))
                    };
                    self.changed()
                }
                _ => {}
            },
            GalleryPage::Onboarding => match key.code {
                KeyCode::Enter | KeyCode::Right => {
                    self.onboarding_step = self.onboarding_step.saturating_add(1).min(3);
                    self.changed()
                }
                KeyCode::Left => {
                    self.onboarding_step = self.onboarding_step.saturating_sub(1);
                    self.changed()
                }
                _ => {}
            },
            GalleryPage::Transcript => match key.code {
                KeyCode::Enter | KeyCode::Char(' ') => {
                    self.transcript_expanded = !self.transcript_expanded;
                    self.changed()
                }
                _ => {}
            },
            GalleryPage::Approval => match key.code {
                KeyCode::Up | KeyCode::Down => {
                    self.selected = if self.selected == 0 { 1 } else { 0 };
                    self.changed()
                }
                KeyCode::Enter => return GalleryEffect::ApprovalDecision(self.selected == 0),
                _ => {}
            },
            GalleryPage::ProviderEdit => match key.code {
                KeyCode::Tab | KeyCode::Down => {
                    self.provider_field = (self.provider_field + 1) % 5;
                    self.changed()
                }
                KeyCode::Up => {
                    self.provider_field = self.provider_field.saturating_sub(1);
                    self.changed()
                }
                KeyCode::Enter => {
                    if self.provider_field == 4 {
                        if self.provider_draft.label.trim().is_empty() {
                            return GalleryEffect::Consumed;
                        }
                        let draft = self.provider_draft.clone();
                        if let Some(existing) = self
                            .configured_providers
                            .iter_mut()
                            .find(|provider| provider.label == draft.label)
                        {
                            *existing = draft.clone();
                        } else {
                            self.configured_providers.push(draft.clone());
                        }
                        return GalleryEffect::ProviderSaved(draft);
                    }
                    self.provider_field = (self.provider_field + 1).min(4);
                    self.changed()
                }
                KeyCode::Char(c) => self.append_provider_char(c),
                KeyCode::Backspace => {
                    let target = match self.provider_field {
                        0 => Some(&mut self.provider_draft.label),
                        1 => Some(&mut self.provider_draft.endpoint),
                        2 => Some(&mut self.provider_draft.credential_source),
                        _ => Some(&mut self.provider_draft.model_id),
                    };
                    if let Some(text) = target {
                        text.pop();
                    }
                    self.changed()
                }
                _ => {}
            },
            GalleryPage::Welcome => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Char('p')) {
                    self.open_child(GalleryPage::Preview)
                }
            }
            GalleryPage::Help | GalleryPage::Status | GalleryPage::Diff => {
                if key.code == KeyCode::Enter {
                    return GalleryEffect::Close;
                }
            }
        }
        GalleryEffect::Consumed
    }
    pub fn render(&mut self, frame: &mut Frame<'_>, viewport: Rect, options: RenderOptions) {
        self.setting_ascii = options.ascii;
        let (body, start) = self.layout(viewport);
        if viewport.width < 12 || viewport.height < 5 {
            return;
        }
        let (background, foreground, muted, accent, rule) = match options.colors {
            polaris_ui_cli::screen::ColorMode::TrueColor => (
                Color::Rgb(0, 0, 0),
                Color::Rgb(232, 232, 232),
                Color::Rgb(133, 133, 133),
                Color::Rgb(118, 185, 0),
                Color::Rgb(52, 52, 52),
            ),
            polaris_ui_cli::screen::ColorMode::Terminal => (
                Color::Black,
                Color::White,
                Color::DarkGray,
                Color::Green,
                Color::DarkGray,
            ),
            polaris_ui_cli::screen::ColorMode::Ansi => (
                Color::Black,
                Color::White,
                Color::DarkGray,
                Color::Green,
                Color::DarkGray,
            ),
            polaris_ui_cli::screen::ColorMode::NoColor => (
                Color::Reset,
                Color::Reset,
                Color::Reset,
                Color::Reset,
                Color::Reset,
            ),
        };
        let base = Style::default().fg(foreground).bg(background);
        let dim = Style::default().fg(muted).bg(background);
        let chosen = Style::default()
            .fg(accent)
            .bg(background)
            .add_modifier(Modifier::BOLD);
        frame.render_widget(Clear, viewport);
        frame.render_widget(Block::default().style(base), viewport);
        frame.render_widget(
            Paragraph::new(self.title()).style(base),
            Rect::new(viewport.x, viewport.y, viewport.width, 1),
        );
        let rule_text = if options.ascii { "-" } else { "─" }.repeat(usize::from(viewport.width));
        frame.render_widget(
            Paragraph::new(rule_text).style(Style::default().fg(rule).bg(background)),
            Rect::new(viewport.x, viewport.y.saturating_add(1), viewport.width, 1),
        );
        let lines = self.content(body, start, options, base, dim, chosen);
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).style(base),
            body,
        );
    }
    fn title(&self) -> &'static str {
        match self.page {
            GalleryPage::Welcome => "Polaris · Welcome",
            GalleryPage::Preview => "Preview gallery",
            GalleryPage::Model => "Choose a model",
            GalleryPage::Providers => "Providers",
            GalleryPage::ProviderEdit => "Provider configuration",
            GalleryPage::ProviderPicker => "Choose provider",
            GalleryPage::Sessions => "Session history",
            GalleryPage::Status => "Session status",
            GalleryPage::Usage => "Usage",
            GalleryPage::Stats => "Statistics",
            GalleryPage::Diff => "Review changes",
            GalleryPage::Transcript => "Transcript",
            GalleryPage::Onboarding => "Welcome to Polaris",
            GalleryPage::Approval => "Tool approval · demo",
            GalleryPage::Help => "Help & shortcuts",
            GalleryPage::Settings => "Settings",
            GalleryPage::Agents => "Agent activity · demo",
        }
    }
    fn content(
        &self,
        body: Rect,
        start: usize,
        options: RenderOptions,
        base: Style,
        dim: Style,
        chosen: Style,
    ) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        let mut push = |text: String, style: Style| out.push(Line::styled(text, style));
        match self.page {
            GalleryPage::Welcome => {
                push("A local, deterministic Polaris preview.".to_owned(), chosen);
                push(
                    "No provider is connected. Nothing is sent or saved.".to_owned(),
                    dim,
                );
                push("Enter or p · Browse screens and scenarios".to_owned(), base);
            }
            GalleryPage::Preview => {
                push(
                    "↑/↓ select · Enter loads a labeled fixture · Esc closes".to_owned(),
                    dim,
                );
                let visible = usize::from(body.height.saturating_sub(1));
                for (idx, (name, desc)) in SCENARIOS.iter().enumerate().skip(start).take(visible) {
                    let mark = if idx == self.selected { ">" } else { " " };
                    push(
                        format!("{mark} {name:<20} {desc}"),
                        if idx == self.selected { chosen } else { base },
                    );
                }
            }
            GalleryPage::Model => {
                if self.model_provider == 0 {
                    push(
                        "NVIDIA reference catalog · model names are not an availability check"
                            .to_owned(),
                        dim,
                    );
                    for (idx, (id, name, provider)) in MODELS.iter().enumerate() {
                        let mark = if idx == self.model { ">" } else { " " };
                        push(
                            format!("{mark} {name} · {provider}"),
                            if idx == self.model { chosen } else { base },
                        );
                        push(format!("    {id}"), dim);
                    }
                    let mark = if self.model >= MODELS.len() { ">" } else { " " };
                    push(
                        format!("{mark} Change provider  (p / Enter)"),
                        if self.model >= MODELS.len() {
                            chosen
                        } else {
                            base
                        },
                    );
                    push(
                        format!(
                            "Reasoning effort: {}  (←/→)",
                            Effort::ALL
                                .get(self.effort)
                                .copied()
                                .unwrap_or(Effort::High)
                                .name()
                        ),
                        chosen,
                    );
                    push(
                        "Enter applies this reference to the demo session.".to_owned(),
                        dim,
                    );
                } else {
                    let provider = PROVIDERS
                        .get(self.model_provider)
                        .map_or("provider", |row| row.0);
                    push(
                        format!("{provider} has no bundled model catalog in this preview."),
                        dim,
                    );
                    push(
                        "Enter opens the in-memory provider reference form.".to_owned(),
                        chosen,
                    );
                    push(
                        "No provider availability or model list was queried.".to_owned(),
                        dim,
                    );
                }
            }
            GalleryPage::Providers | GalleryPage::ProviderPicker => {
                push(
                    "Reference provider registry · no credentials loaded or saved".to_owned(),
                    dim,
                );
                let count = self.line_count();
                let visible = usize::from(body.height.saturating_sub(1));
                for idx in start..start.saturating_add(visible).min(count) {
                    let mark = if idx == self.selected { ">" } else { " " };
                    if let Some((name, endpoint, _credential)) = PROVIDERS.get(idx) {
                        push(
                            format!("{mark} {name} · {endpoint}"),
                            if idx == self.selected { chosen } else { base },
                        );
                    } else if self.page == GalleryPage::Providers
                        && idx
                            < PROVIDERS
                                .len()
                                .saturating_add(self.configured_providers.len())
                    {
                        if let Some(provider) = self
                            .configured_providers
                            .get(idx.saturating_sub(PROVIDERS.len()))
                        {
                            push(
                                format!(
                                    "{mark} {} · {} [in-memory]",
                                    provider.label, provider.endpoint
                                ),
                                if idx == self.selected { chosen } else { base },
                            );
                        }
                    } else if self.page == GalleryPage::Providers {
                        push(
                            format!("{mark} Add provider reference"),
                            if idx == self.selected { chosen } else { base },
                        );
                    }
                }
            }
            GalleryPage::ProviderEdit => {
                push(
                    "In-memory reference form · never enter a secret here.".to_owned(),
                    dim,
                );
                for (idx, (label, value)) in [
                    ("Provider", self.provider_draft.label.as_str()),
                    ("Endpoint", self.provider_draft.endpoint.as_str()),
                    (
                        "Credential source",
                        self.provider_draft.credential_source.as_str(),
                    ),
                    ("Model ID", self.provider_draft.model_id.as_str()),
                    ("Apply in memory", ""),
                ]
                .iter()
                .enumerate()
                {
                    let mark = if idx == self.provider_field { ">" } else { " " };
                    push(
                        format!("{mark} {label}: {value}"),
                        if idx == self.provider_field {
                            chosen
                        } else {
                            base
                        },
                    );
                }
                push(
                    "Tab / ↑↓ moves fields · type to edit · Enter advances or applies in memory"
                        .to_owned(),
                    dim,
                );
                if self.provider_draft.label.trim().is_empty() {
                    push(
                        "Provider name is required before applying this draft.".to_owned(),
                        dim,
                    );
                }
            }
            GalleryPage::Sessions => {
                push(
                    format!(
                        "Search title or provider: {}  ·  Enter opens fixture",
                        self.query
                    ),
                    dim,
                );
                let visible = usize::from(body.height.saturating_sub(1));
                for idx in start..start.saturating_add(visible).min(self.session_count()) {
                    if let Some((title, owner, date)) = self.visible_session(idx) {
                        let mark = if idx == self.selected { ">" } else { " " };
                        push(
                            format!("{mark} {title} · {owner} · {date}"),
                            if idx == self.selected { chosen } else { base },
                        );
                    }
                }
                if self.session_count() == 0 {
                    push("No matching demo sessions.".to_owned(), dim);
                }
            }
            GalleryPage::Status => {
                push("LOCAL DEMO SESSION".to_owned(), chosen);
                push("Workspace: preview fixture".to_owned(), base);
                push("Backend: not connected".to_owned(), base);
                push("Model: static reference only".to_owned(), base);
                push("No files modified by this screen.".to_owned(), dim);
            }
            GalleryPage::Usage => {
                drop(push);
                out.extend(usage_fixture_lines(
                    body,
                    self.usage_demo,
                    self.usage_details,
                    self.usage_bar_style(),
                    options,
                    base,
                    dim,
                ));
            }
            GalleryPage::Stats => {
                drop(push);
                out.push(Line::from(vec![
                    Span::styled("Overview", if self.stats_tab == 0 { chosen } else { base }),
                    Span::styled("   ", base),
                    Span::styled("Models", if self.stats_tab == 1 { chosen } else { base }),
                    Span::styled("   [", dim),
                    Span::styled(
                        "All time",
                        if self.stats_range == 0 { chosen } else { base },
                    ),
                    Span::styled("] [", dim),
                    Span::styled(
                        "Last 7 days",
                        if self.stats_range == 1 { chosen } else { base },
                    ),
                    Span::styled("] [", dim),
                    Span::styled(
                        "Last 30 days",
                        if self.stats_range == 2 { chosen } else { base },
                    ),
                    Span::styled("]  ·  ", dim),
                    Span::styled(
                        if self.stats_demo {
                            "DEMO DATA"
                        } else {
                            "Unavailable"
                        },
                        dim,
                    ),
                ]));
                let mut push = |text: String, style: Style| out.push(Line::styled(text, style));
                push("Tab changes tabs · click a range or press r · [ ] inspect a day · Ctrl+O details · c changes chart style on Models".to_owned(),dim);
                if !self.stats_demo {
                    push("Unavailable · no provider telemetry is connected. Press d to show the labeled sample dataset.".to_owned(),dim);
                } else {
                    let summary = stats_summary(self.stats_range);
                    if self.stats_tab == 0 {
                        push(
                            format!(
                                "Total tokens  {}    Sessions  {}    Active days  {}",
                                summary.tokens, summary.sessions, summary.active_days
                            ),
                            base,
                        );
                        push(
                            format!(
                                "Activity / sessions per day · normalized demo dataset · {}–{}",
                                stats_date(0),
                                stats_date(364)
                            ),
                            dim,
                        );
                        let mut weekdays: Vec<Vec<Span<'static>>> = vec![Vec::new(); 7];
                        for day in 0..365 {
                            let row = stats_day(day);
                            let dow = (day + 6) % 7;
                            let glyph = match row.sessions {
                                0 => {
                                    if options.ascii {
                                        '.'
                                    } else {
                                        ' '
                                    }
                                }
                                1 => {
                                    if options.ascii {
                                        '+'
                                    } else {
                                        '▂'
                                    }
                                }
                                2..=3 => {
                                    if options.ascii {
                                        '+'
                                    } else {
                                        '▄'
                                    }
                                }
                                4..=7 => {
                                    if options.ascii {
                                        '#'
                                    } else {
                                        '▆'
                                    }
                                }
                                _ => {
                                    if options.ascii {
                                        '#'
                                    } else {
                                        '█'
                                    }
                                }
                            };
                            let color = match row.sessions {
                                0 => Color::Rgb(30, 30, 30),
                                1 => Color::Rgb(45, 70, 14),
                                2..=3 => Color::Rgb(65, 110, 14),
                                4..=7 => Color::Rgb(95, 150, 10),
                                _ => Color::Rgb(118, 185, 0),
                            };
                            let cell_style =
                                if options.colors == polaris_ui_cli::screen::ColorMode::NoColor {
                                    base
                                } else {
                                    Style::default().fg(color)
                                };
                            if let Some(line) = weekdays.get_mut(dow) {
                                line.push(Span::styled(glyph.to_string(), cell_style));
                            }
                        }
                        drop(push);
                        for (idx, mut cells) in weekdays.into_iter().enumerate() {
                            let day = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
                                .get(idx)
                                .copied()
                                .map_or("Day", |v| v);
                            let mut spans = vec![Span::styled(format!("{day} "), dim)];
                            spans.append(&mut cells);
                            out.push(Line::from(spans));
                        }
                        let mut push =
                            |text: String, style: Style| out.push(Line::styled(text, style));
                        let top_index = summary
                            .model_tokens
                            .iter()
                            .enumerate()
                            .max_by_key(|(_, value)| *value)
                            .map_or(0, |(index, _)| index);
                        let top_name = MODELS.get(top_index).map_or("Unavailable", |row| row.1);
                        let top_total = summary.model_tokens.get(top_index).copied().unwrap_or(0);
                        let (range_start, range_end) = stats_range_bounds(self.stats_range);
                        let busiest = (range_start..range_end)
                            .max_by_key(|day| stats_day(*day).sessions)
                            .unwrap_or(range_start);
                        let busiest_sessions = stats_day(busiest).sessions;
                        push(format!("Top model  {top_name} · {top_total} tokens"), base);
                        push(
                            format!(
                                "Busiest day  {} · {busiest_sessions} sessions · {} active days in range",
                                stats_date(busiest),
                                summary.active_days
                            ),
                            base,
                        );
                        if self.stats_details {
                            push(
                                "Token details · model totals from the same session records"
                                    .to_owned(),
                                base,
                            );
                            let inspected_index = self
                                .stats_bucket
                                .map_or(range_end.saturating_sub(range_start + 1), |bucket| {
                                    bucket.min(range_end.saturating_sub(range_start + 1))
                                })
                                .saturating_add(range_start);
                            let inspected = stats_day(inspected_index);
                            let inspected_tokens = inspected
                                .models
                                .iter()
                                .fold(0u64, |sum, value| sum.saturating_add(*value));
                            push(
                                format!(
                                    "Inspector · {} · {} sessions · {inspected_tokens} tokens",
                                    stats_date(inspected_index),
                                    inspected.sessions
                                ),
                                base,
                            );
                            for idx in 0..MODELS.len() {
                                let name = MODELS.get(idx).map_or("Model", |model| model.1);
                                let total = summary.model_tokens.get(idx).copied().unwrap_or(0);
                                push(format!("{name:<26} {total:>12} tokens"), dim);
                            }
                        }
                    } else {
                        let (start, end) = stats_range_bounds(self.stats_range);
                        let group = if self.stats_range == 0 { 7 } else { 1 };
                        let mut points: Vec<[u64; 3]> = Vec::new();
                        let mut cursor = start;
                        while cursor < end {
                            let stop = cursor.saturating_add(group).min(end);
                            let mut values = [0u64; 3];
                            for day in cursor..stop {
                                let row = stats_day(day);
                                for model in 0..3 {
                                    if let (Some(total), Some(value)) =
                                        (values.get_mut(model), row.models.get(model))
                                    {
                                        *total = total.saturating_add(*value);
                                    }
                                }
                            }
                            points.push(values);
                            cursor = stop;
                        }
                        let range_label = if self.stats_range == 0 {
                            "weekly totals"
                        } else {
                            "daily totals"
                        };
                        push(
                            format!(
                                "{} · {} · {} tokens · {range_label}",
                                self.stats_chart_label(),
                                self.stats_range_label(),
                                summary.tokens
                            ),
                            base,
                        );
                        let chart_width = usize::from(body.width.saturating_sub(32)).min(48);
                        let names = [
                            "Nemotron Super 49B v1.5",
                            "Nemotron Nano 9B v2",
                            "Nemotron Nano 8B v1",
                        ];
                        let panel_width = chart_width.max(1);
                        if chart_width == 0 {
                            push(
                                "Expand terminal width to view the time series.".to_owned(),
                                dim,
                            );
                        } else {
                            let bucket_peak = points
                                .iter()
                                .map(|point| {
                                    point
                                        .iter()
                                        .fold(0u64, |sum, value| sum.saturating_add(*value))
                                })
                                .max()
                                .map_or(1, |value| value.max(1));
                            let shared_scale = points
                                .iter()
                                .flat_map(|point| point.iter().copied())
                                .max()
                                .map_or(1, |value| value.max(1));
                            let palette = [
                                Color::Rgb(118, 185, 0),
                                Color::Rgb(130, 184, 200),
                                Color::Rgb(229, 180, 84),
                            ];
                            let panel_count = points.len().div_ceil(panel_width).max(1);
                            for (panel_index, panel) in points.chunks(panel_width).enumerate() {
                                let first_bucket = panel_index
                                    .saturating_mul(panel_width)
                                    .saturating_mul(group)
                                    .saturating_add(start);
                                let last_bucket = first_bucket
                                    .saturating_add(panel.len().saturating_mul(group))
                                    .saturating_sub(1)
                                    .min(end.saturating_sub(1));
                                push(
                                    format!(
                                        "Timeline panel {}/{} · {}–{}",
                                        panel_index.saturating_add(1),
                                        panel_count,
                                        stats_date(first_bucket),
                                        stats_date(last_bucket)
                                    ),
                                    dim,
                                );
                                if self.stats_chart == 0 {
                                    push(
                                        "Stacked buckets · 1 Super · 2 Nano9 · 3 Nano8 · shared scale"
                                            .to_owned(),
                                        dim,
                                    );
                                    for row in 0..5 {
                                        let mut chart_row =
                                            format!("{:>2} ", 5usize.saturating_sub(row));
                                        chart_row.push_str(&stats_stack_row(
                                            panel,
                                            bucket_peak,
                                            row,
                                            5,
                                        ));
                                        push(chart_row, base);
                                    }
                                } else {
                                    for idx in 0..3 {
                                        let name =
                                            names.get(idx).copied().map_or("Model", |value| value);
                                        let total =
                                            summary.model_tokens.get(idx).copied().unwrap_or(0);
                                        let scale = if self.stats_chart == 2 {
                                            points
                                                .iter()
                                                .filter_map(|point| point.get(idx).copied())
                                                .max()
                                                .map_or(1, |value| value.max(1))
                                        } else {
                                            shared_scale
                                        };
                                        let series =
                                            stats_sparkline(panel, idx, scale, options.ascii);
                                        let color =
                                            palette.get(idx).copied().unwrap_or(Color::Green);
                                        let style = if options.colors
                                            == polaris_ui_cli::screen::ColorMode::NoColor
                                        {
                                            base
                                        } else {
                                            base.fg(color)
                                        };
                                        push(
                                            format!("{name:<26} {total:>12} tokens  {series}"),
                                            style,
                                        );
                                    }
                                    push(
                                        if self.stats_chart == 2 {
                                            "Trends use independent model scales; every bucket is shown.".to_owned()
                                        } else {
                                            "Lines use one shared scale; every bucket is shown."
                                                .to_owned()
                                        },
                                        dim,
                                    );
                                }
                            }
                        }
                        if self.stats_details {
                            let index = self
                                .stats_bucket
                                .map_or(end.saturating_sub(start + 1), |value| {
                                    value.min(end.saturating_sub(start + 1))
                                })
                                .saturating_add(start);
                            let inspected = stats_day(index);
                            push(
                                format!(
                                    "Inspector · {} · {} sessions · {} tokens",
                                    stats_date(index),
                                    inspected.sessions,
                                    inspected
                                        .models
                                        .iter()
                                        .fold(0u64, |sum, value| sum.saturating_add(*value))
                                ),
                                base,
                            );
                            for idx in 0..3 {
                                let name = names.get(idx).copied().map_or("Model", |value| value);
                                let value = inspected.models.get(idx).copied().unwrap_or(0);
                                push(format!("{name}: {value} tokens"), dim);
                            }
                        }
                    }
                }
            }
            GalleryPage::Diff => {
                push(
                    "DEMO PATCH · no repository file is modified".to_owned(),
                    dim,
                );
                for text in [
                    "diff --git a/src/parser.rs b/src/parser.rs",
                    "@@ -18,2 +18,4 @@",
                    " fn parse(input: &str) -> Result<Node> {",
                    "+    let input = input.trim();",
                    "+    validate_delimiters(input)?;",
                    "     parse_node(input)",
                    " }",
                ] {
                    push(
                        text.to_owned(),
                        if text.starts_with('+') {
                            Style::default().fg(Color::Green)
                        } else {
                            base
                        },
                    );
                }
            }
            GalleryPage::Transcript => {
                push(
                    "Local transcript fixture · Enter expands tool output".to_owned(),
                    dim,
                );
                push(
                    "User  Add validation for malformed input".to_owned(),
                    chosen,
                );
                push(
                    "Assistant  I will inspect the parser and add a focused check.".to_owned(),
                    base,
                );
                push("Tool  read_file(src/parser.rs)".to_owned(), dim);
                if self.transcript_expanded {
                    push(
                        "  fn parse(input: &str) -> Result<Node> { parse_node(input) }".to_owned(),
                        dim,
                    );
                    push("  output: source fixture · no tool ran".to_owned(), dim);
                }
                push(
                    "Assistant  The fixture shows a validation change.".to_owned(),
                    base,
                );
            }
            GalleryPage::Approval => {
                push(
                    "DEMO TOOL APPROVAL · no command executes and no file changes".to_owned(),
                    dim,
                );
                push(
                    "Request: apply a small parser validation patch [fixture]".to_owned(),
                    base,
                );
                push(
                    "Choose a mock decision; this only updates the local demo notice.".to_owned(),
                    dim,
                );
                for (index, label) in ["Approve request", "Deny request"].iter().enumerate() {
                    let selected = index == self.selected;
                    push(
                        format!("{} {label}", if selected { ">" } else { " " }),
                        if selected { chosen } else { base },
                    );
                }
                push(
                    "↑/↓ choose · Enter records the mock decision · Esc returns".to_owned(),
                    dim,
                );
            }
            GalleryPage::Onboarding => {
                push(format!("Step {} of 4", self.onboarding_step + 1), chosen);
                let title = [
                    "Choose a provider",
                    "Select a model",
                    "Set a workspace",
                    "Ready to explore",
                ]
                .get(self.onboarding_step)
                .copied()
                .map_or("Ready to explore", |v| v);
                push(title.to_owned(), base);
                push(
                    "This is a local walkthrough. Provider setup is not performed.".to_owned(),
                    dim,
                );
                push("←/→ or Enter to move through steps".to_owned(), dim);
            }
            GalleryPage::Help => {
                push("Commands".to_owned(), chosen);
                for text in [
                    "/preview  Browse scenarios",
                    "/model    Select provider, model and effort",
                    "/providers  Inspect provider references",
                    "/sessions  Search session fixtures",
                    "/usage  Inspect sample or unavailable metrics",
                    "/stats  Inspect the normalized demo dataset",
                    "/diff  Review a patch fixture",
                    "/settings  Change in-memory preview preferences",
                    "/agents  Inspect agent fixtures",
                    "/onboarding  Walk through local setup",
                    "/help  Show this guide",
                ] {
                    push(text.to_owned(), base);
                }
            }
            GalleryPage::Settings => {
                push(
                    format!(
                        "{} category · changes stay in this demo",
                        self.settings_category_label()
                    ),
                    dim,
                );
                for (idx, category) in SETTINGS_CATEGORIES
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(usize::from(body.height.saturating_sub(1)))
                {
                    let mark = if idx == self.settings_category {
                        ">"
                    } else {
                        " "
                    };
                    push(
                        format!("{mark} {category}"),
                        if idx == self.settings_category && self.settings_focused {
                            chosen
                        } else {
                            dim
                        },
                    );
                }
                let (label, value) = match (self.settings_category, self.settings_row) {
                    (0, 0) => (
                        "Theme",
                        if if self.settings_focused {
                            self.setting_theme
                        } else {
                            self.settings_value
                        } == 0
                        {
                            "Dark"
                        } else {
                            "Terminal"
                        },
                    ),
                    (0, _) => (
                        "Text",
                        if if self.settings_focused {
                            usize::from(options.ascii)
                        } else {
                            self.settings_value
                        } == 1
                        {
                            "ASCII"
                        } else {
                            "Unicode"
                        },
                    ),
                    (1, _) => (
                        "Usage bar style",
                        UsageBarStyle::ALL
                            .get(if self.settings_focused {
                                self.usage_style
                            } else {
                                self.settings_value
                            })
                            .copied()
                            .unwrap_or(UsageBarStyle::Strip)
                            .label(),
                    ),
                    (2, _) => (
                        "Statistics chart style",
                        ["Stacked", "Lines", "Trends"]
                            .get(if self.settings_focused {
                                self.stats_chart
                            } else {
                                self.settings_value
                            })
                            .copied()
                            .map_or("Stacked", |v| v),
                    ),
                    (3, _) => (
                        "Interaction mode",
                        ["Auto", "Plan"]
                            .get(self.settings_value % 2)
                            .copied()
                            .map_or("Auto", |v| v),
                    ),
                    (4, 0) => (
                        "Color policy",
                        ["Auto", "Always", "Never"]
                            .get(if self.settings_focused {
                                0
                            } else {
                                self.settings_value
                            })
                            .copied()
                            .map_or("Auto", |v| v),
                    ),
                    (4, _) => (
                        "Motion",
                        if if self.settings_focused {
                            self.setting_motion
                        } else {
                            self.settings_value == 1
                        } {
                            "Reduced"
                        } else {
                            "Normal"
                        },
                    ),
                    _ => ("Providers", "Manage provider references"),
                };
                if self.settings_category == 1 {
                    for (idx, style) in UsageBarStyle::ALL.iter().copied().enumerate() {
                        let selected = idx == self.settings_row;
                        push(
                            format!(
                                "{} {}  {}",
                                if selected { ">" } else { " " },
                                style.label(),
                                usage_bar(
                                    style,
                                    42,
                                    100,
                                    usize::from(body.width.saturating_sub(20)),
                                    options.ascii
                                )
                            ),
                            if selected { chosen } else { base },
                        );
                    }
                } else if self.settings_category == 2 {
                    for (idx, chart) in ["Stacked", "Lines", "Trends"].iter().enumerate() {
                        let selected = idx == self.settings_row;
                        push(
                            format!(
                                "{} Statistics chart style: {chart}",
                                if selected { ">" } else { " " }
                            ),
                            if selected { chosen } else { base },
                        );
                    }
                } else {
                    push(format!("{}: {value}", label), chosen);
                }
                push(
                    "Tab enters/leaves values · ↑/↓ categories or rows · Enter applies in memory"
                        .to_owned(),
                    dim,
                );
            }
            GalleryPage::Agents => {
                if let Some(scenario) = self.agent_scenario {
                    push(
                        format!(
                            "AGENT FIXTURE · {scenario} · {} records [demo]",
                            self.agent_fixture_rows().len()
                        ),
                        dim,
                    );
                    for (idx, row) in self
                        .agent_fixture_rows()
                        .iter()
                        .enumerate()
                        .skip(start)
                        .take(usize::from(body.height).saturating_sub(2))
                    {
                        let mark = if idx == self.selected { ">" } else { " " };
                        let status = row.status.label();
                        let marker = if options.ascii {
                            match row.status {
                                AgentStatus::Queued => "q",
                                AgentStatus::Working => "o",
                                AgentStatus::Waiting => "w",
                                AgentStatus::Paused => "p",
                                AgentStatus::Completed => "c",
                                AgentStatus::Failed => "!",
                                AgentStatus::Cancelled => "x",
                            }
                        } else {
                            match row.status {
                                AgentStatus::Queued => "◷",
                                AgentStatus::Working => "◯",
                                AgentStatus::Waiting => "◌",
                                AgentStatus::Paused => "Ⅱ",
                                AgentStatus::Completed => "✓",
                                AgentStatus::Failed => "!",
                                AgentStatus::Cancelled => "×",
                            }
                        };
                        let tokens = match row.tokens {
                            Some(value) => format!("{value} tokens"),
                            None => "—".to_owned(),
                        };
                        let failure = match row.failure_note {
                            Some(_) => " · Sample failure [demo]".to_owned(),
                            None => String::new(),
                        };
                        let content = if body.width >= 100 && row.failure_note.is_none() {
                            format!(
                                "{mark} {marker} {status:<9} · {} · {} · {}s · {tokens} · {}",
                                row.name, row.task, row.elapsed_seconds, row.effort
                            )
                        } else {
                            format!(
                                "{mark} {marker} {status:<9} · {} · {}s · {tokens}{failure}",
                                row.name, row.elapsed_seconds
                            )
                        };
                        push(content, if idx == self.selected { chosen } else { base });
                    }
                    push(
                        "Enter views this agent conversation · Esc returns".to_owned(),
                        dim,
                    );
                } else {
                    push(
                        "STATIC THREAD FIXTURES · no agents are running".to_owned(),
                        dim,
                    );
                    for (idx, title) in [
                        "Working · 2 active agents [demo]",
                        "Mixed · 7 status states [demo]",
                        "Completed · 2 completed agents [demo]",
                        "Many · 9 scrollable agents [demo]",
                    ]
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(usize::from(body.height.saturating_sub(1)))
                    {
                        let mark = if idx == self.selected { ">" } else { " " };
                        push(
                            format!("{mark} {title}"),
                            if idx == self.selected { chosen } else { base },
                        );
                    }
                }
            }
        }
        if out.len() > usize::from(body.height) {
            out.truncate(usize::from(body.height));
        }
        out
    }
}

#[cfg(test)]
#[path = "../../../tests/apps/polaris-cli/src/demo_gallery.rs"]
mod tests;
