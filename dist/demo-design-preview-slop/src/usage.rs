//! Usage presentation and explicitly labeled preview data; no provider requests.
use crate::{
    app::App,
    interaction::{HitAction, UiMap},
    theme::{Theme, ThemeMode},
    ui,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum UsageBarStyle {
    #[serde(rename = "ticks")]
    FineTicks,
    #[default]
    #[serde(rename = "strip")]
    ContinuousStrip,
    #[serde(rename = "segments")]
    SegmentedRail,
}
impl UsageBarStyle {
    pub const ALL: [Self; 3] = [Self::FineTicks, Self::ContinuousStrip, Self::SegmentedRail];
    pub fn label(self) -> &'static str {
        match self {
            Self::FineTicks => "Ticks",
            Self::ContinuousStrip => "Strip",
            Self::SegmentedRail => "Segments",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UsageAction {
    Details,
}
#[derive(Clone, Debug)]
pub struct UsageViewState {
    pub details: bool,
    pub page_rows: usize,
}
impl Default for UsageViewState {
    fn default() -> Self {
        Self {
            details: false,
            page_rows: 10,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageSource {
    Unavailable,
    Demo,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Money {
    pub minor: u64,
    pub currency: &'static str,
}
#[derive(Clone, Debug)]
pub struct ModelUsage {
    pub name: String,
    pub input: u64,
    pub output: u64,
    pub cost: Option<Money>,
    pub cache_read: u64,
    pub cache_write: u64,
}
#[derive(Clone, Debug)]
pub struct SubagentUsage {
    pub name: String,
    pub calls: u64,
    pub input: u64,
    pub output: u64,
    pub model: String,
}
#[derive(Clone, Debug)]
pub struct Gauge {
    pub label: String,
    pub used: Option<u64>,
    pub capacity: Option<u64>,
    pub unit: &'static str,
    pub reset: Option<String>,
}
impl Gauge {
    pub fn ratio(&self) -> Option<(u64, u64)> {
        Some((self.used?, self.capacity.filter(|n| *n > 0)?))
    }
    pub fn percentage(&self) -> String {
        match self.ratio() {
            None => "Unavailable".into(),
            Some((n, d)) if n > 0 && u128::from(n) * 100 < u128::from(d) => "<1%".into(),
            Some((n, d)) => format!("{}%", u128::from(n) * 100 / u128::from(d)),
        }
    }
    pub fn status(&self) -> &'static str {
        match self.ratio() {
            Some((n, d)) if n > d => "Over limit",
            Some((n, d)) if n == d => "Limit reached",
            Some((n, d)) if u128::from(n) * 100 >= u128::from(d) * 80 => "Near limit",
            _ => "",
        }
    }
}
#[derive(Clone, Debug)]
pub struct UsageSnapshot {
    pub source: UsageSource,
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cost: Option<Money>,
    pub wall_seconds: Option<u64>,
    pub api_seconds: Option<u64>,
    pub changes: Option<(u64, u64, u64)>,
    pub context: Gauge,
    pub limits: Vec<Gauge>,
    pub models: Option<Vec<ModelUsage>>,
    pub cache_hits: Option<(u64, u64)>,
    pub subagents: Option<Vec<SubagentUsage>>,
}
fn gauge(
    label: &str,
    used: Option<u64>,
    capacity: Option<u64>,
    unit: &'static str,
    reset: Option<&str>,
) -> Gauge {
    Gauge {
        label: label.into(),
        used,
        capacity,
        unit,
        reset: reset.map(str::to_owned),
    }
}
impl UsageSnapshot {
    pub fn unavailable() -> Self {
        Self {
            source: UsageSource::Unavailable,
            input: None,
            output: None,
            cost: None,
            wall_seconds: None,
            api_seconds: None,
            changes: None,
            context: gauge("Context", None, None, "tokens", None),
            limits: vec![
                gauge("Current window", None, None, "units", None),
                gauge("Weekly allowance", None, None, "units", None),
            ],
            models: None,
            cache_hits: None,
            subagents: None,
        }
    }
    pub fn demo() -> Self {
        let models = vec![
            ModelUsage {
                name: "Nemotron Super 49B v1.5".into(),
                input: 10_000,
                output: 2_500,
                cost: Some(Money {
                    minor: 35,
                    currency: "USD",
                }),
                cache_read: 80_000,
                cache_write: 1_600,
            },
            ModelUsage {
                name: "Nemotron Nano 9B v2".into(),
                input: 2_400,
                output: 680,
                cost: Some(Money {
                    minor: 7,
                    currency: "USD",
                }),
                cache_read: 20_000,
                cache_write: 400,
            },
        ];
        Self {
            source: UsageSource::Demo,
            input: Some(models.iter().map(|m| m.input).sum()),
            output: Some(models.iter().map(|m| m.output).sum()),
            cost: Some(Money {
                minor: models.iter().map(|m| m.cost.as_ref().unwrap().minor).sum(),
                currency: "USD",
            }),
            wall_seconds: Some(134),
            api_seconds: Some(116),
            changes: Some((2, 14, 3)),
            context: gauge("Context", Some(25_600), Some(128_000), "tokens", None),
            limits: vec![
                gauge(
                    "Current window",
                    Some(420),
                    Some(1_000),
                    "units",
                    Some("Resets in 2h 30m"),
                ),
                gauge(
                    "Weekly allowance",
                    Some(2_900),
                    Some(5_000),
                    "units",
                    Some("Resets in 4d"),
                ),
            ],
            models: Some(models),
            cache_hits: Some((90, 100)),
            subagents: Some(vec![
                SubagentUsage {
                    name: "General purpose".into(),
                    calls: 2,
                    input: 1_800,
                    output: 400,
                    model: "Nemotron Nano 9B v2".into(),
                },
                SubagentUsage {
                    name: "Explore".into(),
                    calls: 1,
                    input: 600,
                    output: 280,
                    model: "Nemotron Nano 9B v2".into(),
                },
            ]),
        }
    }
}
fn number(n: impl Into<u128>) -> String {
    let s = n.into().to_string();
    let len = s.len();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
fn optional(n: Option<u64>) -> String {
    n.map(number).unwrap_or_else(|| "Unavailable".into())
}
fn money(m: Option<&Money>) -> String {
    m.map(|m| format!("{} {}.{:02}", m.currency, m.minor / 100, m.minor % 100))
        .unwrap_or_else(|| "Unavailable".into())
}
fn duration(n: Option<u64>) -> String {
    n.map(|n| crate::work_summary::elapsed_label(std::time::Duration::from_secs(n)))
        .unwrap_or_else(|| "Unavailable".into())
}
fn text(s: impl Into<String>, style: Style) -> Line<'static> {
    Line::styled(s.into(), style)
}
fn pair(label: &str, value: impl Into<String>, t: Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<13}"), t.muted()),
        Span::styled(value.into(), t.base()),
    ])
}
fn heading(s: &str, t: Theme) -> Line<'static> {
    text(s, t.base())
}
fn ink(g: &Gauge, t: Theme, leading: bool) -> Style {
    if g.ratio().is_none() {
        return t.muted();
    }
    if !t.color {
        return t.base();
    }
    if g.ratio().is_some_and(|(n, d)| n >= d) {
        return t.error();
    }
    if !g.status().is_empty() {
        return t.warning();
    }
    if leading {
        t.base().fg(if t.rgb && t.mode == ThemeMode::Dark {
            Color::Rgb(192, 229, 140)
        } else {
            Color::LightGreen
        })
    } else {
        t.base().patch(t.selected())
    }
}
/// The same exact count ratio drives all styles; visual fill alone is clamped.
pub fn bar(g: &Gauge, style: UsageBarStyle, width: u16, t: Theme, ascii: bool) -> Line<'static> {
    let w = usize::from(width.min(55));
    if w == 0 {
        return Line::default();
    }
    let Some((used, cap)) = g.ratio() else {
        return text(ui::clip("Unavailable", w), t.muted());
    };
    let fill =
        |cells: usize| (u128::from(used.min(cap)) * cells as u128 / u128::from(cap)) as usize;
    let mut spans = Vec::new();
    match style {
        UsageBarStyle::FineTicks => {
            let pos = fill(w.saturating_sub(1));
            for x in 0..w {
                let tick = x == 0 || x == w - 1 || [1, 2, 3].iter().any(|q| x == (w - 1) * q / 4);
                let glyph = if x == pos {
                    if ascii { '+' } else { '╋' }
                } else if ascii {
                    if x < pos {
                        '#'
                    } else if tick {
                        '|'
                    } else {
                        '-'
                    }
                } else if tick {
                    '┬'
                } else if !t.color && x < pos {
                    '━'
                } else {
                    '─'
                };
                spans.push(Span::styled(
                    glyph.to_string(),
                    if x <= pos {
                        ink(g, t, x == pos)
                    } else {
                        t.rule()
                    },
                ));
            }
        }
        UsageBarStyle::ContinuousStrip => {
            let filled = fill(w);
            for x in 0..w {
                let active = x < filled;
                let glyph = if ascii {
                    if active { '#' } else { '-' }
                } else if !t.color && !active {
                    '·'
                } else {
                    '▄'
                };
                spans.push(Span::styled(
                    glyph.to_string(),
                    if active {
                        ink(g, t, x + 1 == filled)
                    } else {
                        t.rule()
                    },
                ));
            }
        }
        UsageBarStyle::SegmentedRail => {
            let cells = w.div_ceil(2);
            let filled = fill(cells);
            for x in 0..w {
                let active = x / 2 < filled;
                let glyph = if x % 2 == 1 {
                    ' '
                } else if ascii {
                    if active { '#' } else { '-' }
                } else if !t.color && !active {
                    '▯'
                } else {
                    '▮'
                };
                spans.push(Span::styled(
                    glyph.to_string(),
                    if active {
                        ink(g, t, x / 2 + 1 == filled)
                    } else {
                        t.rule()
                    },
                ));
            }
        }
    }
    Line::from(spans)
}
fn gauge_lines(
    g: &Gauge,
    style: UsageBarStyle,
    width: u16,
    t: Theme,
    ascii: bool,
    reset: bool,
) -> Vec<Line<'static>> {
    let pct = if g.ratio().is_some() {
        format!("{} used", g.percentage())
    } else {
        g.percentage()
    };
    let header = Line::from(vec![
        Span::styled(g.label.clone(), t.base()),
        Span::styled(
            format!(
                "{}{pct}",
                " ".repeat(
                    usize::from(width.min(55))
                        .saturating_sub(g.label.width() + pct.width())
                        .max(2)
                )
            ),
            ink(g, t, false),
        ),
    ]);
    if g.ratio().is_none() {
        let mut out = vec![header];
        if let Some(n) = g.used {
            out.push(text(
                format!("{} {} used; capacity unavailable", number(n), g.unit),
                t.muted(),
            ));
        } else if let Some(d) = g.capacity.filter(|d| *d > 0) {
            out.push(text(
                format!("Capacity {} {}; usage unavailable", number(d), g.unit),
                t.muted(),
            ));
        }
        if reset {
            out.push(text(
                g.reset
                    .clone()
                    .unwrap_or_else(|| "Reset unavailable".into()),
                t.muted(),
            ));
        }
        return out;
    }
    let mut out = vec![header, bar(g, style, width, t, ascii)];
    let counts = match (g.used, g.capacity.filter(|n| *n > 0)) {
        (Some(n), Some(d)) => format!("{} / {} {}", number(n), number(d), g.unit),
        (Some(n), None) => format!("{} {} used; capacity unavailable", number(n), g.unit),
        _ => "Usage unavailable".into(),
    };
    out.push(text(counts, t.muted()));
    if !g.status().is_empty() {
        out.push(text(g.status(), ink(g, t, false)));
    }
    if reset {
        out.push(text(
            g.reset
                .clone()
                .unwrap_or_else(|| "Reset unavailable".into()),
            t.muted(),
        ));
    }
    out
}
fn left_lines(data: &UsageSnapshot, app: &App, width: u16) -> Vec<Line<'static>> {
    let t = app.theme;
    let mut out = vec![
        heading("Session", t),
        pair("Total cost", money(data.cost.as_ref()), t),
        pair(
            "Tokens",
            match (data.input, data.output) {
                (None, None) => "Unavailable".into(),
                (Some(i), Some(o)) => format!(
                    "{} total / {} in / {} out",
                    number(u128::from(i) + u128::from(o)),
                    number(i),
                    number(o)
                ),
                _ => format!(
                    "{} in / {} out",
                    optional(data.input),
                    optional(data.output)
                ),
            },
            t,
        ),
        pair(
            "Duration",
            if data.wall_seconds.is_none() && data.api_seconds.is_none() {
                "Unavailable".into()
            } else {
                format!(
                    "{} wall / {} API",
                    duration(data.wall_seconds),
                    duration(data.api_seconds)
                )
            },
            t,
        ),
        pair(
            "Changes",
            data.changes
                .map(|(f, a, r)| format!("{f} files / +{a} -{r}"))
                .unwrap_or_else(|| "Unavailable".into()),
            t,
        ),
        Line::default(),
    ];
    out.extend(gauge_lines(
        &data.context,
        app.preferences.values.usage_bar_style,
        width,
        t,
        app.ascii,
        false,
    ));
    out.push(Line::default());
    out.push(heading(
        if data.source == UsageSource::Demo {
            "Limits / sample policy"
        } else {
            "Provider limits"
        },
        t,
    ));
    for g in &data.limits {
        out.extend(gauge_lines(
            g,
            app.preferences.values.usage_bar_style,
            width,
            t,
            app.ascii,
            true,
        ));
        out.push(Line::default());
    }
    out
}
fn right_lines(data: &UsageSnapshot, app: &App, width: u16) -> Vec<Line<'static>> {
    let t = app.theme;
    let mut out = vec![heading("Models", t)];
    match &data.models {
        None => out.push(text("Usage unavailable", t.muted())),
        Some(rows) if rows.is_empty() => out.push(text("No model usage recorded", t.muted())),
        Some(rows) => {
            if width >= 76 {
                out.push(text(
                    format!("{:<32} {:>11} {:>11}  Cost", "Model", "Input", "Output"),
                    t.muted(),
                ));
            }
            for m in rows {
                if width >= 76 {
                    out.push(text(
                        format!(
                            "{:<32} {:>11} {:>11}  {}",
                            ui::clip(&m.name, 32),
                            number(m.input),
                            number(m.output),
                            money(m.cost.as_ref())
                        ),
                        t.base(),
                    ));
                } else {
                    out.push(text(m.name.clone(), t.base()));
                    out.push(text(
                        format!(
                            "  {} in / {} out / {}",
                            number(m.input),
                            number(m.output),
                            money(m.cost.as_ref())
                        ),
                        t.muted(),
                    ));
                }
            }
        }
    }
    if !app.usage.details {
        return out;
    }
    out.push(Line::default());
    out.push(heading("Cache", t));
    if let Some(rows) = &data.models {
        out.push(pair(
            "Read / write",
            format!(
                "{} / {} tokens",
                number(rows.iter().map(|m| u128::from(m.cache_read)).sum::<u128>()),
                number(rows.iter().map(|m| u128::from(m.cache_write)).sum::<u128>())
            ),
            t,
        ));
    } else {
        out.push(pair("Read / write", "Unavailable", t));
    }
    out.push(pair(
        "Hit rate",
        data.cache_hits
            .filter(|(n, d)| *d > 0 && n <= d)
            .map(|(n, d)| {
                format!(
                    "{}% / {} of {} requests",
                    u128::from(n) * 100 / u128::from(d),
                    number(n),
                    number(d)
                )
            })
            .unwrap_or_else(|| "Unavailable".into()),
        t,
    ));
    out.push(Line::default());
    out.push(heading("Subagents", t));
    match &data.subagents {
        None => out.push(text("Usage unavailable", t.muted())),
        Some(rows) if rows.is_empty() => out.push(text("No subagent calls recorded", t.muted())),
        Some(rows) => {
            for r in rows {
                out.push(text(
                    format!(
                        "{} / {} {} / {} tokens",
                        r.name,
                        r.calls,
                        if r.calls == 1 { "call" } else { "calls" },
                        number(u128::from(r.input) + u128::from(r.output))
                    ),
                    t.base(),
                ));
                out.push(text(format!("  {}", r.model), t.muted()));
            }
            out.push(text("Included in session/model totals.", t.muted()));
        }
    }
    out
}
/// Wrap complete graphemes while preserving each span's style.
fn wrapped(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![];
    }
    let mut out = Vec::new();
    for line in lines {
        let mut spans = Vec::new();
        let mut col = 0;
        let line_style = line.style;
        for span in line.spans {
            for g in span.content.graphemes(true) {
                let n = UnicodeWidthStr::width(g) as u16;
                if col + n > width && col > 0 {
                    out.push(Line::from(std::mem::take(&mut spans)));
                    col = 0;
                }
                spans.push(Span::styled(g.to_owned(), line_style.patch(span.style)));
                col += n;
            }
        }
        out.push(Line::from(spans));
    }
    out
}
pub fn information(data: &UsageSnapshot, app: &App, width: u16) -> Vec<Line<'static>> {
    if width >= 104 {
        let left_width = (width - 4) / 2;
        let right_width = width - 4 - left_width;
        let left = wrapped(left_lines(data, app, left_width), left_width);
        let right = wrapped(right_lines(data, app, right_width), right_width);
        let mut out = Vec::new();
        for i in 0..left.len().max(right.len()) {
            let l = left.get(i).cloned().unwrap_or_default();
            let used = l.width();
            let mut spans = l.spans;
            spans.push(Span::raw(" ".repeat(usize::from(left_width) + 4 - used)));
            spans.extend(right.get(i).cloned().unwrap_or_default().spans);
            out.push(Line::from(spans));
        }
        out
    } else {
        let mut out = left_lines(data, app, width);
        out.extend(right_lines(data, app, width));
        wrapped(out, width)
    }
}
pub fn activate(app: &mut App, action: UsageAction) {
    match action {
        UsageAction::Details => {
            app.usage.details = !app.usage.details;
            app.scroll = 0;
        }
    }
}
pub fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::ALT) {
        return false;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if key.code == KeyCode::Char('o') {
            activate(app, UsageAction::Details);
            return true;
        }
        return false;
    }
    match key.code {
        KeyCode::Char(' ') => activate(app, UsageAction::Details),
        KeyCode::PageUp => app.scroll = app.scroll.saturating_sub(app.usage.page_rows.max(1)),
        KeyCode::PageDown => app.scroll = app.scroll.saturating_add(app.usage.page_rows.max(1)),
        KeyCode::Up => app.scroll = app.scroll.saturating_sub(1),
        KeyCode::Down => app.scroll = app.scroll.saturating_add(1),
        KeyCode::Tab | KeyCode::BackTab => {}
        _ => return false,
    }
    true
}
fn controls(area: Rect) -> (Rect, Rect, Rect) {
    let button = Rect::new(
        area.x,
        area.y,
        area.width.min(20),
        u16::from(area.height > 0),
    );
    let start = (area.y + 2).min(area.bottom());
    let info = Rect::new(
        area.x,
        start,
        area.width,
        area.bottom().saturating_sub(start + 1),
    );
    let pager = Rect::new(
        area.x,
        area.bottom().saturating_sub(1),
        area.width,
        u16::from(area.height > 0),
    );
    (button, info, pager)
}
pub fn page_rows(_app: &App, area: Rect) -> usize {
    controls(area).1.height as usize
}
pub fn max_scroll(app: &App, area: Rect) -> usize {
    let (_, info, _) = controls(area);
    let data = if app.metrics_demo {
        UsageSnapshot::demo()
    } else {
        UsageSnapshot::unavailable()
    };
    information(&data, app, info.width)
        .len()
        .saturating_sub(info.height as usize)
}
pub fn render(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let (buttons, info, pager) = controls(area);
    ui::render_control(
        frame,
        buttons,
        if app.usage.details {
            "Ctrl+O Less"
        } else {
            "Ctrl+O Details"
        },
        app.theme.muted(),
        app,
        false,
    );
    map.add_target(buttons, HitAction::Usage(UsageAction::Details));
    let data = if app.metrics_demo {
        UsageSnapshot::demo()
    } else {
        UsageSnapshot::unavailable()
    };
    let rows = information(&data, app, info.width);
    let max = rows.len().saturating_sub(info.height as usize);
    let offset = app.scroll.min(max);
    if info.height > 0 {
        frame.render_widget(
            Paragraph::new(rows.clone()).scroll((offset.min(u16::MAX as usize) as u16, 0)),
            info,
        );
    }
    let label = if max > 0 {
        format!(
            "{}-{} / {} rows",
            offset + 1,
            (offset + info.height as usize).min(rows.len()),
            rows.len()
        )
    } else if data.source == UsageSource::Unavailable {
        "No usage source connected; missing is not zero.".into()
    } else {
        "Sample policy / no provider quota claims".into()
    };
    frame.render_widget(Paragraph::new(label).style(app.theme.muted()), pager);
}
