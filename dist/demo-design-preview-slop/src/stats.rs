//! Adaptive Stats sheet. All three chart styles share normalized demo records.
use crate::{
    app::App,
    interaction::{HitAction, UiMap},
    stats_chart as charts,
    stats_data::{self as data, Dataset, ModelTotal, Summary},
    ui,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget, Wrap},
};
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChartStyle {
    #[default]
    Stacked,
    Lines,
    Trends,
}
impl ChartStyle {
    pub const ALL: [Self; 3] = [Self::Stacked, Self::Lines, Self::Trends];
    pub fn label(self) -> &'static str {
        match self {
            Self::Stacked => "Stacked",
            Self::Lines => "Lines",
            Self::Trends => "Trends",
        }
    }
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap()
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatsTab {
    #[default]
    Overview,
    Models,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatsRange {
    #[default]
    All,
    Week,
    Month,
}
impl StatsRange {
    pub const ALL: [Self; 3] = [Self::All, Self::Week, Self::Month];
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All time",
            Self::Week => "Last 7 days",
            Self::Month => "Last 30 days",
        }
    }
    pub fn bounds(self, d: &Dataset) -> (i32, i32) {
        (
            match self {
                Self::All => d.start,
                Self::Week => (d.end - 6).max(d.start),
                Self::Month => (d.end - 29).max(d.start),
            },
            d.end,
        )
    }
}
#[derive(Clone, Debug, Default)]
pub struct StatsViewState {
    pub tab: StatsTab,
    pub range: StatsRange,
    pub demo: bool,
    pub details: bool,
    pub preview: Option<ChartStyle>,
    pub inspected: Option<usize>,
    pub model: Option<String>,
    pub hover: Option<StatsAction>,
    pub page_rows: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatsAction {
    Tab(StatsTab),
    Range(StatsRange),
    Style(ChartStyle),
    Details,
    Inspect(usize),
    Model(String),
    Demo,
}
pub fn effective_style(app: &App) -> ChartStyle {
    app.stats
        .preview
        .unwrap_or(app.preferences.values.stats_chart_style)
}
pub fn activate(app: &mut App, action: StatsAction) {
    match action {
        StatsAction::Tab(tab) => {
            app.stats.tab = tab;
            app.scroll = 0;
            app.stats.inspected = None;
            app.stats.model = None;
        }
        StatsAction::Range(range) => {
            app.stats.range = range;
            app.scroll = 0;
            app.stats.inspected = None;
            app.stats.model = None;
        }
        StatsAction::Style(style) => {
            app.stats.preview = Some(style);
            app.scroll = 0;
        }
        StatsAction::Details => {
            app.stats.details = !app.stats.details;
        }
        StatsAction::Inspect(index) => app.stats.inspected = Some(index),
        StatsAction::Model(id) => {
            app.stats.model = if app.stats.model.as_ref() == Some(&id) {
                None
            } else {
                Some(id)
            };
        }
        StatsAction::Demo => {
            app.stats.demo = !app.stats.demo;
            app.scroll = 0;
            app.stats.inspected = None;
            app.stats.model = None;
        }
    }
    app.stats.hover = None;
}
pub fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('o') {
        activate(app, StatsAction::Details);
        return true;
    }
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return false;
    }
    let action = match key.code {
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
            Some(StatsAction::Tab(if app.stats.tab == StatsTab::Overview {
                StatsTab::Models
            } else {
                StatsTab::Overview
            }))
        }
        KeyCode::Char('r') => Some(StatsAction::Range(match app.stats.range {
            StatsRange::All => StatsRange::Week,
            StatsRange::Week => StatsRange::Month,
            StatsRange::Month => StatsRange::All,
        })),
        KeyCode::Char('d') => Some(StatsAction::Demo),
        KeyCode::Char('c') if app.stats.tab == StatsTab::Models => Some(StatsAction::Style(
            ChartStyle::ALL[(effective_style(app).index() + 1) % 3],
        )),
        KeyCode::Char('[' | ']') if app.stats.demo => {
            let d = data::demo();
            let (start, end) = app.stats.range.bounds(d);
            let n = if app.stats.tab == StatsTab::Overview {
                (end - start + 1) as usize
            } else {
                d.buckets(
                    start,
                    end,
                    app.stats.range == StatsRange::All,
                    &data::chart_models(&d.summary(start, end)),
                )
                .len()
            };
            let current = app
                .stats
                .inspected
                .unwrap_or(n.saturating_sub(1))
                .min(n.saturating_sub(1));
            Some(StatsAction::Inspect(if key.code == KeyCode::Char('[') {
                current.saturating_sub(1)
            } else {
                (current + 1).min(n.saturating_sub(1))
            }))
        }
        KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {
            let step = if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
                app.stats.page_rows.max(1)
            } else {
                1
            };
            app.scroll = if matches!(key.code, KeyCode::Up | KeyCode::PageUp) {
                app.scroll.saturating_sub(step)
            } else {
                app.scroll.saturating_add(step)
            };
            return true;
        }
        _ => None,
    };
    if let Some(a) = action {
        activate(app, a);
        true
    } else {
        false
    }
}
pub fn hover(app: &mut App, action: Option<&HitAction>) {
    app.stats.hover = match action {
        Some(HitAction::Stats(a @ StatsAction::Inspect(_) | a @ StatsAction::Model(_))) => {
            Some(a.clone())
        }
        _ => None,
    };
}
#[derive(Default)]
struct Doc {
    lines: Vec<Line<'static>>,
    hits: Vec<(Rect, StatsAction)>,
}
impl Doc {
    fn line(&mut self, l: impl Into<Line<'static>>) {
        self.lines.push(l.into());
    }
    fn gap(&mut self) {
        self.line("");
    }
    fn text(&mut self, text: &str, style: Style, width: u16) {
        if width == 0 {
            return;
        }
        let p = Paragraph::new(text.to_owned())
            .style(style)
            .wrap(Wrap { trim: false });
        let height = p.line_count(width).max(1) as u16;
        let mut b = Buffer::empty(Rect::new(0, 0, width, height));
        p.render(b.area, &mut b);
        for y in 0..height {
            self.line(Line::from(
                (0..width)
                    .map(|x| Span::styled(b[(x, y)].symbol().to_owned(), b[(x, y)].style()))
                    .collect::<Vec<_>>(),
            ));
        }
    }
    fn chart(&mut self, chart: charts::ChartRows) {
        let y = self.lines.len() as u16;
        self.hits.extend(
            chart
                .hits
                .into_iter()
                .map(|(r, a)| (Rect::new(r.x, r.y + y, r.width, r.height), a)),
        );
        self.lines.extend(chart.lines);
    }
}
fn chosen_index(app: &App, n: usize) -> usize {
    match app.stats.hover {
        Some(StatsAction::Inspect(i)) => i,
        _ => app.stats.inspected.unwrap_or(n.saturating_sub(1)),
    }
    .min(n.saturating_sub(1))
}
fn chosen_model(app: &App) -> Option<&str> {
    match &app.stats.hover {
        Some(StatsAction::Model(id)) => Some(id),
        _ => app.stats.model.as_deref(),
    }
}
fn token_details(doc: &mut Doc, t: data::Tokens, app: &App, width: u16) {
    for (label, value) in [
        ("Uncached input", t.input),
        ("Output", t.output),
        ("Cache read", t.cache_read),
        ("Cache write", t.cache_write),
    ] {
        doc.text(
            &format!("{label:<16} {}", data::count(value)),
            app.theme.muted(),
            width,
        );
    }
}
const CALENDAR_COLORS: [(u8, u8, u8); 5] = [
    (18, 18, 18),
    (45, 70, 14),
    (65, 110, 14),
    (95, 150, 10),
    (118, 185, 0),
];
fn calendar_level(sessions: usize) -> usize {
    match sessions {
        0 => 0,
        1 => 1,
        2..=3 => 2,
        4..=7 => 3,
        _ => 4,
    }
}
/// Shared by day tiles and the legend; colored tiles contain only spaces.
fn calendar_style(app: &App, level: usize, hovered: bool, selected: bool) -> Style {
    if !app.theme.color {
        return app.theme.base().patch(app.theme.control(hovered, selected));
    }
    let background = if app.theme.rgb && app.theme.mode == crate::theme::ThemeMode::Dark {
        let percent = match (hovered, selected) {
            (true, true) => 45,
            (false, true) => 35,
            (true, false) => 20,
            _ => 0,
        };
        let (r, g, b) = CALENDAR_COLORS[level];
        let lighten = |v: u8| ((u16::from(v) * (100 - percent) + 255 * percent + 50) / 100) as u8;
        Color::Rgb(lighten(r), lighten(g), lighten(b))
    } else {
        // Portable indexed green ramps keep interaction states distinct after RGB fallback.
        let ramp = match (hovered, selected) {
            (true, true) => [240, 40, 46, 148, 190],
            (false, true) => [238, 34, 40, 112, 148],
            (true, false) => [236, 28, 34, 70, 112],
            _ => [234, 22, 28, 64, 76],
        };
        Color::Indexed(ramp[level])
    };
    app.theme.base().bg(background)
}
fn calendar(doc: &mut Doc, app: &App, width: u16, d: &Dataset, start: i32, end: i32) {
    let first = data::weekday(start);
    let weeks = (first + (end - start + 1) as usize).div_ceil(7);
    let available = usize::from(width.saturating_sub(4)).max(1);
    let step = if weeks * 2 <= available { 2 } else { 1 };
    let capacity = (available / step).max(1);
    let columns = weeks.div_ceil(weeks.div_ceil(capacity)).max(1);
    let mut counts = vec![];
    for day in start..=end {
        let s = d.summary(day, day);
        counts.push((s.sessions, s.tokens.total()));
    }
    doc.line(Line::styled(
        "Activity / sessions per day",
        app.theme.base(),
    ));
    for offset in (0..weeks).step_by(columns.max(1)) {
        let cols = (weeks - offset).min(columns.max(1));
        let lower = (start - first as i32 + offset as i32 * 7).max(start);
        let upper = (start - first as i32 + (offset + cols) as i32 * 7 - 1).min(end);
        doc.line(Line::styled(
            format!("{} - {}", data::date(lower), data::date(upper)),
            app.theme.muted(),
        ));
        let mut months = vec![' '; cols * step];
        let mut previous = None;
        for col in 0..cols {
            let day = (start - first as i32 + (offset + col) as i32 * 7).max(start);
            let month = data::civil(day).1;
            if previous != Some(month) {
                let label = data::month(month);
                if months.len() >= 3 {
                    let x = (col * step).min(months.len() - 3);
                    let next = (col + 1..cols).find(|&j| {
                        data::civil((start - first as i32 + (offset + j) as i32 * 7).max(start)).1
                            != month
                    });
                    if next.is_some_and(|j| (j * step).min(months.len() - 3) < x + 4) {
                        previous = Some(month);
                        continue;
                    }
                    for (i, c) in label.chars().enumerate() {
                        months[x + i] = c;
                    }
                }
                previous = Some(month);
            }
        }
        doc.line(Line::styled(
            format!("    {}", months.iter().collect::<String>()),
            app.theme.muted(),
        ));
        for wd in 0..7 {
            let label = ["Mon", "", "Wed", "", "Fri", "", ""][wd];
            let mut spans = vec![Span::styled(format!("{label:<4}"), app.theme.muted())];
            for col in 0..cols {
                let day = start - first as i32 + ((offset + col) * 7 + wd) as i32;
                if day < start || day > end {
                    spans.push(Span::raw(" ".repeat(step)));
                    continue;
                }
                let index = (day - start) as usize;
                let (n, _) = counts[index];
                let level = calendar_level(n);
                let hovered = matches!(app.stats.hover,Some(StatsAction::Inspect(i)) if i==index);
                let selected = app.stats.inspected == Some(index);
                let content = if app.theme.color {
                    " ".repeat(step)
                } else {
                    format!(
                        "{}{}",
                        [".", "1", "2", "3", "4"][level],
                        " ".repeat(step - 1)
                    )
                };
                spans.push(Span::styled(
                    content,
                    calendar_style(app, level, hovered, selected),
                ));
                doc.hits.push((
                    Rect::new(
                        (4 + col * step) as u16,
                        doc.lines.len() as u16,
                        step as u16,
                        1,
                    ),
                    StatsAction::Inspect(index),
                ));
            }
            doc.line(Line::from(spans));
        }
        doc.gap();
    }
    let mut legend = vec![Span::styled("Less ", app.theme.muted())];
    for level in 0..5 {
        let content = if app.theme.color {
            "  ".to_owned()
        } else {
            format!("{} ", [".", "1", "2", "3", "4"][level])
        };
        legend.push(Span::styled(
            content,
            calendar_style(app, level, false, false),
        ));
    }
    legend.push(Span::styled(" More / sessions", app.theme.muted()));
    doc.line(Line::from(legend));
}
fn overview(app: &App, width: u16, d: &Dataset, start: i32, end: i32, s: &Summary) -> Doc {
    let mut doc = Doc::default();
    let values = [
        ("Total tokens", data::count(s.tokens.total())),
        ("Sessions", s.sessions.to_string()),
        (
            "Active days",
            format!("{} / {}", s.active_days, end - start + 1),
        ),
    ];
    if width >= 72 {
        let col = width as usize / 3;
        doc.line(Line::from(
            values
                .iter()
                .map(|(l, _)| Span::styled(format!("{l:<col$}"), app.theme.muted()))
                .collect::<Vec<_>>(),
        ));
        doc.line(Line::from(
            values
                .iter()
                .map(|(_, v)| Span::styled(format!("{v:<col$}"), app.theme.base()))
                .collect::<Vec<_>>(),
        ));
    } else {
        doc.text(
            &format!(
                "Tokens {} / Sessions {} / Days {}",
                values[0].1, values[1].1, values[2].1
            ),
            app.theme.base(),
            width,
        );
    }
    doc.gap();
    calendar(&mut doc, app, width, d, start, end);
    doc.gap();
    let top = s.models.first().map_or("Unavailable", |m| m.name.as_str());
    let peak = s
        .busiest
        .map(|(d, n)| format!("{} / {n} sessions", data::short_date(d)))
        .unwrap_or_else(|| "Unavailable".into());
    let facts = [
        ("Top model", top.to_owned()),
        ("Busiest day", peak),
        ("Current streak", format!("{} days", s.current_streak)),
        ("Longest streak", format!("{} days", s.longest_streak)),
        ("Longest session", data::duration(s.longest_session)),
    ];
    if width >= 80 {
        for pair in facts.chunks(2) {
            let mut spans = vec![];
            for (l, v) in pair {
                let text = format!("{l}: {v}");
                spans.push(Span::styled(
                    format!("{text:<col$}", col = width as usize / 2),
                    app.theme.muted(),
                ));
            }
            doc.line(Line::from(spans));
        }
    } else {
        for (l, v) in facts {
            doc.text(&format!("{l:<16} {v}"), app.theme.muted(), width);
        }
    }
    if app.stats.details {
        doc.gap();
        doc.line(Line::styled(
            "Token details / total includes cache",
            app.theme.base(),
        ));
        token_details(&mut doc, s.tokens, app, width);
        doc.text(
            &format!("Session time     {}", data::duration(s.seconds)),
            app.theme.muted(),
            width,
        );
    }
    doc
}
fn share(tokens: Option<u64>, total: Option<u64>) -> String {
    match (tokens, total) {
        (Some(_), Some(0)) => "0.0%".into(),
        (Some(t), Some(all)) => format!("{:.1}%", t as f64 * 100.0 / all as f64),
        _ => "Unavailable".into(),
    }
}
fn model_heading(
    doc: &mut Doc,
    app: &App,
    width: u16,
    m: &ModelTotal,
    total: Option<u64>,
    index: usize,
) {
    let focused = chosen_model(app) == Some(m.id.as_str());
    let title = format!(
        "{} {} / {} tokens / {}",
        if app.ascii
            || !app.theme.color
            || !app.theme.rgb
            || app.theme.mode == crate::theme::ThemeMode::Terminal
        {
            (index + 1).to_string()
        } else {
            "▪".into()
        },
        m.name,
        data::count(m.tokens.total()),
        share(m.tokens.total(), total)
    );
    let y = doc.lines.len() as u16;
    doc.text(
        &title,
        charts::ink(app, m.color, chosen_model(app).is_some() && !focused).patch(
            app.theme.control(
                matches!(&app.stats.hover,Some(StatsAction::Model(id)) if id==&m.id),
                app.stats.model.as_deref() == Some(m.id.as_str()),
            ),
        ),
        width,
    );
    doc.hits.push((
        Rect::new(0, y, width, doc.lines.len() as u16 - y),
        StatsAction::Model(m.id.clone()),
    ));
}
fn models(
    app: &App,
    width: u16,
    terminal_height: u16,
    d: &Dataset,
    start: i32,
    end: i32,
    s: &Summary,
) -> Doc {
    let mut doc = Doc::default();
    let models = data::chart_models(s);
    let buckets = d.buckets(start, end, app.stats.range == StatsRange::All, &models);
    let title = format!(
        "{} tokens / {}",
        data::count(s.tokens.total()),
        if app.stats.range == StatsRange::All {
            "weekly totals"
        } else {
            "daily totals"
        }
    );
    doc.text(
        &format!("{title} / {} - {}", data::date(start), data::date(end)),
        app.theme.base(),
        width,
    );
    doc.gap();
    let style = effective_style(app);
    let highlighted = chosen_model(app).and_then(|id| models.iter().position(|m| m.id == id));
    if style == ChartStyle::Trends {
        for (i, m) in models.iter().enumerate() {
            model_heading(&mut doc, app, width, m, s.tokens.total(), i);
            let peak = buckets.iter().filter_map(|b| b.models[i]).max();
            doc.line(Line::styled(
                format!("Own scale / 0 - {} tokens", data::count(peak)),
                app.theme.muted(),
            ));
            let lines = charts::mini(app, width, 3, &buckets, i, m.color);
            let offset = doc.lines.len() as u16;
            let panels = charts::panels(width + 9, buckets.len());
            let mut row = 0;
            for range in panels {
                let len = range.len();
                for j in range.clone() {
                    let x = ((j - range.start) * width as usize / len) as u16;
                    let next = ((j - range.start + 1) * width as usize / len) as u16;
                    doc.hits.push((
                        Rect::new(x, offset + row, next - x, 3),
                        StatsAction::Inspect(j),
                    ));
                }
                row += 3;
            }
            doc.lines.extend(lines);
            doc.line(Line::styled(
                format!("{} - {}", data::date(start), data::date(end)),
                app.theme.muted(),
            ));
            if app.stats.details {
                token_details(&mut doc, m.tokens, app, width);
            }
            doc.gap();
        }
    } else {
        doc.chart(charts::timeline(
            app,
            style,
            width,
            if terminal_height < 32 { 8 } else { 10 },
            &buckets,
            &models,
            highlighted,
        ));
        for (i, m) in models.iter().enumerate() {
            model_heading(&mut doc, app, width, m, s.tokens.total(), i);
            if app.stats.details {
                token_details(&mut doc, m.tokens, app, width);
                doc.gap();
            }
        }
    }

    doc
}
fn document(app: &App, width: u16, height: u16) -> Doc {
    if !app.stats.demo {
        let mut doc = Doc::default();
        doc.line(Line::styled("Statistics unavailable", app.theme.base()));
        doc.gap();
        doc.text("No activity source is connected. Real sessions, tokens and model history are unavailable.",app.theme.muted(),width);
        doc.gap();
        doc.text(
            "Press D or click Sample to explore labeled demo data.",
            app.theme.muted(),
            width,
        );
        return doc;
    }
    let d = data::demo();
    let (start, end) = app.stats.range.bounds(d);
    let s = d.summary(start, end);
    if app.stats.tab == StatsTab::Overview {
        overview(app, width, d, start, end, &s)
    } else {
        models(app, width, height, d, start, end, &s)
    }
}
fn inner(area: Rect) -> Rect {
    Rect::new(
        area.x + 2,
        area.y + 1,
        area.width.saturating_sub(4),
        area.height.saturating_sub(2),
    )
}
fn body(area: Rect, app: &App) -> Rect {
    let content = inner(area);
    let header = 3 + u16::from(app.stats.tab == StatsTab::Models);
    Rect::new(
        content.x,
        content.y + header.min(content.height),
        content.width,
        content.height.saturating_sub(header + 3),
    )
}
pub fn page_rows(app: &App, area: Rect) -> usize {
    body(area, app).height as usize
}
pub fn max_scroll(app: &App, area: Rect, terminal_height: u16) -> usize {
    let b = body(area, app);
    document(app, b.width, terminal_height)
        .lines
        .len()
        .saturating_sub(b.height as usize)
}
fn control(
    frame: &mut Frame,
    r: Rect,
    label: &str,
    selected: bool,
    app: &App,
    map: &mut UiMap,
    a: StatsAction,
) {
    ui::render_control(
        frame,
        r,
        label,
        if selected {
            app.theme.selected()
        } else {
            app.theme.muted()
        },
        app,
        selected,
    );
    map.add_target(r, HitAction::Stats(a));
}
pub fn render(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    let content = inner(area);
    let b = body(area, app);
    if content.width == 0 || content.height == 0 {
        return;
    }
    let t = app.theme;
    let rule = if app.ascii { "-" } else { "─" }.repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(rule).style(t.rule()),
        Rect::new(area.x, area.y, area.width, 1),
    );
    frame.render_widget(
        Paragraph::new("Statistics").style(t.base()),
        Rect::new(content.x, content.y, 10, 1),
    );
    let badge = if app.stats.demo {
        "DEMO DATA"
    } else {
        "Unavailable"
    };
    let bw = badge.width() as u16;
    frame.render_widget(
        Paragraph::new(badge).style(if app.stats.demo {
            t.warning()
        } else {
            t.muted()
        }),
        Rect::new(content.right() - bw, content.y, bw, 1),
    );
    let mut x = content.x;
    for (tab, label) in [
        (StatsTab::Overview, "Overview"),
        (StatsTab::Models, "Models"),
    ] {
        let w = label.width() as u16 + 4;
        control(
            frame,
            Rect::new(x, content.y + 1, w, 1),
            label,
            app.stats.tab == tab,
            app,
            map,
            StatsAction::Tab(tab),
        );
        x += w + 1;
    }
    let label = if app.stats.details { "Less" } else { "Details" };
    let w = label.len() as u16 + 4;
    control(
        frame,
        Rect::new(content.right() - w, content.y + 1, w, 1),
        label,
        app.stats.details,
        app,
        map,
        StatsAction::Details,
    );
    let mut x = content.x;
    for range in StatsRange::ALL {
        let label = range.label();
        let w = label.width() as u16 + 4;
        control(
            frame,
            Rect::new(x, content.y + 2, w, 1),
            label,
            app.stats.range == range,
            app,
            map,
            StatsAction::Range(range),
        );
        x += w + 1;
    }
    if app.stats.tab == StatsTab::Models {
        let mut x = content.x;
        for style in ChartStyle::ALL {
            let label = style.label();
            let w = label.len() as u16 + 4;
            control(
                frame,
                Rect::new(x, content.y + 3, w, 1),
                label,
                effective_style(app) == style,
                app,
                map,
                StatsAction::Style(style),
            );
            x += w + 1;
        }
        if app.stats.preview.is_some() && x + 8 < content.right() {
            frame.render_widget(
                Paragraph::new("Preview").style(t.muted()),
                Rect::new(x, content.y + 3, 7, 1),
            );
        }
    }
    let doc = document(app, b.width, frame.area().height);
    let offset = app
        .scroll
        .min(doc.lines.len().saturating_sub(b.height as usize));
    frame.render_widget(
        Paragraph::new(doc.lines)
            .scroll((offset.min(u16::MAX as usize) as u16, 0))
            .style(t.base()),
        b,
    );
    for (r, a) in doc.hits {
        let start = (r.y as usize).max(offset);
        let end = (r.bottom() as usize).min(offset + b.height as usize);
        if start < end {
            if app.stats.tab == StatsTab::Models
                && let StatsAction::Inspect(index) = a
            {
                let hovered = matches!(app.stats.hover,Some(StatsAction::Inspect(i)) if i==index);
                let selected = app.stats.inspected == Some(index);
                if hovered || selected {
                    for y in b.y + (start - offset) as u16..b.y + (end - offset) as u16 {
                        for x in b.x + r.x..b.x + r.right() {
                            frame.buffer_mut()[(x, y)]
                                .set_style(app.theme.control(hovered, selected));
                        }
                    }
                }
            }
            map.add_target(
                Rect::new(
                    b.x + r.x,
                    b.y + (start - offset) as u16,
                    r.width,
                    (end - start) as u16,
                ),
                HitAction::Stats(a),
            );
        }
    }
    render_inspector(
        frame,
        Rect::new(
            content.x,
            content.bottom().saturating_sub(3),
            content.width,
            2,
        ),
        app,
    );
    let y = content.bottom() - 1;
    let hint = if content.width >= 96 {
        "Tab views / R dates / C chart / Ctrl+O details / [ ] inspect / Esc/Enter close"
    } else if content.width >= 72 {
        "Tab / R dates / C chart / PgUp/Dn / Esc/Enter close"
    } else {
        "Tab / R dates / PgDn / Esc/Enter close"
    };
    frame.render_widget(
        Paragraph::new(ui::clip(hint, content.width.saturating_sub(10) as usize)).style(t.muted()),
        Rect::new(content.x, y, content.width.saturating_sub(10), 1),
    );
    control(
        frame,
        Rect::new(content.right() - 10, y, 10, 1),
        "Sample",
        app.stats.demo,
        app,
        map,
        StatsAction::Demo,
    );
}

