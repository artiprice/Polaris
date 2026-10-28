//! Responsive settings form. Persistence belongs to preferences, not the Usage screen.
use crate::{
    app::{App, Screen},
    interaction::{HitAction, UiMap},
    preferences::{ColorPolicy, PreferenceField, SaveStatus},
    theme::ThemeMode,
    ui,
    usage::{self, Gauge, UsageBarStyle},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    widgets::{Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsCategory {
    Appearance,
    Usage,
    Statistics,
    Interaction,
    Accessibility,
    Providers,
}
impl SettingsCategory {
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn from_index(i: usize) -> Self {
        [
            Self::Appearance,
            Self::Usage,
            Self::Statistics,
            Self::Interaction,
            Self::Accessibility,
            Self::Providers,
        ][i.min(5)]
    }
}
pub const CATEGORIES: [&str; 6] = [
    "Appearance",
    "Usage",
    "Statistics",
    "Interaction",
    "Accessibility",
    "Providers",
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    Category(usize),
    Select(usize),
    Choose { row: usize, value: usize },
    Apply,
}
#[derive(Clone, Debug)]
pub struct SettingsViewState {
    pub categories_focused: bool,
    pub reveal_selection: bool,
    pub page_rows: usize,
    pub highlighted_value: Option<usize>,
}
impl Default for SettingsViewState {
    fn default() -> Self {
        Self {
            categories_focused: true,
            reveal_selection: true,
            page_rows: 8,
            highlighted_value: None,
        }
    }
}
pub fn sync_effective(app: &mut App) {
    app.theme.mode = app.preferences.values.theme;
    app.theme.color = app
        .preferences
        .values
        .color
        .enabled(app.preferences.auto_color);
    app.ascii = app.preferences.values.ascii;
    app.reduce_motion = app.preferences.values.reduced_motion;
}
pub fn row_count(category: usize) -> usize {
    match SettingsCategory::from_index(category) {
        SettingsCategory::Appearance | SettingsCategory::Accessibility => 2,
        SettingsCategory::Usage | SettingsCategory::Statistics => 3,
        _ => 1,
    }
}
fn choices(app: &App, row: usize) -> (Vec<&'static str>, usize, Option<PreferenceField>) {
    let p = &app.preferences.values;
    match (SettingsCategory::from_index(app.tab), row) {
        (SettingsCategory::Appearance, 0) => (
            vec!["Dark", "Terminal"],
            usize::from(p.theme == ThemeMode::Terminal),
            Some(PreferenceField::Theme),
        ),
        (SettingsCategory::Appearance, _) => (
            vec!["Unicode", "ASCII"],
            usize::from(p.ascii),
            Some(PreferenceField::Ascii),
        ),
        (SettingsCategory::Usage, _) => (vec!["Use style"], 0, Some(PreferenceField::UsageBar)),
        (SettingsCategory::Statistics, _) => {
            (vec!["Use style"], 0, Some(PreferenceField::StatsChart))
        }
        (SettingsCategory::Interaction, _) => (vec!["Auto", "Plan"], app.mode.min(1), None),
        (SettingsCategory::Accessibility, 0) => (
            vec!["Auto", "Always", "Never"],
            match p.color {
                ColorPolicy::Auto => 0,
                ColorPolicy::Always => 1,
                ColorPolicy::Never => 2,
            },
            Some(PreferenceField::Color),
        ),
        (SettingsCategory::Accessibility, _) => (
            vec!["Normal", "Reduced"],
            usize::from(p.reduced_motion),
            Some(PreferenceField::Motion),
        ),
        _ => (vec!["Manage providers"], 0, None),
    }
}
const STYLES: [UsageBarStyle; 3] = [
    UsageBarStyle::ContinuousStrip,
    UsageBarStyle::FineTicks,
    UsageBarStyle::SegmentedRail,
];
fn candidate(app: &App) -> usize {
    let (options, applied, _) = choices(app, app.selected);
    app.settings
        .highlighted_value
        .unwrap_or(applied)
        .min(options.len() - 1)
}
fn pending(app: &App) -> bool {
    !app.settings.categories_focused
        && if app.tab == SettingsCategory::Statistics.index() {
            crate::stats::ChartStyle::ALL[app.selected] != app.preferences.values.stats_chart_style
        } else if app.tab == 1 {
            STYLES[app.selected] != app.preferences.values.usage_bar_style
        } else {
            candidate(app) != choices(app, app.selected).1
        }
}
fn categories(app: &mut App) {
    app.settings.categories_focused = true;
    app.settings.highlighted_value = None;
    app.settings.reveal_selection = true;
}
fn enter_controls(app: &mut App) {
    let row = if app.tab == SettingsCategory::Statistics.index() {
        app.preferences.values.stats_chart_style.index()
    } else if app.tab == 1 {
        STYLES
            .iter()
            .position(|s| *s == app.preferences.values.usage_bar_style)
            .unwrap_or(0)
    } else {
        app.selected
    };
    activate(app, SettingsAction::Select(row));
}
pub fn activate(app: &mut App, action: SettingsAction) {
    match action {
        SettingsAction::Category(i) => {
            app.tab = i.min(CATEGORIES.len() - 1);
            app.selected = 0;
            app.scroll = 0;
            categories(app);
        }
        SettingsAction::Select(row) => {
            app.selected = row.min(row_count(app.tab) - 1);
            app.settings.categories_focused = false;
            app.settings.highlighted_value = Some(choices(app, app.selected).1);
            app.settings.reveal_selection = true;
        }
        SettingsAction::Choose { row, value } => {
            activate(app, SettingsAction::Select(row));
            app.settings.highlighted_value =
                Some(value.min(choices(app, app.selected).0.len() - 1));
        }
        SettingsAction::Apply => {
            if app.settings.categories_focused {
                enter_controls(app);
                return;
            }
            let row = app.selected;
            let (_, _, field) = choices(app, row);
            let value = candidate(app);
            let changed = pending(app);
            let retry = matches!(app.preferences.status, SaveStatus::Error(_));
            let overridden = field.is_some_and(|f| app.preferences.overrides.contains(&f));
            if app.tab != SettingsCategory::Providers.index() && !changed && !retry && !overridden {
                return;
            }
            match (SettingsCategory::from_index(app.tab), row) {
                (SettingsCategory::Appearance, 0) => {
                    app.preferences.values.theme = if value == 0 {
                        ThemeMode::Dark
                    } else {
                        ThemeMode::Terminal
                    }
                }
                (SettingsCategory::Appearance, _) => app.preferences.values.ascii = value != 0,
                (SettingsCategory::Usage, _) => {
                    app.preferences.values.usage_bar_style = STYLES[row]
                }
                (SettingsCategory::Statistics, _) => {
                    app.preferences.values.stats_chart_style = crate::stats::ChartStyle::ALL[row]
                }
                (SettingsCategory::Interaction, _) => app.mode = value,
                (SettingsCategory::Accessibility, 0) => {
                    app.preferences.values.color =
                        [ColorPolicy::Auto, ColorPolicy::Always, ColorPolicy::Never][value]
                }
                (SettingsCategory::Accessibility, _) => {
                    app.preferences.values.reduced_motion = value != 0
                }
                _ => {
                    app.open(Screen::Providers);
                    return;
                }
            }
            if let Some(field) = field {
                app.preferences.commit(field);
                sync_effective(app);
            }
        }
    }
}
pub fn handle_key(app: &mut App, key: KeyEvent) {
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return;
    }
    match key.code {
        KeyCode::Tab | KeyCode::BackTab => {
            if app.settings.categories_focused {
                enter_controls(app);
            } else {
                categories(app);
            }
        }
        KeyCode::Up | KeyCode::Down if app.settings.categories_focused => {
            let n = CATEGORIES.len();
            let next = if key.code == KeyCode::Up {
                (app.tab + n - 1) % n
            } else {
                (app.tab + 1) % n
            };
            activate(app, SettingsAction::Category(next));
        }
        KeyCode::Up | KeyCode::Down => {
            let n = row_count(app.tab);
            let next = if key.code == KeyCode::Up {
                (app.selected + n - 1) % n
            } else {
                (app.selected + 1) % n
            };
            activate(app, SettingsAction::Select(next));
        }
        KeyCode::Right | KeyCode::Enter if app.settings.categories_focused => {
            enter_controls(app);
        }
        KeyCode::Left if app.settings.categories_focused => {}
        KeyCode::Enter => activate(app, SettingsAction::Apply),
        KeyCode::Left | KeyCode::Right => {
            let current = candidate(app);
            if key.code == KeyCode::Left && current == 0 {
                categories(app);
                return;
            }
            let (values, _, _) = choices(app, app.selected);
            let next = if key.code == KeyCode::Left {
                current - 1
            } else {
                (current + 1).min(values.len() - 1)
            };
            activate(
                app,
                SettingsAction::Choose {
                    row: app.selected,
                    value: next,
                },
            );
        }
        KeyCode::PageUp | KeyCode::PageDown => {
            app.settings.reveal_selection = false;
            app.scroll = if key.code == KeyCode::PageUp {
                app.scroll.saturating_sub(app.settings.page_rows.max(1))
            } else {
                app.scroll.saturating_add(app.settings.page_rows.max(1))
            };
        }
        _ => {}
    }
}
struct Regions {
    categories: Vec<Rect>,
    detail: Rect,
    divider: Option<Rect>,
}
fn regions(area: Rect, wide: bool) -> Regions {
    if wide {
        let categories = (0..CATEGORIES.len())
            .map(|i| Rect::new(area.x, area.y + i as u16 * 2, 19, 1))
            .collect();
        Regions {
            categories,
            detail: Rect::new(
                area.x + 23,
                area.y,
                area.width.saturating_sub(23),
                area.height,
            ),
            divider: Some(Rect::new(area.x + 20, area.y, 1, area.height)),
        }
    } else {
        let (mut x, mut y) = (area.x, area.y);
        let mut categories = vec![];
        for label in CATEGORIES {
            let w = label.width() as u16 + 4;
            if x + w > area.right() {
                x = area.x;
                y += 1;
            }
            categories.push(Rect::new(x, y, w.min(area.width), 1));
            x += w + 1;
        }
        let start = (y + 2).min(area.bottom());
        Regions {
            categories,
            detail: Rect::new(
                area.x,
                start,
                area.width,
                area.bottom().saturating_sub(start),
            ),
            divider: None,
        }
    }
}
fn description(app: &App, row: usize) -> (&'static str, &'static str) {
    match (SettingsCategory::from_index(app.tab), row) {
        (SettingsCategory::Appearance, 0) => {
            ("Theme", "Black Polaris surface or your terminal palette.")
        }
        (SettingsCategory::Appearance, _) => ("Glyphs", "Use ASCII when Unicode renders unevenly."),
        (SettingsCategory::Usage, 0) => ("Strip / default", ""),
        (SettingsCategory::Usage, 1) => ("Ticks", ""),
        (SettingsCategory::Usage, _) => ("Segments", ""),
        (SettingsCategory::Statistics, 0) => ("Stacked / default", ""),
        (SettingsCategory::Statistics, 1) => ("Lines", ""),
        (SettingsCategory::Statistics, _) => ("Trends", ""),
        (SettingsCategory::Interaction, _) => (
            "Interaction mode",
            "Session only / does not change saved preferences.",
        ),
        (SettingsCategory::Accessibility, 0) => {
            ("Color policy", "Auto respects NO_COLOR and TERM=dumb.")
        }
        (SettingsCategory::Accessibility, _) => {
            ("Motion", "Reduced uses static activity indicators.")
        }
        _ => (
            "Providers",
            "Session only / credentials and connections stay local.",
        ),
    }
}
fn diagnostic(app: &App) -> Option<&str> {
    match &app.preferences.status {
        SaveStatus::Warning(s) | SaveStatus::Error(s) => Some(s),
        _ => None,
    }
}
fn detail_parts(app: &App, area: Rect) -> (Rect, Rect, usize) {
    let header = usize::from(area.height > 4);
    let message_rows = diagnostic(app).map_or(0, |s| {
        Paragraph::new(s)
            .wrap(Wrap { trim: false })
            .line_count(area.width)
            .min(3)
    });
    let footer_height = (1 + message_rows as u16).min(area.height.saturating_sub(2));
    let content = Rect::new(
        area.x,
        area.y + header as u16,
        area.width,
        area.height.saturating_sub(footer_height + header as u16),
    );
    let footer = Rect::new(
        area.x,
        area.bottom().saturating_sub(footer_height),
        area.width,
        footer_height,
    );
    (content, footer, header)
}
pub fn page_rows(app: &App, area: Rect, wide: bool) -> usize {
    detail_parts(app, regions(area, wide).detail).0.height as usize
}
pub fn max_scroll(app: &App, area: Rect, wide: bool) -> usize {
    (row_count(app.tab) * 4).saturating_sub(page_rows(app, area, wide))
}
pub fn scroll_offset(app: &App, area: Rect, wide: bool) -> usize {
    let height = page_rows(app, area, wide);
    let max = max_scroll(app, area, wide);
    let mut offset = app.scroll.min(max);
    if app.settings.reveal_selection && !app.settings.categories_focused {
        let start = app.selected * 4;
        let end = start + 2;
        if start < offset {
            offset = start;
        } else if end >= offset + height {
            offset = (end + 1).saturating_sub(height).min(max);
        }
    }
    offset
}
pub fn render(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    let t = app.theme;
    let r = regions(area, frame.area().width >= 100);
    if let Some(divider) = r.divider {
        for y in divider.y..divider.bottom() {
            frame.render_widget(
                Paragraph::new(if app.ascii { "|" } else { "│" }).style(t.rule()),
                Rect::new(divider.x, y, 1, 1),
            );
        }
    }
    for (i, rect) in r.categories.into_iter().enumerate() {
        if rect.bottom() > area.bottom() {
            continue;
        }
        let style = if i == app.tab {
            t.selected()
        } else {
            t.muted()
        };
        ui::render_control(
            frame,
            rect,
            CATEGORIES[i],
            style,
            app,
            app.settings.categories_focused && i == app.tab,
        );
        map.add_target(rect, HitAction::Settings(SettingsAction::Category(i)));
    }
    let (content, footer, header) = detail_parts(app, r.detail);
    if header > 0 {
        let label = if app.tab == SettingsCategory::Statistics.index() {
            "Chart style / labeled demo preview"
        } else if app.tab == 1 {
            "Bar style / preview only / 42% sample"
        } else {
            CATEGORIES[app.tab.min(CATEGORIES.len() - 1)]
        };
        frame.render_widget(
            Paragraph::new(ui::clip(label, r.detail.width as usize)).style(t.muted()),
            Rect::new(r.detail.x, r.detail.y, r.detail.width, 1),
        );
    }
    let total = row_count(app.tab) * 4;
    let max = total.saturating_sub(content.height as usize);
    let offset = scroll_offset(app, area, frame.area().width >= 100);
    let gauge = Gauge {
        label: "Preview".into(),
        used: Some(42),
        capacity: Some(100),
        unit: "sample",
        reset: None,
    };
    for (row, bar_style) in STYLES.into_iter().enumerate().take(row_count(app.tab)) {
        let (title, desc) = description(app, row);
        let (options, current, field) = choices(app, row);
        for line in 0..3usize {
            let logical = row * 4 + line;
            if logical < offset || logical >= offset + content.height as usize {
                continue;
            }
            let rect = Rect::new(
                content.x,
                content.y + (logical - offset) as u16,
                content.width,
                1,
            );
            let hovered = app.mouse_position.is_some_and(|(x, y)| {
                x >= rect.x
                    && x < rect.right()
                    && y >= content.y + (row * 4).saturating_sub(offset) as u16
                    && y < content.y + (row * 4 + 3).saturating_sub(offset) as u16
            });
            let focused = !app.settings.categories_focused && app.selected == row;
            let bg = t.control(hovered, focused).remove_modifier(Modifier::BOLD);
            frame.render_widget(
                Paragraph::new(" ".repeat(rect.width as usize)).style(t.base().patch(bg)),
                rect,
            );
            if app.tab == 1 || app.tab == SettingsCategory::Statistics.index() {
                let is_stats = app.tab == SettingsCategory::Statistics.index();
                let active = if is_stats {
                    app.preferences.values.stats_chart_style == crate::stats::ChartStyle::ALL[row]
                } else {
                    app.preferences.values.usage_bar_style == bar_style
                };
                let style = if active { t.selected() } else { t.muted() };
                if line == 0 {
                    frame.render_widget(
                        Paragraph::new(ui::clip(
                            &format!(
                                "{} {} {title}",
                                if focused { ">" } else { " " },
                                if active { "[x]" } else { "[ ]" }
                            ),
                            rect.width as usize,
                        ))
                        .style(if focused { t.base() } else { style }.patch(bg)),
                        rect,
                    );
                } else if is_stats {
                    let preview = crate::stats_chart::preview(
                        app,
                        crate::stats::ChartStyle::ALL[row],
                        rect.width.saturating_sub(4),
                    );
                    if let Some(line) = preview.get(line - 1) {
                        frame.render_widget(
                            Paragraph::new(line.clone()).style(bg),
                            Rect::new(rect.x + 4, rect.y, rect.width.saturating_sub(4), 1),
                        );
                    }
                } else if line == 1 {
                    frame.render_widget(
                        Paragraph::new(usage::bar(
                            &gauge,
                            bar_style,
                            rect.width.saturating_sub(4),
                            t,
                            app.ascii,
                        ))
                        .style(bg),
                        Rect::new(rect.x + 4, rect.y, rect.width.saturating_sub(4), 1),
                    );
                }
                map.add_target(
                    rect,
                    HitAction::Settings(SettingsAction::Choose { row, value: 0 }),
                );
            } else if line == 0 {
                let overridden = field.is_some_and(|f| app.preferences.overrides.contains(&f));
                let title = format!(
                    "{title}{}",
                    if overridden { " / launch override" } else { "" }
                );
                frame.render_widget(
                    Paragraph::new(ui::clip(&title, rect.width as usize)).style(t.base().patch(bg)),
                    rect,
                );
                map.add_target(rect, HitAction::Settings(SettingsAction::Select(row)));
            } else if line == 1 {
                frame.render_widget(
                    Paragraph::new(ui::clip(desc, rect.width as usize)).style(t.muted().patch(bg)),
                    rect,
                );
                map.add_target(rect, HitAction::Settings(SettingsAction::Select(row)));
            } else {
                map.add_target(rect, HitAction::Settings(SettingsAction::Select(row)));
                let mut x = rect.x;
                for (value, label) in options.iter().enumerate() {
                    let candidate_focused = focused && value == candidate(app);
                    let label = if app.tab == SettingsCategory::Providers.index() {
                        format!("{} {label}", if candidate_focused { ">" } else { " " })
                    } else {
                        format!(
                            "{}{} {label}",
                            if candidate_focused { ">" } else { " " },
                            if value == current { "[x]" } else { "[ ]" }
                        )
                    };
                    let width = (label.width() as u16 + 4).min(rect.right().saturating_sub(x));
                    if width == 0 {
                        break;
                    }
                    let option = Rect::new(x, rect.y, width, 1);
                    let ink = if app.tab == SettingsCategory::Providers.index() {
                        t.muted()
                    } else if value == current {
                        t.selected()
                    } else {
                        t.muted()
                    };
                    ui::render_control(
                        frame,
                        option,
                        &label,
                        if candidate_focused { t.base() } else { ink },
                        app,
                        candidate_focused,
                    );
                    map.add_target(
                        option,
                        HitAction::Settings(SettingsAction::Choose { row, value }),
                    );
                    x += width + 1;
                }
            }
        }
    }
    if footer.height > 0 {
        if let Some(message) = diagnostic(app) {
            frame.render_widget(
                Paragraph::new(message).wrap(Wrap { trim: false }).style(
                    if matches!(app.preferences.status, SaveStatus::Error(_)) {
                        t.error()
                    } else {
                        t.warning()
                    },
                ),
                Rect::new(
                    footer.x,
                    footer.y,
                    footer.width,
                    footer.height.saturating_sub(1),
                ),
            );
        }
        let path = app
            .preferences
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "In-memory preview / saving disabled".into());
        let path = if pending(app) {
            format!("Enter to apply / {path}")
        } else if max > 0 {
            format!(
                "{}-{} / {} rows / {path}",
                offset + 1,
                (offset + content.height as usize).min(total),
                total
            )
        } else {
            path
        };
        frame.render_widget(
            Paragraph::new(ui::clip(&path, footer.width as usize)).style(t.muted()),
            Rect::new(footer.x, footer.bottom() - 1, footer.width, 1),
        );
    }
}