fn render_inspector(frame: &mut Frame, r: Rect, app: &App) {
    if !app.stats.demo || r.height < 2 {
        return;
    }
    let d = data::demo();
    let (start, end) = app.stats.range.bounds(d);
    let t = app.theme;
    let (first, second) = if app.stats.tab == StatsTab::Overview {
        let day = start + chosen_index(app, (end - start + 1) as usize) as i32;
        let s = d.summary(day, day);
        (
            format!("{} / {} sessions", data::date(day), s.sessions),
            format!(
                "{} tokens / [ ] inspect day",
                s.tokens
                    .total()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "Unavailable".into())
            ),
        )
    } else {
        let summary = d.summary(start, end);
        let models = data::chart_models(&summary);
        let buckets = d.buckets(start, end, app.stats.range == StatsRange::All, &models);
        let Some(bucket) = buckets.get(chosen_index(app, buckets.len())) else {
            return;
        };
        let total = bucket
            .models
            .iter()
            .try_fold(0u64, |a, b| a.checked_add((*b)?));
        let first = format!(
            "{} - {} / {} tokens",
            data::short_date(bucket.start),
            data::short_date(bucket.end),
            data::count(total)
        );
        let model = chosen_model(app)
            .and_then(|id| models.iter().position(|m| m.id == id))
            .unwrap_or(0);
        let second = models
            .get(model)
            .map(|m| {
                let value = bucket.models[model]
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "Unavailable".into());
                let tail = format!(" / {value} tokens");
                format!(
                    "{}{}",
                    ui::clip(
                        &m.name,
                        r.width.saturating_sub(tail.width() as u16) as usize
                    ),
                    tail
                )
            })
            .unwrap_or_else(|| "No model activity".into());
        (first, second)
    };
    frame.render_widget(
        Paragraph::new(ui::clip(&first, r.width as usize)).style(t.muted()),
        Rect::new(r.x, r.y, r.width, 1),
    );
    frame.render_widget(
        Paragraph::new(ui::clip(&second, r.width as usize)).style(t.base()),
        Rect::new(r.x, r.y + 1, r.width, 1),
    );
}

#[cfg(test)]
mod calendar_fill_tests {
    use super::*;
    #[test]
    fn every_threshold_and_interaction_state_has_the_expected_fill() {
        let a = App::default();
        for (count, level) in [
            (0, 0),
            (1, 1),
            (2, 2),
            (3, 2),
            (4, 3),
            (7, 3),
            (8, 4),
            (100, 4),
        ] {
            assert_eq!(calendar_level(count), level);
            let (r, g, b) = CALENDAR_COLORS[level];
            assert_eq!(
                calendar_style(&a, level, false, false).bg,
                Some(Color::Rgb(r, g, b))
            );
        }
        assert_eq!(
            calendar_style(&a, 3, true, false).bg,
            Some(Color::Rgb(127, 171, 59))
        );
        assert_eq!(
            calendar_style(&a, 3, false, true).bg,
            Some(Color::Rgb(151, 187, 96))
        );
        assert_eq!(
            calendar_style(&a, 3, true, true).bg,
            Some(Color::Rgb(167, 197, 120))
        );
    }
    #[test]
    fn custom_week_covers_all_levels_and_keeps_padding_black() {
        let a = App::default();
        let start = data::civil_day(2024, 2, 28);
        let counts = [0, 1, 2, 4, 8, 3, 7];
        let mut sessions = vec![];
        for (day, &count) in counts.iter().enumerate() {
            for _ in 0..count {
                sessions.push(data::Session {
                    id: sessions.len() as u64,
                    day: start + day as i32,
                    seconds: Some(0),
                    usage: vec![],
                });
            }
        }
        let dataset = Dataset {
            start,
            end: start + 6,
            models: vec![],
            sessions,
        };
        let mut doc = Doc::default();
        calendar(&mut doc, &a, 80, &dataset, start, start + 6);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 80, doc.lines.len() as u16));
        Paragraph::new(doc.lines)
            .style(a.theme.base())
            .render(buffer.area, &mut buffer);
        assert_eq!(doc.hits.len(), 7);
        for (r, action) in &doc.hits {
            let StatsAction::Inspect(index) = action else {
                panic!()
            };
            let (r0, g, b) = CALENDAR_COLORS[calendar_level(counts[*index])];
            assert_eq!(r.width, 2);
            for x in r.x..r.right() {
                assert_eq!(buffer[(x, r.y)].symbol(), " ");
                assert_eq!(buffer[(x, r.y)].bg, Color::Rgb(r0, g, b));
            }
        }
        let first = doc.hits[0].0;
        for y in first.y - data::weekday(start) as u16..first.y {
            assert_eq!(buffer[(first.x, y)].bg, Color::Rgb(0, 0, 0));
        }
        let last = doc.hits.last().unwrap().0;
        for y in last.y + 1..first.y - data::weekday(start) as u16 + 7 {
            assert_eq!(buffer[(last.x, y)].bg, Color::Rgb(0, 0, 0));
        }
    }
}
