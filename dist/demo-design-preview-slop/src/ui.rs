mod cache;
use crate::{
    app::{App, COMMANDS, EFFORTS, Entry, EntryKind, SCENARIOS, SESSIONS, Screen, SuggestionKind},
    code_view::CodeBlock,
    completion::active_token,
    composer::{AttachmentPayload, display_width},
    interaction::{HitAction, TextRegion, TextRegionId, UiMap},
    logo,
    models::{model_description, model_label},
    theme::ThemeMode,
};
use ratatui::{
    Frame, Terminal,
    backend::TestBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Clear, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 20;

pub fn clip(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.into();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > width.saturating_sub(1) {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('~');
    out
}

// Explicit display-cell wrapping also supplies the editor cursor location.
pub fn wrap_editor(text: &str, cursor: usize, width: u16) -> (Vec<String>, usize, usize) {
    let width = usize::from(width.max(1));
    let mut lines = vec![String::new()];
    let mut col = 0;
    let mut pos = (0, 0);
    for (i, ch) in text.char_indices() {
        if i == cursor {
            pos = (lines.len() - 1, col);
        }
        if ch == '\n' {
            lines.push(String::new());
            col = 0;
            continue;
        }
        let w = ch.width().unwrap_or(0);
        if col + w > width {
            lines.push(String::new());
            col = 0;
            if i == cursor {
                pos = (lines.len() - 1, 0);
            }
        }
        lines.last_mut().unwrap().push(ch);
        col += w;
    }
    if cursor == text.len() {
        if col >= width {
            lines.push(String::new());
            col = 0;
        }
        pos = (lines.len() - 1, col);
    }
    (lines, pos.0, pos.1)
}

fn line(text: impl Into<String>, style: Style) -> Line<'static> {
    Line::styled(text.into(), style)
}
fn put(frame: &mut Frame, area: Rect, text: impl Into<Text<'static>>, style: Style) {
    frame.render_widget(Paragraph::new(text).style(style), area);
}
fn render_outline(frame: &mut Frame, area: Rect, app: &App) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    if app.ascii {
        let rule = format!("+{}+", "-".repeat(area.width.saturating_sub(2) as usize));
        put(frame, row(area, 0), rule.clone(), app.theme.rule());
        put(frame, row(area, area.height - 1), rule, app.theme.rule());
        for y in area.y + 1..area.bottom() - 1 {
            put(frame, Rect::new(area.x, y, 1, 1), "|", app.theme.rule());
            put(
                frame,
                Rect::new(area.right() - 1, y, 1, 1),
                "|",
                app.theme.rule(),
            );
        }
    } else {
        frame.render_widget(
            Block::bordered()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(app.theme.rule())
                .style(app.theme.base()),
            area,
        );
    }
}

fn inset(area: Rect, x: u16, y: u16) -> Rect {
    Rect::new(
        area.x + x.min(area.width),
        area.y + y.min(area.height),
        area.width.saturating_sub(x * 2),
        area.height.saturating_sub(y * 2),
    )
}
fn row(area: Rect, y: u16) -> Rect {
    Rect::new(area.x, area.y + y, area.width, 1)
}

pub fn welcome_lines(app: &App) -> Vec<Line<'static>> {
    let t = app.theme;
    let mut lines = vec![Line::from(vec![
        Span::styled("Polaris", t.base()),
        Span::styled(
            format!(" v{}", env!("CARGO_PKG_VERSION").replace("-alpha", "alpha")),
            t.muted(),
        ),
    ])];
    if let Some(account) = &app.account_email {
        lines.push(line(account.clone(), t.muted()));
    }
    lines.push(line(app.workspace.clone(), t.muted()));
    lines
}

/// Clip complete graphemes while retaining the title/version's distinct styles.
fn header_line(source: Line<'static>, width: usize, app: &App) -> Line<'static> {
    if source.width() <= width {
        return source;
    }
    if width == 0 {
        return Line::default();
    }
    let suffix = if app.ascii {
        &"..."[..width.min(3)]
    } else {
        "…"
    };
    let limit = width.saturating_sub(UnicodeWidthStr::width(suffix));
    let mut spans = Vec::new();
    let mut used = 0;
    'spans: for span in source.spans {
        let mut text = String::new();
        for g in span.content.graphemes(true) {
            let cells = UnicodeWidthStr::width(g);
            if used + cells > limit {
                spans.push(Span::styled(text, span.style));
                break 'spans;
            }
            text.push_str(g);
            used += cells;
        }
        spans.push(Span::styled(text, span.style));
    }
    if used < limit {
        spans.push(Span::styled(" ".repeat(limit - used), app.theme.muted()));
    }
    spans.push(Span::styled(suffix.to_owned(), app.theme.muted()));
    Line::from(spans)
}

fn render_header(frame: &mut Frame, header: logo::HeaderLayout, app: &App) {
    let t = app.theme;
    logo::render(
        frame,
        Rect::new(header.bar.x, header.bar.y, 1, 1),
        logo::Variant::Emblem,
        t,
        app.ascii,
    );
    let brand = welcome_lines(app).remove(0);
    let brand_width = brand.width() as u16;
    put(
        frame,
        Rect::new(
            header.bar.x + 2,
            header.bar.y,
            header.bar.width.saturating_sub(2),
            1,
        ),
        header_line(brand, header.bar.width.saturating_sub(2) as usize, app),
        t.base(),
    );
    let path_width = header.bar.width.saturating_sub(brand_width + 6);
    if path_width >= 12 {
        let rr = Rect::new(header.bar.right() - path_width, header.bar.y, path_width, 1);
        put(
            frame,
            rr,
            header_line(
                line(app.workspace.clone(), t.muted()),
                path_width as usize,
                app,
            )
            .right_aligned(),
            t.muted(),
        );
    }
    if let (Some(rect), Some(account)) = (header.account, &app.account_email) {
        put(
            frame,
            rect,
            header_line(line(account.clone(), t.muted()), rect.width as usize, app),
            t.muted(),
        );
    }
    if let Some(welcome) = header.welcome {
        logo::render(frame, welcome.art, welcome.variant, t, app.ascii);
        put(
            frame,
            welcome.heading,
            Line::from("What's on your mind for Polaris?").centered(),
            t.base(),
        );
    }
}

pub fn entry_lines(entry: &Entry, app: &App, expanded: bool) -> Vec<Line<'static>> {
    let t = app.theme;
    let marker = match entry.kind {
        EntryKind::User => ">",
        EntryKind::Assistant => {
            if app.ascii {
                "*"
            } else {
                "◆"
            }
        }
        EntryKind::Tool => {
            if app.ascii {
                "+"
            } else {
                "▪"
            }
        }
        EntryKind::Error => "!",
        EntryKind::Warning => "!",
        EntryKind::Completion => "",
        EntryKind::Note => "-",
        EntryKind::AgentLaunch => {
            if app.ascii {
                "+"
            } else {
                "●"
            }
        }
    };
    let title_style = match entry.kind {
        EntryKind::Error => t.error(),
        EntryKind::Warning => t.warning(),
        EntryKind::Tool => t.tool_title(false),
        EntryKind::AgentLaunch => t.selected(),
        EntryKind::Assistant => t.bold(),
        EntryKind::User => t.bold(),
        _ => t.muted(),
    };
    let mut out = if matches!(
        entry.kind,
        EntryKind::User | EntryKind::Assistant | EntryKind::Completion
    ) {
        Vec::new()
    } else {
        vec![Line::from(vec![
            Span::styled(
                format!("{marker} "),
                if entry.kind == EntryKind::Assistant {
                    t.accent()
                } else {
                    title_style
                },
            ),
            Span::styled(
                match entry.kind {
                    EntryKind::Warning => format!("Warning · {}", entry.title),
                    EntryKind::Error => format!("Error · {}", entry.title),
                    _ => entry.title.clone(),
                },
                title_style,
            ),
        ])]
    };
    if entry.kind == EntryKind::Tool {
        out = vec![Line::from(Span::styled(
            format!("[ {} ]", entry.title),
            t.tool_title(false),
        ))];
    }
    let limit = if entry.kind == EntryKind::Tool && !expanded {
        0
    } else {
        usize::MAX
    };
    let body_lines = entry
        .draft
        .as_ref()
        .map(|draft| {
            draft
                .text
                .split('\n')
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| entry.lines.clone());
    let mut code_block: Option<CodeBlock> = None;
    for text in body_lines.iter().take(limit) {
        if matches!(entry.kind, EntryKind::Assistant | EntryKind::Tool) {
            if let Some(language) = text.strip_prefix("```") {
                if code_block.is_none() {
                    out.push(line(
                        format!(
                            "  code / {}",
                            if language.is_empty() {
                                "text"
                            } else {
                                language
                            }
                        ),
                        t.muted(),
                    ));
                }
                code_block = if code_block.is_none() {
                    Some(CodeBlock::new(language))
                } else {
                    None
                };
            } else if let Some(code) = &mut code_block {
                out.push(code.row(text, t).line);
            } else if entry.kind == EntryKind::Tool {
                out.push(line(format!("  {}", expand_tabs(text, 2)), t.muted()));
            } else if let Some(heading) = text
                .strip_prefix("### ")
                .or_else(|| text.strip_prefix("## "))
                .or_else(|| text.strip_prefix("# "))
            {
                out.push(line(format!("  {}", expand_tabs(heading, 2)), t.bold()));
            } else {
                out.push(markdown_line(text, app));
            }
        } else {
            out.push(line(
                if entry.kind == EntryKind::User && text.is_empty() {
                    String::new()
                } else {
                    format!("  {}", expand_tabs(text, 2))
                },
                if entry.kind == EntryKind::Tool {
                    t.muted()
                } else if entry.kind == EntryKind::User {
                    t.sent_prompt()
                } else if entry.kind == EntryKind::Warning {
                    t.warning()
                } else if entry.kind == EntryKind::Error {
                    t.error()
                } else if entry.kind == EntryKind::Completion {
                    t.muted()
                } else {
                    t.base()
                },
            ));
        }
    }
    if entry.kind == EntryKind::User {
        for line in &mut out {
            *line = line.clone().style(t.sent_prompt());
        }
    }
    if body_lines.len() > limit {
        out.push(line("  Ctrl+O to expand", t.muted()));
    }
    out = out
        .into_iter()
        .map(|line| decorate_line(line, app))
        .collect();
    out.push(line("", t.base()));
    out
}

fn expand_tabs(text: &str, mut column: usize) -> String {
    let mut out = String::new();
    for grapheme in text.graphemes(true) {
        if grapheme == "\t" {
            let spaces = 4 - column % 4;
            out.push_str(&" ".repeat(spaces));
            column += spaces;
        } else {
            out.push_str(grapheme);
            column += UnicodeWidthStr::width(grapheme);
        }
    }
    out
}

fn markdown_line(text: &str, app: &App) -> Line<'static> {
    let t = app.theme;
    let mut spans = vec![Span::raw("  ")];
    let mut remaining = text;
    let mut bold = false;
    let mut code = false;
    let mut column = 2usize;
    while !remaining.is_empty() {
        if remaining.starts_with("**") {
            bold = !bold;
            remaining = &remaining[2..];
            continue;
        }
        if remaining.starts_with('`') {
            code = !code;
            remaining = &remaining[1..];
            continue;
        }
        let next = remaining
            .find("**")
            .into_iter()
            .chain(remaining.find('`'))
            .min()
            .unwrap_or(remaining.len());
        let segment = expand_tabs(&remaining[..next], column);
        column += display_width(&segment);
        spans.push(Span::styled(
            segment,
            if code {
                t.panel()
            } else if bold {
                t.bold()
            } else {
                t.base()
            },
        ));
        remaining = &remaining[next..];
    }
    Line::from(spans)
}

/// Drawing, wrapping and runtime navigation share the full width with two-cell margins.
pub fn content_column(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y,
        width,
        area.height,
    )
}
pub fn editor_width(terminal_width: u16) -> usize {
    content_column(Rect::new(0, 0, terminal_width, 1))
        .width
        .saturating_sub(6)
        .max(1) as usize
}
fn approval_rows(app: &App, height: u16) -> u16 {
    if !app.approval {
        0
    } else if height < 28 {
        3
    } else {
        6
    }
}
fn composer_status_rows(app: &App, height: u16) -> u16 {
    u16::from(app.working)
        + u16::from(app.goal.is_some())
        + u16::from(!app.notice.is_empty())
        + approval_rows(app, height)
}
pub fn composer_height(app: &App, width: u16, height: u16) -> u16 {
    let status = composer_status_rows(app, height);
    let fixed = 12
        + u16::from(app.account_email.is_some())
        + status
        + crate::agents_ui::dock_height(app, height)
        + u16::from(app.agents.viewed.is_some()) * 2;
    let max_rows = (height.saturating_mul(3) / 8)
        .max(1)
        .min(height.saturating_sub(fixed).max(1));
    let rows = cache::editor(&app.input.text, editor_width(width))
        .len()
        .min(max_rows as usize) as u16;
    rows.max(1) + 2 + status
}
/// Bottom sheets leave the conversation and global footer on the same surface.
pub fn popup_area(app: &App, area: Rect) -> Rect {
    let preferred = match app.screen {
        Screen::Models => app.model_rows().len().min(10) as u16 + 11,
        Screen::ProviderEdit => 21,
        Screen::Settings => area.height.saturating_sub(9).max(14),
        Screen::Status => 17,
        Screen::Usage => {
            if app.usage.details && area.width.saturating_sub(8) < 104 {
                40
            } else {
                30
            }
        }
        Screen::Stats => 32,
        Screen::Transcript | Screen::Diff => 24,
        Screen::Help | Screen::Preview => 23,
        _ => 18,
    };
    let height = preferred.min(area.height.saturating_sub(6));
    Rect::new(
        area.x,
        area.bottom().saturating_sub(2 + height),
        area.width,
        height,
    )
}

fn usage_body(app: &App, width: u16, height: u16) -> Option<Rect> {
    if width < MIN_WIDTH || height < MIN_HEIGHT {
        return None;
    }
    let sheet = content_column(popup_area(app, Rect::new(0, 0, width, height)));
    let content = inset(sheet, 2, 1);
    let footer_height = if app.notice.is_empty() { 1 } else { 2 };
    let body = Rect::new(
        content.x,
        content.y + 2,
        content.width,
        content.height.saturating_sub(2 + footer_height),
    );
    Some(body)
}
pub fn usage_page_rows(app: &App, width: u16, height: u16) -> usize {
    usage_body(app, width, height).map_or(0, |body| crate::usage::page_rows(app, body))
}
pub fn usage_max_scroll(app: &App, width: u16, height: u16) -> usize {
    usage_body(app, width, height).map_or(0, |body| crate::usage::max_scroll(app, body))
}

pub fn settings_page_rows(app: &App, width: u16, height: u16) -> usize {
    usage_body(app, width, height).map_or(0, |body| {
        crate::settings::page_rows(app, body, width >= 100)
    })
}

pub fn settings_max_scroll(app: &App, width: u16, height: u16) -> usize {
    usage_body(app, width, height).map_or(0, |body| {
        crate::settings::max_scroll(app, body, width >= 100)
    })
}

pub fn settings_scroll_offset(app: &App, width: u16, height: u16) -> usize {
    usage_body(app, width, height).map_or(0, |body| {
        crate::settings::scroll_offset(app, body, width >= 100)
    })
}

pub fn render(frame: &mut Frame, app: &App) {
    let _ = render_interactive(frame, app);
}

pub fn render_interactive(frame: &mut Frame, app: &App) -> UiMap {
    let _time = crate::frame::FrameTime::capture();
    let area = frame.area();
    let mut map = UiMap::default();
    frame.render_widget(Block::default().style(app.theme.base()), area);
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        render_small(frame, area, app);
    } else {
        render_home(frame, area, app, &mut map);
        if app.screen != Screen::Home {
            let sheet = popup_area(app, area);
            map.occlude(sheet);
            frame.render_widget(Clear, sheet);
            frame.render_widget(Block::default().style(app.theme.base()), sheet);
            render_panel(frame, sheet, app, &mut map);
        }
    }
    if !app.theme.color {
        for cell in &mut frame.buffer_mut().content {
            cell.set_fg(Color::Reset);
            cell.set_bg(Color::Reset);
        }
    }
    normalize_ascii_buffer(frame.buffer_mut(), app.ascii);
    map
}

pub fn render_small(frame: &mut Frame, area: Rect, app: &App) {
    let message = vec![
        line("Polaris", app.theme.accent()),
        line("Resize to at least 60 columns x 20 rows.", app.theme.base()),
        line(
            "Your draft and navigation state are preserved.",
            app.theme.muted(),
        ),
    ];
    frame.render_widget(
        Paragraph::new(message).wrap(Wrap { trim: false }),
        inset(area, 1, 0),
    );
}

struct HomeLayout {
    header: logo::HeaderLayout,
    history: Rect,
    composer: Rect,
    footer: Rect,
    agents: Rect,
    banner: Rect,
}
fn home_layout(app: &App, area: Rect) -> HomeLayout {
    let composer = composer_height(app, area.width, area.height).min(area.height.saturating_sub(6));
    let dock_height = crate::agents_ui::dock_height(app, area.height);
    let parts = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(composer),
        Constraint::Length(dock_height),
        Constraint::Length(2),
    ])
    .split(area);
    let body = content_column(parts[0]);
    let header = logo::layout(app, body, body.height);
    let banner_height = if app.agents.viewed.is_some() {
        2.min(body.bottom().saturating_sub(header.history_y))
    } else {
        0
    };
    HomeLayout {
        header,
        banner: Rect::new(
            body.x + 2,
            header.history_y,
            body.width.saturating_sub(4),
            banner_height,
        ),
        history: Rect::new(
            body.x + 2,
            header.history_y + banner_height,
            body.width.saturating_sub(4),
            body.bottom()
                .saturating_sub(header.history_y + banner_height),
        ),
        composer: content_column(parts[1]),
        agents: content_column(parts[2]),
        footer: content_column(parts[3]),
    }
}
pub fn conversation_max_scroll(app: &App, width: u16, height: u16) -> usize {
    if width < MIN_WIDTH || height < MIN_HEIGHT {
        return 0;
    }
    let layout = home_layout(app, Rect::new(0, 0, width, height));
    cache::transcript(app, layout.history.width)
        .total_rows
        .saturating_sub(layout.history.height as usize)
}
fn render_home(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    let layout = home_layout(app, area);
    render_header(frame, layout.header, app);
    let history = layout.history;
    let prepared = cache::transcript(app, history.width);
    let max_scroll = prepared.total_rows.saturating_sub(history.height as usize);
    let offset = max_scroll
        .saturating_sub(app.conversation_scroll())
        .min(u16::MAX as usize) as u16;
    cache::paint(frame, history, app, map, offset as usize, &prepared);
    render_composer_mapped(frame, layout.composer, app, map);
    render_attachment_overlay(frame, history, app, map);
    render_suggestions(frame, history, app, map);
    crate::agents_ui::banner(frame, layout.banner, app, map);
    crate::agents_ui::render_dock(frame, layout.agents, app, map);
    render_footer(frame, layout.footer, app, map);
    if app.screen == Screen::Home
        && app.preview.is_none()
        && !app.completion_open()
        && (offset as usize) < max_scroll
    {
        let label = if app.ascii {
            "To latest v"
        } else {
            "To latest ↓"
        };
        let width = UnicodeWidthStr::width(label) as u16 + 2;
        let rect = Rect::new(
            history.x + history.width.saturating_sub(width) / 2,
            history.bottom().saturating_sub(1),
            width,
            1,
        );
        let rect = align_overlay_to_glyphs(frame, rect);
        map.occlude(rect);
        let hovered = app
            .mouse_position
            .is_some_and(|(x, y)| x >= rect.x && x < rect.right() && y == rect.y);
        put(
            frame,
            rect,
            format!(" {label} "),
            app.theme.latest_button(hovered),
        );
        map.add_target(rect, HitAction::ToLatest);
    }
}

/// An overlay must not leave half of a wide source glyph selectable or tinted.
fn align_overlay_to_glyphs(frame: &mut Frame, mut rect: Rect) -> Rect {
    let mut left = rect.x;
    let mut right = rect.right();
    for y in rect.y..rect.bottom() {
        for x in frame.area().x..frame.area().right() {
            let width = UnicodeWidthStr::width(frame.buffer_mut()[(x, y)].symbol()).max(1) as u16;
            if width > 1 && x < right && x + width > left {
                left = left.min(x);
                right = right.max(x + width).min(frame.area().right());
            }
        }
    }
    rect.x = left;
    rect.width = right - left;
    rect
}

#[derive(Clone)]
struct DisplaySourceLine {
    display: String,
    prefix_bytes: usize,
    source: String,
    source_start: usize,
    display_index: usize,
}

fn strip_inline_markdown(text: &str) -> String {
    let tick = char::from(96);
    let mut out = String::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.starts_with("**") {
            remaining = &remaining[2..];
        } else if remaining.starts_with(tick) {
            remaining = &remaining[1..];
        } else {
            let next = remaining
                .find("**")
                .into_iter()
                .chain(remaining.find(tick))
                .min()
                .unwrap_or(remaining.len());
            out.push_str(&remaining[..next]);
            remaining = &remaining[next..];
        }
    }
    out
}

fn display_source_lines(entry: &Entry, app: &App, expanded: bool) -> Vec<DisplaySourceLine> {
    let body = entry
        .draft
        .as_ref()
        .map(|draft| {
            draft
                .text
                .split('\n')
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| entry.lines.clone());
    let limit = if entry.kind == EntryKind::Tool && !expanded {
        0
    } else {
        usize::MAX
    };
    let mut lines = Vec::new();
    let mut display_index = 0usize;
    let mut code_block: Option<CodeBlock> = None;
    let fence = char::from(96).to_string().repeat(3);
    for raw in body.iter().take(limit) {
        if matches!(entry.kind, EntryKind::Assistant | EntryKind::Tool) {
            if let Some(language) = raw.strip_prefix(&fence) {
                if code_block.is_none() {
                    display_index += 1;
                }
                code_block = if code_block.is_none() {
                    Some(CodeBlock::new(language))
                } else {
                    None
                };
                continue;
            }
            if let Some(code) = &mut code_block {
                let code_row = code.row(raw, app.theme);
                lines.push(DisplaySourceLine {
                    display: format!("{}{raw}", code_row.prefix),
                    prefix_bytes: code_row.prefix.len(),
                    source: raw.clone(),
                    source_start: 0,
                    display_index,
                });
            } else {
                let semantic = if entry.kind == EntryKind::Tool {
                    raw.clone()
                } else {
                    raw.strip_prefix("### ")
                        .or_else(|| raw.strip_prefix("## "))
                        .or_else(|| raw.strip_prefix("# "))
                        .map(str::to_owned)
                        .unwrap_or_else(|| strip_inline_markdown(raw))
                };
                lines.push(DisplaySourceLine {
                    display: format!("  {semantic}"),
                    prefix_bytes: 2,
                    source: semantic,
                    source_start: 0,
                    display_index,
                });
            }
        } else {
            lines.push(DisplaySourceLine {
                display: if entry.kind == EntryKind::User && raw.is_empty() {
                    String::new()
                } else {
                    format!("  {raw}")
                },
                prefix_bytes: if entry.kind == EntryKind::User && raw.is_empty() {
                    0
                } else {
                    2
                },
                source: raw.clone(),
                source_start: 0,
                display_index,
            });
        }
        display_index += 1;
    }
    lines
}

/// Ask the same Paragraph renderer used by the transcript to wrap byte-tagged
/// graphemes. Tags live only in this offscreen buffer, so word-boundary whitespace,
/// expanded tabs and wide Unicode cells map back to their exact source offsets.
fn transcript_source_rows(display: &str, width: u16) -> Vec<crate::composer::LayoutRow> {
    use ratatui::{buffer::Buffer, widgets::Widget};
    let width = width.max(1);
    let mut column = 0usize;
    let spans = display
        .grapheme_indices(true)
        .map(|(byte, grapheme)| {
            let symbol = expand_tabs(grapheme, column);
            column += UnicodeWidthStr::width(symbol.as_str());
            Span::styled(
                symbol,
                Style::default().fg(Color::Rgb(
                    ((byte >> 16) & 255) as u8,
                    ((byte >> 8) & 255) as u8,
                    (byte & 255) as u8,
                )),
            )
        })
        .collect::<Vec<_>>();
    let paragraph = Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false });
    let height = paragraph.line_count(width).max(1).min(u16::MAX as usize) as u16;
    let rect = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(rect);
    paragraph.render(rect, &mut buffer);
    (0..height)
        .map(|y| {
            let mut cells = Vec::new();
            let mut end = 0usize;
            let mut x = 0u16;
            while x < width {
                let cell = &buffer[(x, y)];
                let step = UnicodeWidthStr::width(cell.symbol()).max(1) as u16;
                if let Color::Rgb(r, g, b) = cell.fg {
                    let byte = ((r as usize) << 16) | ((g as usize) << 8) | b as usize;
                    if display.is_char_boundary(byte) && byte < display.len() {
                        cells.push((x as usize, byte));
                        end = byte + display[byte..].graphemes(true).next().map_or(0, str::len);
                    }
                }
                x += step;
            }
            crate::composer::LayoutRow {
                start: cells.first().map_or(end, |(_, byte)| *byte),
                end,
                cells,
            }
        })
        .collect()
}

fn wrapped_source_rows(text: &str, width: u16) -> Vec<crate::composer::LayoutRow> {
    let mut base = 0;
    text.split('\n')
        .flat_map(|line| {
            let rows = transcript_source_rows(line, width)
                .into_iter()
                .map(|mut row| {
                    row.start += base;
                    row.end += base;
                    for (_, byte) in &mut row.cells {
                        *byte += base;
                    }
                    row
                })
                .collect::<Vec<_>>();
            base += line.len() + 1;
            rows
        })
        .collect()
}

fn map_entry_sources(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    map: &mut UiMap,
    scroll: usize,
    prepared: &cache::Transcript,
) {
    let selection = app.view_selection.as_ref();
    let mut conversation = TextRegion::new(TextRegionId::Conversation, area, String::new());
    conversation.source = prepared.source.clone();
    for (entry_index, cached) in prepared.entries.iter().enumerate() {
        let entry = &cached.entry;
        let rendered = &cached.lines;
        let rendered_rows = &cached.counts;
        let logical = prepared.starts[entry_index];
        if entry.kind == EntryKind::Tool {
            for title_row in 0..rendered_rows.first().copied().unwrap_or(0) {
                let absolute = logical + title_row;
                if absolute < scroll || absolute >= scroll + area.height as usize {
                    continue;
                }
                let y = area.y + (absolute - scroll) as u16;
                let right = (area.x..area.right())
                    .rev()
                    .find(|&x| !frame.buffer_mut()[(x, y)].symbol().trim().is_empty());
                if let Some(last) = right {
                    let glyph_width = UnicodeWidthStr::width(frame.buffer_mut()[(last, y)].symbol())
                        .max(1) as u16;
                    let rect = Rect::new(
                        area.x,
                        y,
                        last.saturating_add(glyph_width).min(area.right()) - area.x,
                        1,
                    );
                    let hovered = app
                        .mouse_position
                        .is_some_and(|(x, y)| rect.contains((x, y).into()));
                    for x in rect.x..rect.right() {
                        frame.buffer_mut()[(x, y)].set_style(app.theme.tool_title(hovered));
                    }
                    map.add_target(rect, HitAction::ToggleTool(entry_index));
                }
            }
        }
        let descriptors = &cached.descriptors;
        let conversation_start = prepared.source_offsets[entry_index];
        let id = TextRegionId::Entry(entry_index);
        let decorations = &cached.decorations;
        let mut region = TextRegion::new(id.clone(), area, String::new());
        region.source = cached.source.clone();
        if let Some(draft) = &entry.draft {
            for span in &draft.spans {
                if matches!(&span.attachment.payload, AttachmentPayload::Text(_)) {
                    conversation.replacements.push((
                        (conversation_start + span.range.start)
                            ..(conversation_start + span.range.end),
                        span.attachment.clone(),
                    ));
                }
            }
        }
        if entry.kind == EntryKind::User {
            let body_rows = rendered_rows
                .iter()
                .take(rendered_rows.len().saturating_sub(1))
                .sum::<usize>();
            for absolute in logical..logical + body_rows {
                if absolute >= scroll && absolute < scroll + area.height as usize {
                    let y = area.y + (absolute - scroll) as u16;
                    for x in area.x..area.right() {
                        frame.buffer_mut()[(x, y)].set_style(app.theme.sent_prompt());
                    }
                }
            }
        }
        for (descriptor_index, descriptor) in descriptors.iter().enumerate() {
            let line_index = usize::from(!matches!(
                entry.kind,
                EntryKind::User | EntryKind::Assistant | EntryKind::Completion
            )) + descriptor.display_index;
            if line_index >= rendered.len() {
                continue;
            }
            let absolute_start = logical + rendered_rows.iter().take(line_index).sum::<usize>();
            if absolute_start + rendered_rows[line_index] <= scroll
                || absolute_start >= scroll + area.height as usize
            {
                continue;
            }
            if entry.kind == EntryKind::AgentLaunch
                && app.agents.viewed.is_none()
                && let Some(id) = app
                    .agents
                    .launch_entries
                    .get(&entry_index)
                    .and_then(|ids| ids.get(descriptor.display_index))
            {
                for absolute in absolute_start..absolute_start + rendered_rows[line_index] {
                    if absolute >= scroll && absolute < scroll + area.height as usize {
                        let rect =
                            Rect::new(area.x, area.y + (absolute - scroll) as u16, area.width, 1);
                        let hovered = app.mouse_position.is_some_and(|p| rect.contains(p.into()));
                        for x in rect.x..rect.right() {
                            frame.buffer_mut()[(x, rect.y)].set_style(
                                app.theme.base().patch(app.theme.control(hovered, false)),
                            );
                        }
                        map.add_target(
                            rect,
                            HitAction::Agent(crate::agents::AgentAction::Open(Some(*id))),
                        );
                    }
                }
            }
            let source_rows = cached.source_rows(descriptor_index, area.width);
            for (wrapped_index, display_row) in source_rows.iter().enumerate() {
                let absolute = absolute_start + wrapped_index;
                if absolute < scroll || absolute >= scroll + area.height as usize {
                    continue;
                }
                let y = area.y + (absolute - scroll) as u16;
                for &(col, display_byte) in &display_row.cells {
                    if display_byte < descriptor.prefix_bytes {
                        continue;
                    }
                    let byte = descriptor.source_start + display_byte - descriptor.prefix_bytes;
                    if byte >= region.source.len() || !region.source.is_char_boundary(byte) {
                        continue;
                    }
                    let x = area.x + col as u16;
                    region.push_cell(x, y, byte);
                    let selected = selection.is_some_and(|sel| {
                        let offset = if sel.region == TextRegionId::Conversation {
                            conversation_start
                        } else {
                            0
                        };
                        (sel.region == id || sel.region == TextRegionId::Conversation)
                            && byte + offset >= sel.anchor.min(sel.end)
                            && byte + offset < sel.anchor.max(sel.end)
                    });
                    if let Some(style) = lexical_style(app, byte, decorations) {
                        frame.buffer_mut()[(x, y)].set_style(style);
                    }
                    if let Some(draft) = entry.draft.as_ref()
                        && let Some(span) =
                            draft.spans.iter().find(|span| span.range.contains(&byte))
                    {
                        let grapheme = descriptor.display[display_byte..]
                            .graphemes(true)
                            .next()
                            .unwrap_or("");
                        let width = UnicodeWidthStr::width(grapheme).max(1) as u16;
                        map.add_target(
                            Rect::new(x, y, width, 1),
                            HitAction::Attachment(span.attachment.id),
                        );
                        if app.theme.color {
                            frame.buffer_mut()[(x, y)].set_fg(
                                if app.theme.color
                                    && app.theme.rgb
                                    && app.theme.mode == ThemeMode::Dark
                                {
                                    Color::Rgb(130, 184, 200)
                                } else {
                                    Color::Cyan
                                },
                            );
                        }
                    }
                    if selected {
                        shade_selection_cell(frame, x, y, app);
                    }
                }
                let end_display = display_row.end;
                if end_display >= descriptor.prefix_bytes {
                    let end = (descriptor.source_start + end_display - descriptor.prefix_bytes)
                        .min(region.source.len());
                    if region.source.is_char_boundary(end) {
                        let end_x = display_row.cells.last().map_or(0, |(col, byte)| {
                            col + UnicodeWidthStr::width(
                                descriptor.display[*byte..]
                                    .graphemes(true)
                                    .next()
                                    .unwrap_or(""),
                            )
                            .max(1)
                        });
                        region.push_cell(area.x + end_x as u16, y, end);
                    }
                }
            }
        }
        conversation.cells.extend(
            region
                .cells
                .iter()
                .map(|cell| crate::interaction::TextCell {
                    byte_offset: conversation_start + cell.byte_offset,
                    ..*cell
                }),
        );
        map.add_text_region(region);
    }
    map.add_text_region(conversation);
}

pub fn render_composer(frame: &mut Frame, area: Rect, app: &App) {
    let mut ignored = UiMap::default();
    render_composer_mapped(frame, area, app, &mut ignored);
}

use crate::syntax::{TokenDecoration, TokenKind, token_decorations};

fn lexical_style(app: &App, byte: usize, decorations: &[TokenDecoration]) -> Option<Style> {
    let index = decorations.partition_point(|token| token.range.end <= byte);
    let token = decorations
        .get(index)
        .filter(|token| token.range.contains(&byte))?;
    match token.kind {
        TokenKind::Link => Some(Style::default().add_modifier(Modifier::UNDERLINED)),
        TokenKind::Mention if app.theme.color => Some(app.theme.mention()),
        TokenKind::Command if app.theme.color => Some(app.theme.selected()),
        TokenKind::Setting if app.theme.color => Some(app.theme.setting(false)),
        _ => None,
    }
}

/// Apply lexical overlays without replacing the line's foreground/background or
/// Markdown emphasis. This also covers transcript popups and wrapped history.
fn decorate_line(mut line: Line<'static>, app: &App) -> Line<'static> {
    let decorations = token_decorations(&line.to_string());
    if decorations.is_empty() {
        return line;
    }
    let mut spans = Vec::new();
    let mut base = 0usize;
    for span in line.spans {
        let end = base + span.content.len();
        let mut cursor = base;
        for token in decorations
            .iter()
            .filter(|t| t.range.start < end && t.range.end > base)
        {
            let start = token.range.start.max(base);
            let stop = token.range.end.min(end);
            if cursor < start {
                spans.push(Span::styled(
                    span.content[cursor - base..start - base].to_owned(),
                    span.style,
                ));
            }
            let style = lexical_style(app, start, &decorations)
                .map_or(span.style, |overlay| span.style.patch(overlay));
            spans.push(Span::styled(
                span.content[start - base..stop - base].to_owned(),
                style,
            ));
            cursor = stop;
        }
        if cursor < end {
            spans.push(Span::styled(
                span.content[cursor - base..].to_owned(),
                span.style,
            ));
        }
        base = end;
    }
    line.spans = spans;
    line
}

fn shade_selection_cell(frame: &mut Frame, x: u16, y: u16, app: &App) {
    let cell = &frame.buffer_mut()[(x, y)];
    let width = UnicodeWidthStr::width(cell.symbol()).max(1) as u16;
    let style = app
        .theme
        .copy_selection(cell.fg, cell.bg, cell.symbol().trim().is_empty());
    let end = x.saturating_add(width).min(frame.area().right());
    for column in x..end {
        frame.buffer_mut()[(column, y)].set_style(style);
    }
}

#[cfg(test)]
fn rich_style(app: &App, byte: usize, decorations: &[TokenDecoration]) -> Style {
    let completion = app
        .completion_open()
        .then(|| active_token(&app.input.text, app.input.cursor))
        .flatten();
    rich_style_prepared(app, byte, decorations, completion.as_ref())
}
fn rich_style_prepared(
    app: &App,
    byte: usize,
    decorations: &[TokenDecoration],
    completion: Option<&crate::completion::CompletionToken>,
) -> Style {
    if app.theme.color && app.input.spans.iter().any(|s| s.range.contains(&byte)) {
        return app
            .theme
            .base()
            .fg(if app.theme.rgb && app.theme.mode == ThemeMode::Dark {
                Color::Rgb(130, 184, 200)
            } else {
                Color::Cyan
            });
    }
    let completion = completion.filter(|token| token.range.contains(&byte));
    if completion
        .as_ref()
        .is_some_and(|token| token.query.is_empty())
    {
        return app.theme.muted();
    }
    if let Some(style) = lexical_style(app, byte, decorations) {
        return app.theme.base().patch(style);
    }
    if completion.is_some() {
        return app.theme.muted();
    }
    app.theme.base()
}

fn render_composer_mapped(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    let status = inset(area, 2, 0);
    let t = app.theme;
    let mut y = 0u16;
    if app.working {
        let time = crate::work_summary::elapsed_label(app.working_elapsed());
        put(
            frame,
            row(status, y),
            Line::from(vec![
                Span::styled(if app.ascii { "* " } else { "• " }, t.tool_title(false)),
                Span::styled(
                    format!("Working ({time} {} ", if app.ascii { "." } else { "·" }),
                    t.muted(),
                ),
                Span::styled("esc", t.bold()),
                Span::styled(" to interrupt)", t.muted()),
            ]),
            t.muted(),
        );
        y += 1;
    }
    if app.approval {
        put(frame, row(status, y), "Action approval [demo]", t.warning());
        y += 1;
        if approval_rows(app, frame.area().height) == 6 {
            put(
                frame,
                row(status, y),
                "Proposed edit · src/validation.rs · workspace only",
                t.base(),
            );
            y += 1;
            put(
                frame,
                row(status, y),
                "Extract the repeated guard into a shared helper.",
                t.muted(),
            );
            y += 1;
        }
        let mut x = status.x;
        for (i, label) in ["Allow once", "Decline", "View details"].iter().enumerate() {
            let width = label.width() as u16 + 4;
            let rect = Rect::new(x, status.y + y, width, 1);
            render_control(
                frame,
                rect,
                label,
                t.base(),
                app,
                app.approval_selected == i,
            );
            map.add_target(rect, HitAction::Approval(i));
            x += width + 1;
        }
        y += 1;
        put(
            frame,
            row(status, y),
            "Left/Right select · Enter confirm · Esc decline",
            t.muted(),
        );
        y += 1;
        if approval_rows(app, frame.area().height) == 6 {
            y += 1;
        }
    }
    if !app.notice.is_empty() {
        put(
            frame,
            row(status, y),
            clip(&app.notice, status.width as usize),
            t.warning(),
        );
        y += 1;
    }
    if let Some(goal) = &app.goal {
        let state = if goal.is_active() { "active" } else { "paused" };
        let time = crate::work_summary::elapsed_label(goal.elapsed());
        put(
            frame,
            row(status, y),
            Line::from(format!("goal {state} ({time})")).right_aligned(),
            t.goal_status(),
        );
        y += 1;
    }
    let border = Rect::new(
        area.x,
        area.y + y,
        area.width,
        area.height.saturating_sub(y),
    );
    let rule = if app.ascii { "-" } else { "─" }.repeat(border.width as usize);
    put(frame, row(border, 0), rule.clone(), t.rule());
    if app.agents.viewed.is_some() {
        let label = " prompt to Main ";
        if border.width > label.len() as u16 + 4 {
            put(
                frame,
                Rect::new(
                    border.right() - label.len() as u16 - 2,
                    border.y,
                    label.len() as u16,
                    1,
                ),
                label,
                t.muted(),
            );
        }
    }
    put(frame, row(border, border.height - 1), rule, t.rule());
    let content = inset(border, 2, 1);
    let y = 0u16;
    let prompt_width = 2usize;
    let text_width = usize::from(content.width.saturating_sub(prompt_width as u16).max(1));
    let rows = cache::editor(&app.input.text, text_width);
    let cursor_row = rows
        .iter()
        .rposition(|r| r.start <= app.input.cursor)
        .unwrap_or(0);
    let cursor_col = display_width(
        &app.input.text[rows[cursor_row].start..app.input.cursor.min(rows[cursor_row].end)],
    );
    let visible = content.height.saturating_sub(y).max(1) as usize;
    let max_first = rows.len().saturating_sub(visible);
    let first = if app.composer_scroll == 0 {
        cursor_row
            .saturating_sub(visible.saturating_sub(1))
            .min(max_first)
    } else {
        max_first.saturating_sub(app.composer_scroll.min(max_first))
    };
    let text_rect = Rect::new(
        content.x + prompt_width as u16,
        content.y + y,
        content.width.saturating_sub(prompt_width as u16),
        visible.min(content.height.saturating_sub(y) as usize) as u16,
    );
    let mut region = TextRegion::new(TextRegionId::Composer, text_rect, app.input.text.clone());
    let selection = app.view_selection.as_ref();
    let selected_text = app.input.selection();
    let decorations = token_decorations(&app.input.text);
    let completion = app
        .completion_open()
        .then(|| active_token(&app.input.text, app.input.cursor))
        .flatten();
    map.add_target(content, HitAction::Composer);
    for (shown, row_index) in (first..rows.len().min(first + visible)).enumerate() {
        let layout_row = &rows[row_index];
        let yy = content.y + y + shown as u16;
        if yy >= content.bottom() {
            break;
        }
        let prompt = if shown == 0 { "> " } else { "  " };
        put(
            frame,
            Rect::new(content.x, yy, prompt_width as u16, 1),
            prompt,
            t.muted(),
        );
        let mut spans = Vec::new();
        let mut end_x = 0usize;
        for &(col, byte) in &layout_row.cells {
            let grapheme = app.input.text[byte..]
                .graphemes(true)
                .next()
                .map_or(0, str::len);
            let next = (byte + grapheme).min(app.input.text.len());
            let cell_text = &app.input.text[byte..next];
            let rendered = if cell_text == "\t" {
                " ".repeat(4 - col % 4)
            } else {
                cell_text.to_string()
            };
            let width = if cell_text == "\t" {
                4 - col % 4
            } else {
                UnicodeWidthStr::width(cell_text)
            };
            spans.push(Span::styled(
                rendered,
                rich_style_prepared(app, byte, &decorations, completion.as_ref()),
            ));
            region.push_cell(text_rect.x + col as u16, yy, byte);
            end_x = col + width;
            for attachment in &app.input.spans {
                if attachment.range.start == byte {
                    let span_width = if cell_text == "\t" {
                        width
                    } else {
                        UnicodeWidthStr::width(&app.input.text[attachment.range.clone()]).max(1)
                    };
                    map.add_target(
                        Rect::new(text_rect.x + col as u16, yy, span_width as u16, 1),
                        HitAction::Attachment(attachment.attachment.id),
                    );
                }
            }
        }
        region.push_cell(
            text_rect.x + end_x.min(u16::MAX as usize) as u16,
            yy,
            layout_row.end,
        );
        put(
            frame,
            Rect::new(text_rect.x, yy, text_rect.width, 1),
            Line::from(spans),
            t.base(),
        );
        // Paragraph resets continuation cells for wide glyphs; shade them after rendering.
        for &(col, byte) in &layout_row.cells {
            let selected = selection.is_some_and(|s| {
                s.region == TextRegionId::Composer
                    && byte >= s.anchor.min(s.end)
                    && byte < s.anchor.max(s.end)
            }) || selected_text
                .as_ref()
                .is_some_and(|range| range.contains(&byte));
            if selected {
                let width = if app.input.text[byte..].starts_with('\t') {
                    4 - col % 4
                } else {
                    1
                };
                for offset in 0..width {
                    let x = text_rect.x + col as u16 + offset as u16;
                    if x < text_rect.right() {
                        shade_selection_cell(frame, x, yy, app);
                    }
                }
            }
        }
    }
    if app.input.text.is_empty() {
        put(
            frame,
            row(text_rect, 0),
            "What's on your mind for Polaris?",
            t.placeholder(),
        );
    }
    map.add_text_region(region);
    if app.screen == Screen::Home
        && !app.approval
        && !app.agents.focused
        && cursor_row >= first
        && cursor_row < first + visible
    {
        frame.set_cursor_position((
            text_rect.x + cursor_col.min(u16::MAX as usize) as u16,
            content.y + y + cursor_row.saturating_sub(first) as u16,
        ));
    }
}
fn render_suggestions(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    if !app.completion_open() || area.height < 2 {
        return;
    }
    let items = app.suggestions();
    if items.is_empty() {
        return;
    }
    let count = items
        .len()
        .min(8)
        .min(area.height.saturating_sub(1) as usize);
    let start = app
        .selected
        .saturating_sub(count.saturating_sub(1))
        .min(items.len().saturating_sub(count));
    let top = area.bottom().saturating_sub(count as u16 + 1);
    let panel = Rect::new(frame.area().x, top, frame.area().width, count as u16 + 1);
    let content = Rect::new(
        area.x + 2,
        top + 1,
        area.width.saturating_sub(4),
        count as u16,
    );
    map.occlude(panel);
    map.completion_rect = Some(panel);
    frame.render_widget(Clear, panel);
    frame.render_widget(Block::default().style(app.theme.base()), panel);
    for (visible, index) in (start..start + count).enumerate() {
        let item = &items[index];
        let marker = if index == app.selected { ">" } else { " " };
        let name = if item.kind == SuggestionKind::File {
            format!("@{}", item.label)
        } else {
            item.label.clone()
        };
        let detail = if content.width >= 72 {
            format!("  {}", item.detail)
        } else {
            String::new()
        };
        let label = clip(&format!("{marker} {name}{detail}"), content.width as usize);
        let rr = row(content, visible as u16);
        let hovered = app
            .mouse_position
            .is_some_and(|(x, y)| rr.contains((x, y).into()));
        let focused = index == app.selected;
        let style = if !app.theme.color {
            if hovered {
                app.theme.base().add_modifier(Modifier::BOLD)
            } else if focused {
                app.theme.base().add_modifier(Modifier::UNDERLINED)
            } else {
                app.theme.base()
            }
        } else if item.kind == SuggestionKind::Setting && (hovered || focused) {
            app.theme.setting(hovered)
        } else if hovered {
            let rgb = app.theme.rgb && app.theme.mode == ThemeMode::Dark;
            Style::default().fg(match (item.kind == SuggestionKind::File, rgb) {
                (true, true) => Color::Rgb(255, 253, 230),
                (false, true) => Color::Rgb(219, 245, 176),
                (true, false) => Color::LightYellow,
                (false, false) => Color::White,
            })
        } else if focused && item.kind == SuggestionKind::File {
            app.theme.mention()
        } else if focused {
            Style::default().fg(if app.theme.rgb && app.theme.mode == ThemeMode::Dark {
                Color::Rgb(182, 227, 91)
            } else {
                Color::LightGreen
            })
        } else {
            app.theme.muted()
        };
        put(frame, rr, label, style);
        map.add_target(rr, HitAction::Suggestion(index));
    }
}

fn render_attachment_overlay(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    let Some(attachment) = app.preview.as_ref() else {
        return;
    };
    if area.width < 28 || area.height < 5 {
        return;
    }
    let height = 10u16.min(area.height);
    let width = area.width.saturating_sub(4);
    let panel = Rect::new(
        area.x + 2,
        area.bottom().saturating_sub(height),
        width,
        height,
    );
    map.occlude(panel);
    frame.render_widget(Clear, panel);
    frame.render_widget(Block::default().style(app.theme.base()), panel);

    let is_text = matches!(attachment.payload, AttachmentPayload::Text(_));
    let metadata = match &attachment.payload {
        AttachmentPayload::Text(text) => format!("{} · {} bytes", attachment.label, text.len()),
        AttachmentPayload::Image { width, height, .. } => {
            format!("{} · {} × {} px", attachment.label, width, height)
        }
    };
    let title_width = panel.width.saturating_sub(if is_text { 31 } else { 22 });
    put(
        frame,
        Rect::new(panel.x, panel.y, title_width, 1),
        clip(&metadata, title_width as usize),
        app.theme.accent(),
    );
    let mut x = panel.x + title_width;
    if is_text {
        put(
            frame,
            Rect::new(x, panel.y, 10, 1),
            "[ expand ]",
            app.theme.muted(),
        );
        map.add_target(Rect::new(x, panel.y, 10, 1), HitAction::PreviewExpand);
        x += 10;
        put(
            frame,
            Rect::new(x, panel.y, 9, 1),
            "[ copy ]",
            app.theme.muted(),
        );
        map.add_target(Rect::new(x, panel.y, 9, 1), HitAction::PreviewCopy);
        x += 9;
    } else {
        put(
            frame,
            Rect::new(x, panel.y, 13, 1),
            "Image preview",
            app.theme.muted(),
        );
        x += 13;
    }
    put(
        frame,
        Rect::new(x, panel.y, panel.right().saturating_sub(x), 1),
        "[ close ]",
        app.theme.muted(),
    );
    map.add_target(
        Rect::new(x, panel.y, panel.right().saturating_sub(x), 1),
        HitAction::PreviewClose,
    );

    let body = Rect::new(
        panel.x,
        panel.y + 1,
        panel.width,
        panel.height.saturating_sub(1),
    );
    match &attachment.payload {
        AttachmentPayload::Text(text) => {
            let source_rows = wrapped_source_rows(text, body.width.max(1));
            let max = source_rows.len().saturating_sub(body.height as usize);
            let offset = app.preview_scroll.min(max);
            let lines = text
                .split('\n')
                .map(|s| line(expand_tabs(s, 0), app.theme.base()))
                .collect::<Vec<_>>();
            frame.render_widget(
                Paragraph::new(lines)
                    .wrap(Wrap { trim: false })
                    .scroll((offset.min(u16::MAX as usize) as u16, 0)),
                body,
            );
            let id = TextRegionId::AttachmentPreview(attachment.id);
            let mut region = TextRegion::new(id.clone(), body, text.clone());
            for (shown, row_index) in
                (offset..source_rows.len().min(offset + body.height as usize)).enumerate()
            {
                let layout_row = &source_rows[row_index];
                let y = body.y + shown as u16;
                for &(col, byte) in &layout_row.cells {
                    region.push_cell(body.x + col as u16, y, byte);
                    if app.view_selection.as_ref().is_some_and(|sel| {
                        sel.region == id
                            && byte >= sel.anchor.min(sel.end)
                            && byte < sel.anchor.max(sel.end)
                    }) {
                        shade_selection_cell(frame, body.x + col as u16, y, app);
                    }
                }
                let end_x = layout_row.cells.last().map_or(0, |(col, byte)| {
                    let grapheme = text[*byte..].graphemes(true).next().unwrap_or("");
                    col + if grapheme == "\t" {
                        4 - col % 4
                    } else {
                        UnicodeWidthStr::width(grapheme)
                    }
                });
                region.push_cell(body.x + end_x as u16, y, layout_row.end);
            }
            map.add_text_region(region);
        }
        AttachmentPayload::Image {
            width,
            height,
            rgba,
        } => {
            if app.ascii || !app.theme.color || app.theme.mode != ThemeMode::Dark {
                let message = format!(
                    "Image · {} × {} px · color preview unavailable",
                    width, height
                );
                put(
                    frame,
                    body,
                    clip(&message, body.width as usize),
                    app.theme.muted(),
                );
            } else {
                draw_halfblock_image(frame, body, *width, *height, rgba);
            }
        }
    }
    map.preview_rect = Some(body);
}

fn draw_halfblock_image(frame: &mut Frame, area: Rect, width: usize, height: usize, rgba: &[u8]) {
    if width == 0
        || height == 0
        || area.width == 0
        || area.height == 0
        || rgba.len() < width.saturating_mul(height).saturating_mul(4)
    {
        return;
    }
    let scale = (width as f64 / area.width as f64)
        .max(height as f64 / (area.height as f64 * 2.0))
        .max(1.0);
    let out_w = (width as f64 / scale).floor().max(1.0) as usize;
    let out_h = (height as f64 / scale).floor().max(1.0) as usize;
    let x0 = area.x
        + area
            .width
            .saturating_sub(out_w.min(area.width as usize) as u16)
            / 2;
    let y0 = area.y
        + area
            .height
            .saturating_sub(out_h.div_ceil(2).min(area.height as usize) as u16)
            / 2;
    for cy in 0..out_h.div_ceil(2).min(area.height as usize) {
        for cx in 0..out_w.min(area.width as usize) {
            let sx = (cx as f64 * scale).floor() as usize;
            let top_y = (cy * 2) as f64 * scale;
            let bot_y = (cy * 2 + 1) as f64 * scale;
            let top = sample_rgba(rgba, width, height, sx, top_y.floor() as usize);
            let bottom = if cy * 2 + 1 < out_h {
                sample_rgba(rgba, width, height, sx, bot_y.floor() as usize)
            } else {
                (0, 0, 0)
            };
            let cell = &mut frame.buffer_mut()[(x0 + cx as u16, y0 + cy as u16)];
            cell.set_symbol("▀");
            cell.set_fg(Color::Rgb(top.0, top.1, top.2));
            cell.set_bg(Color::Rgb(bottom.0, bottom.1, bottom.2));
        }
    }
}

fn sample_rgba(rgba: &[u8], width: usize, height: usize, x: usize, y: usize) -> (u8, u8, u8) {
    let x = x.min(width.saturating_sub(1));
    let y = y.min(height.saturating_sub(1));
    let i = (y * width + x) * 4;
    let a = rgba[i + 3] as u16;
    (
        (rgba[i] as u16 * a / 255) as u8,
        (rgba[i + 1] as u16 * a / 255) as u8,
        (rgba[i + 2] as u16 * a / 255) as u8,
    )
}

pub(crate) fn render_control(
    frame: &mut Frame,
    rect: Rect,
    label: &str,
    ink: Style,
    app: &App,
    focused: bool,
) {
    let hovered = app.mouse_position.is_some_and(|p| rect.contains(p.into()));
    let overlay = app.theme.control(hovered, focused);
    put(
        frame,
        rect,
        Line::from(vec![
            Span::styled("[ ", app.theme.rule().patch(overlay)),
            Span::styled(label.to_owned(), ink.patch(overlay)),
            Span::styled(" ]", app.theme.rule().patch(overlay)),
        ]),
        app.theme.base(),
    );
}
fn render_footer(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    let t = app.theme;
    let content = inset(area, 2, 0);
    let footer = row(content, content.height.saturating_sub(1));
    let mode = app.mode_status();
    let effort = app.effort_label();
    let mode_w = mode.width() as u16 + 4;
    let effort_w = effort.width() as u16 + 4;
    let model = clip(
        model_label(app.model_label()),
        content.width.saturating_sub(mode_w + effort_w + 6) as usize,
    );
    let model_w = model.width() as u16 + 4;
    let total = mode_w + model_w + effort_w + 2;
    let mut x = content.right().saturating_sub(total);
    let hint = app.toast_text().unwrap_or("? for shortcuts");
    if hint.width() + 2 <= x.saturating_sub(content.x) as usize {
        let r = Rect::new(content.x, footer.y, hint.width() as u16, 1);
        put(frame, r, hint.to_owned(), t.muted());
        map.add_target(r, HitAction::FooterHelp);
    }
    for (label, width, ink, action) in [
        (
            mode,
            mode_w,
            if app.mode == 0 {
                t.auto_mode()
            } else {
                t.plan_mode()
            },
            HitAction::FooterMode,
        ),
        (model.as_str(), model_w, t.base(), HitAction::FooterModel),
        (effort, effort_w, t.muted(), HitAction::FooterModel),
    ] {
        let r = Rect::new(x, footer.y, width, 1);
        render_control(frame, r, label, ink, app, false);
        map.add_target(r, action);
        x += width + 1;
    }
}

fn render_panel(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    let t = app.theme;
    let column = content_column(area);
    if app.screen == Screen::Stats {
        crate::stats::render(frame, column, app, map);
        return;
    }
    render_outline(frame, column, app);
    let content = inset(column, 2, 1);
    put(
        frame,
        row(content, 0),
        app.screen.title().to_string(),
        t.bold(),
    );
    if app.screen == Screen::Usage {
        let badge = if app.metrics_demo {
            "DEMO DATA"
        } else {
            "Usage unavailable"
        };
        let width = badge.width() as u16;
        if content.width >= width + 8 {
            put(
                frame,
                Rect::new(content.right() - width, content.y, width, 1),
                badge,
                (if app.metrics_demo {
                    t.warning()
                } else {
                    t.muted()
                })
                .remove_modifier(Modifier::BOLD),
            );
        }
    }
    if app.screen == Screen::Settings {
        let status = app.preferences.status.label();
        let w = status.width() as u16;
        put(
            frame,
            Rect::new(content.right().saturating_sub(w), content.y, w, 1),
            status,
            (match app.preferences.status {
                crate::preferences::SaveStatus::Error(_) => t.error(),
                crate::preferences::SaveStatus::Warning(_) => t.warning(),
                _ => t.muted(),
            })
            .remove_modifier(Modifier::BOLD),
        );
    }
    let footer_height = if app.notice.is_empty() { 1 } else { 2 };
    let body = Rect::new(
        content.x,
        content.y + 2,
        content.width,
        content.height.saturating_sub(2 + footer_height),
    );
    match app.screen {
        Screen::Preview => render_gallery(frame, body, app, map),
        Screen::Settings => crate::settings::render(frame, body, app, map),
        Screen::Models => render_models(frame, body, app, map),
        Screen::Providers => render_providers(frame, body, app, map),
        Screen::ProviderPicker => render_provider_picker(frame, body, app, map),
        Screen::ProviderEdit => render_form(frame, body, app, map),
        Screen::Sessions => render_sessions(frame, body, app, map),
        Screen::Status => render_status(frame, body, app),
        Screen::Usage => crate::usage::render(frame, body, app, map),
        Screen::Stats => unreachable!("Stats owns its adaptive sheet"),
        Screen::Diff => render_diff(frame, body, app, map),
        Screen::Transcript => render_transcript(frame, body, app),
        Screen::Onboarding => render_onboarding(frame, body, app),
        Screen::Help => render_help(frame, body, app),
        Screen::Home => {}
    }
    if !app.notice.is_empty() {
        put(
            frame,
            row(content, content.height - 2),
            clip(&app.notice, content.width as usize),
            t.warning(),
        );
    }
    let hint = match app.screen {
        Screen::Models => {
            if app.ascii {
                "Up/Down model · Left/Right effort · Enter set · Esc back"
            } else {
                "↑/↓ model · ←/→ effort · Enter set · Esc back"
            }
        }
        Screen::Settings if content.width >= 68 => {
            "Arrows select · Enter applies · Left at first: categories · Esc close"
        }
        Screen::Settings => "Arrows select · Enter applies · Esc close",
        Screen::ProviderEdit => "Tab fields · Ctrl+S save locally · Esc cancel",
        Screen::Transcript => "Type to search · Tab tools · Esc/Enter close",
        Screen::Diff => "Left/Right files · PgUp/PgDn scroll · Esc/Enter close",
        Screen::Sessions => "Type to search · Up/Down select · Enter open · Esc back",
        Screen::Usage => "D sample/empty · PgUp/PgDn scroll · Esc/Enter close",
        Screen::Stats => "Tab views · R dates · D sample · Esc/Enter close",
        Screen::Onboarding => "Enter next · Left back · T theme · P providers · Esc back",
        Screen::Status | Screen::Help => "PgUp/PgDn scroll · Esc/Enter close",
        _ => "Up/Down select · Enter choose · Esc back",
    };
    put(
        frame,
        row(content, content.height - 1),
        clip(hint, content.width as usize),
        t.muted(),
    );
}

fn fill_line_backgrounds(frame: &mut Frame, area: Rect, lines: &[Line<'_>], scroll: usize) {
    let mut logical = 0;
    for line in lines {
        let rows = Paragraph::new(line.clone())
            .wrap(Wrap { trim: false })
            .line_count(area.width);
        if let Some(bg) = line.style.bg {
            for row in logical..logical + rows {
                if row < scroll || row >= scroll + area.height as usize {
                    continue;
                }
                let y = area.y + (row - scroll) as u16;
                for x in area.x..area.right() {
                    frame.buffer_mut()[(x, y)].set_bg(bg);
                }
            }
        }
        logical += rows;
    }
}
fn scroll_text(frame: &mut Frame, area: Rect, app: &App, lines: Vec<Line<'static>>) {
    let p = Paragraph::new(lines.clone()).wrap(Wrap { trim: false });
    let max = p
        .line_count(area.width)
        .saturating_sub(area.height as usize);
    fill_line_backgrounds(frame, area, &lines, app.scroll.min(max));
    frame.render_widget(
        p.scroll((app.scroll.min(max).min(u16::MAX as usize) as u16, 0)),
        area,
    );
}
fn list_mapped(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    items: Vec<(String, String)>,
    selected: usize,
    map: &mut UiMap,
    action: impl Fn(usize) -> HitAction,
) {
    let capacity = (area.height / 2).max(1) as usize;
    let start = selected.saturating_sub(capacity.saturating_sub(1));
    for (y, (i, (label, detail))) in items
        .iter()
        .enumerate()
        .skip(start)
        .take(capacity)
        .enumerate()
    {
        let chosen = i == selected;
        let style = if chosen {
            app.theme.selected()
        } else {
            app.theme.base()
        };
        let text = if area.width >= 76 {
            format!(
                "{} {:<22} {}",
                if chosen { ">" } else { " " },
                label,
                detail
            )
        } else {
            format!("{} {}  {}", if chosen { ">" } else { " " }, label, detail)
        };
        let rr = row(area, y as u16 * 2);
        let hovered = app.mouse_position.is_some_and(|p| rr.contains(p.into()));
        put(
            frame,
            rr,
            clip(&text, area.width as usize),
            style.patch(app.theme.control(hovered, chosen)),
        );
        map.add_target(rr, action(i));
    }
}

fn render_gallery(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    put(
        frame,
        row(area, 0),
        "A navigable design specimen. Every result below is simulated.",
        app.theme.muted(),
    );
    list_mapped(
        frame,
        Rect::new(
            area.x,
            area.y + 2,
            area.width,
            area.height.saturating_sub(2),
        ),
        app,
        SCENARIOS
            .iter()
            .map(|(a, b)| ((*a).into(), (*b).into()))
            .collect(),
        app.selected,
        map,
        HitAction::PopupSelect,
    );
}
fn render_models(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    let t = app.theme;
    let provider = &app.providers[app.model_provider];
    let rows = app.model_rows();
    let capacity = area.height.saturating_sub(6).max(1) as usize;
    let selected_model = app.selected.min(rows.len().saturating_sub(1));
    let start = selected_model.saturating_sub(capacity.saturating_sub(1));
    let wide = area.width >= 90;
    for (y, (i, (_, id))) in rows
        .iter()
        .enumerate()
        .skip(start)
        .take(capacity)
        .enumerate()
    {
        let current = app.active_provider == app.model_provider
            && id.is_some()
            && id.as_ref() == app.active_model.as_ref();
        let label = id
            .as_deref()
            .map(model_label)
            .unwrap_or("Configure a model");
        let text = if wide {
            format!(
                "{} {label:<32} {}",
                if i == app.selected { ">" } else { " " },
                id.as_deref().map(model_description).unwrap_or("")
            )
        } else {
            format!("{} {label}", if i == app.selected { ">" } else { " " })
        };
        let target = row(area, y as u16);
        let hovered = app
            .mouse_position
            .is_some_and(|p| target.contains(p.into()));
        let style = (if i == app.selected {
            t.selected()
        } else {
            t.base()
        })
        .patch(t.control(hovered, i == app.selected));
        put(
            frame,
            target,
            clip(&text, area.width.saturating_sub(2) as usize),
            style,
        );
        if current {
            put(
                frame,
                Rect::new(area.right().saturating_sub(1), target.y, 1, 1),
                if app.ascii { "*" } else { "✓" },
                t.selected().patch(t.control(hovered, i == app.selected)),
            );
        }
        map.add_target(target, HitAction::PopupSelect(i));
    }
    let shown = rows.len().saturating_sub(start).min(capacity) as u16;
    if !wide && shown < area.height {
        let description = rows
            .get(selected_model)
            .and_then(|(_, id)| id.as_deref())
            .map(model_description)
            .unwrap_or("");
        put(
            frame,
            row(area, shown),
            clip(&format!("  {description}"), area.width as usize),
            t.muted(),
        );
    }
    let provider_y = (shown + 1 + u16::from(!wide)).min(area.height.saturating_sub(1));
    let provider_label = clip(
        &format!("Provider: {} · Change", provider.label),
        area.width.saturating_sub(4) as usize,
    );
    let pr = Rect::new(
        area.x,
        area.y + provider_y,
        provider_label.width() as u16 + 4,
        1,
    );
    render_control(
        frame,
        pr,
        &provider_label,
        t.base(),
        app,
        app.selected == rows.len(),
    );
    map.add_target(pr, HitAction::PopupSelect(rows.len()));
    let effort_y = provider_y + 2;
    if effort_y + 1 < area.height {
        put(frame, row(area, effort_y), "Effort", t.muted());
        let mut x = area.x + 7;
        let selected = app.picker_effort();
        for (i, label) in EFFORTS.iter().enumerate() {
            let width = label.width() as u16 + 4;
            let rect = Rect::new(x, area.y + effort_y, width, 1);
            render_control(
                frame,
                rect,
                label,
                if i == selected {
                    t.selected()
                } else {
                    t.muted()
                },
                app,
                i == selected,
            );
            map.add_target(rect, HitAction::PopupEffortValue(i));
            x += width + 1;
        }
        let description = [
            "Quick responses",
            "Balanced reasoning",
            "More deliberate reasoning",
            "Extended reasoning",
            "Most deliberate reasoning",
        ][selected];
        put(
            frame,
            row(area, effort_y + 1),
            clip(
                &format!("        {description} · preview preference"),
                area.width as usize,
            ),
            t.muted(),
        );
    }
}

fn render_provider_picker(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    let items = app
        .providers
        .iter()
        .enumerate()
        .map(|(i, p)| {
            (
                p.label.clone(),
                format!("{}{}", p.state(), if i == 0 { " · default" } else { "" }),
            )
        })
        .collect();
    list_mapped(
        frame,
        area,
        app,
        items,
        app.selected,
        map,
        HitAction::PopupSelect,
    );
}
fn render_providers(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    put(
        frame,
        row(area, 0),
        "Provider registry · configuration is held in memory only",
        app.theme.muted(),
    );
    put(
        frame,
        row(area, 1),
        "No authentication, requests, or endpoint validation happen in this preview.",
        app.theme.muted(),
    );
    let mut items = app
        .providers
        .iter()
        .enumerate()
        .map(|(i, p)| {
            (
                format!("{}{}", p.label, if i == 0 { " · default" } else { "" }),
                p.state().into(),
            )
        })
        .collect::<Vec<_>>();
    items.push((
        "+ Add provider".into(),
        "Custom endpoint and model presentation".into(),
    ));
    list_mapped(
        frame,
        Rect::new(
            area.x,
            area.y + 3,
            area.width,
            area.height.saturating_sub(3),
        ),
        app,
        items,
        app.selected,
        map,
        HitAction::PopupSelect,
    );
}
fn render_form(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    let t = app.theme;
    let labels = [
        "Display label",
        "Endpoint (optional)",
        "Credential reference (env name only)",
        "Model label or identifier",
    ];
    let mut lines = vec![
        line("Local settings · no connection is made", t.muted()),
        line("", t.base()),
    ];
    let mut caret = None;
    for (i, label) in labels.iter().enumerate() {
        let chosen = app.selected == i;
        lines.push(line(
            format!("{} {label}", if chosen { ">" } else { " " }),
            if chosen { t.selected() } else { t.muted() },
        ));
        let editor = &app.form[i];
        let (wrapped, cursor_row, cursor_col) =
            wrap_editor(&editor.text, editor.cursor, area.width.saturating_sub(4));
        let value = if editor.text.is_empty() {
            "(not set)".into()
        } else if chosen {
            wrapped[cursor_row].clone()
        } else {
            clip(&editor.text, area.width.saturating_sub(4) as usize)
        };
        lines.push(line(
            format!("  {value}"),
            if chosen { t.selected() } else { t.base() },
        ));
        if chosen {
            caret = Some((lines.len() - 1, cursor_col));
        }
        lines.push(line("", t.base()));
    }
    lines.push(line(
        format!(
            "{} Save for this session only",
            if app.selected == 4 { ">" } else { " " }
        ),
        if app.selected == 4 {
            t.selected()
        } else {
            t.base()
        },
    ));
    let focus_row = caret.map_or(lines.len() - 1, |(row, _)| row);
    let offset = focus_row.saturating_sub(area.height.saturating_sub(1) as usize);
    for i in 0..labels.len() {
        for local in [2 + i * 3, 3 + i * 3] {
            if local >= offset && local - offset < area.height as usize {
                map.add_target(row(area, (local - offset) as u16), HitAction::PopupField(i));
            }
        }
    }
    let save_row = 2 + labels.len() * 3;
    if save_row >= offset && save_row - offset < area.height as usize {
        map.add_target(
            row(area, (save_row - offset) as u16),
            HitAction::PopupField(labels.len()),
        );
    }
    frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), area);
    if let Some((row, col)) = caret {
        frame.set_cursor_position((area.x + 2 + col as u16, area.y + (row - offset) as u16));
    }
}
fn search_bar(frame: &mut Frame, area: Rect, app: &App) {
    put(
        frame,
        row(area, 0),
        format!(
            "Search / {}",
            if app.query.text.is_empty() {
                "type to filter"
            } else {
                &app.query.text
            }
        ),
        app.theme.muted(),
    );
}
fn render_sessions(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    search_bar(frame, area, app);
    put(
        frame,
        row(area, 1),
        "Demo sessions · these are not records from your machine",
        app.theme.warning(),
    );
    let ids = app.filtered_sessions();
    if ids.is_empty() {
        put(
            frame,
            row(area, 4),
            "No sessions match your search.",
            app.theme.muted(),
        );
        return;
    }
    let list_area = Rect::new(
        area.x,
        area.y + 3,
        area.width,
        area.height.saturating_sub(3),
    );
    let capacity = (list_area.height / 3).max(1) as usize;
    let start = app.selected.saturating_sub(capacity - 1);
    for (y, (i, id)) in ids
        .iter()
        .enumerate()
        .skip(start)
        .take(capacity)
        .enumerate()
    {
        let session = SESSIONS[*id];
        let y = y as u16 * 3;
        let style = if i == app.selected {
            app.theme
                .base()
                .patch(app.theme.selected())
                .patch(app.theme.control(false, true))
        } else {
            app.theme.base()
        };
        let session_row = Rect::new(
            list_area.x,
            list_area.y + y,
            list_area.width,
            3.min(list_area.height.saturating_sub(y)),
        );
        map.add_target(session_row, HitAction::PopupSelect(i));
        put(
            frame,
            row(list_area, y),
            format!(
                "{} {}",
                if i == app.selected { ">" } else { " " },
                session.0
            ),
            style,
        );
        put(
            frame,
            row(list_area, y + 1),
            format!("  {} · {} · demo workspace", session.1, session.2),
            app.theme.muted(),
        );
    }
}
fn render_status(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    let warnings = app.warning_messages();
    let mut lines = vec![line(format!("Warnings ({})", warnings.len()), t.bold())];
    if warnings.is_empty() {
        lines.push(line("No configuration warnings.", t.muted()));
    }
    for warning in warnings {
        lines.push(line(format!("! {warning}"), t.warning()));
    }
    lines.extend([
        line("", t.base()),
        line(format!("Workspace      {}", app.workspace), t.base()),
        line(format!("Provider       {}", app.provider().label), t.base()),
        line(
            format!("Model          {}", model_label(app.model_label())),
            t.base(),
        ),
        line("Connection     Not connected", t.muted()),
        line(
            format!(
                "Account        {}",
                app.account_email.as_deref().unwrap_or("Not configured")
            ),
            t.muted(),
        ),
        line(
            format!("Mode           {}", app.mode_status()),
            if app.mode == 0 {
                t.auto_mode()
            } else {
                t.plan_mode()
            },
        ),
        line("", t.base()),
        line(
            "Provider connections and coding tools are simulated.",
            t.muted(),
        ),
        line(
            "Account labels are local display settings, not authentication.",
            t.muted(),
        ),
    ]);
    let settings = app.prompt_settings.summary_lines();
    if !settings.is_empty() {
        lines.push(line("", t.base()));
        lines.push(line(
            "Session settings [preview; services not connected]",
            t.goal_status(),
        ));
        lines.extend(settings.into_iter().map(|s| line(s, t.base())));
    }
    scroll_text(frame, area, app, lines);
}

pub fn diff_lines(file: usize, app: &App) -> Vec<Line<'static>> {
    let data = if file == 0 {
        vec![
            "--- a/src/validation.rs",
            "+++ b/src/validation.rs",
            "@@ -1,8 +1,13 @@",
            " use crate::Error;",
            " ",
            "+fn require_input(input: &str) -> Result<(), Error> {",
            "+    if input.trim().is_empty() {",
            "+        return Err(Error::EmptyInput);",
            "+    }",
            "+    Ok(())",
            "+}",
            "+ ",
            " pub fn handle(input: &str) -> Result<(), Error> {",
            "-    if input.trim().is_empty() {",
            "-        return Err(Error::EmptyInput);",
            "-    }",
            "+    require_input(input)?;",
            "     Ok(())",
            " }",
        ]
    } else {
        vec![
            "--- a/tests/validation.rs",
            "+++ b/tests/validation.rs",
            "@@ -0,0 +1,6 @@",
            "+#[test]",
            "+fn rejects_empty_input() {",
            "+    let result = handle(\"\");",
            "+    assert!(result.is_err());",
            "+}",
            "+// Demonstration patch; not executed by polaris.",
        ]
    };
    let mut code = CodeBlock::new("diff");
    data.into_iter()
        .map(|raw| code.row(raw, app.theme).line)
        .collect()
}

fn render_diff(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    let t = app.theme;
    put(
        frame,
        row(area, 0),
        "DEMO PATCH · 2 files · no changes have been applied",
        t.warning(),
    );
    let body = Rect::new(
        area.x,
        area.y + 2,
        area.width,
        area.height.saturating_sub(2),
    );
    let files = ["src/validation.rs (+8 -3)", "tests/validation.rs (+6 -0)"];
    let half = body.width / 2;
    for (index, file) in files.iter().enumerate() {
        let rect = Rect::new(
            body.x + half * index as u16,
            body.y,
            if index == 0 { half } else { body.width - half },
            1,
        );
        put(
            frame,
            rect,
            clip(file, rect.width as usize),
            if app.tab == index {
                t.accent()
            } else {
                t.muted()
            },
        );
        map.add_target(rect, HitAction::PopupTab(index));
    }
    scroll_text(
        frame,
        Rect::new(
            body.x,
            body.y + 2,
            body.width,
            body.height.saturating_sub(2),
        ),
        app,
        diff_lines(app.tab, app),
    );
}

fn render_transcript(frame: &mut Frame, area: Rect, app: &App) {
    search_bar(frame, area, app);
    put(
        frame,
        row(area, 1),
        format!(
            "Tools: {} · all content is from the UI preview",
            if app.expanded { "expanded" } else { "compact" }
        ),
        app.theme.muted(),
    );
    let query = app.query.text.to_lowercase();
    let mut lines = vec![];
    for entry in app.visible_entries() {
        if format!("{} {}", entry.title, entry.lines.join(" "))
            .to_lowercase()
            .contains(&query)
        {
            lines.extend(entry_lines(entry, app, app.expanded));
        }
    }
    if lines.is_empty() {
        lines.push(line(
            if query.is_empty() {
                "No conversation yet. Return and enter a task, or open /preview."
            } else {
                "No transcript entries match your search."
            },
            app.theme.muted(),
        ));
    }
    scroll_text(
        frame,
        Rect::new(
            area.x,
            area.y + 3,
            area.width,
            area.height.saturating_sub(3),
        ),
        app,
        lines,
    );
}
fn render_onboarding(frame: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    let mut lines = vec![
        line(
            format!("Step {} of 3 · preview setup", app.onboarding_step + 1),
            t.accent(),
        ),
        line("", t.base()),
    ];
    match app.onboarding_step {
        0 => {
            lines.extend(welcome_lines(app));
            lines.extend([
                line("A coding conversation, with room to think.", t.bold()),
                line(
                    format!(
                        "Appearance: {:?} · T toggles dark / terminal native",
                        app.theme.mode
                    ),
                    t.muted(),
                ),
            ]);
        }
        1 => lines.extend([
            line("NVIDIA NIM comes first.", t.bold()),
            line("", t.base()),
            line(
                "NIM is the default provider in onboarding and model selection.",
                t.base(),
            ),
            line(
                "Anthropic, OpenAI, local, and custom providers share the same screens.",
                t.base(),
            ),
            line("", t.base()),
            line(
                "Press P to edit provider presentation in memory.",
                t.accent(),
            ),
            line(
                "No endpoint is contacted and no credentials are validated.",
                t.muted(),
            ),
        ]),
        _ => lines.extend([
            line("Choose your model when you are ready.", t.bold()),
            line("", t.base()),
            line(format!("Provider: {}", app.provider().label), t.base()),
            line(format!("Model: {}", app.model_label()), t.base()),
            line("", t.base()),
            line(
                "Open /model after setup to configure a model label or identifier.",
                t.muted(),
            ),
            line(
                "Enter finishes setup. UI appearance changes save locally.",
                t.accent(),
            ),
        ]),
    }
    scroll_text(frame, area, app, lines);
}
fn render_help(frame: &mut Frame, area: Rect, app: &App) {
    search_bar(frame, area, app);
    let t = app.theme;
    let mut lines = vec![line("Commands", t.bold())];
    let query = app.query.text.to_lowercase();
    for (command, description) in COMMANDS {
        if format!("{command} {description}")
            .to_lowercase()
            .contains(&query)
        {
            lines.push(Line::from(vec![
                Span::styled(format!("{command:<14}"), t.accent()),
                Span::styled((*description).to_string(), t.base()),
            ]));
        }
    }
    let shortcuts = [
        ("Enter/Return", "Submit / select / close information popup"),
        ("Shift+Enter", "Insert newline (also Alt+Enter / Ctrl+J)"),
        ("Ctrl+V", "Paste clipboard text or image at the caret"),
        ("Ctrl+A", "Select the full draft"),
        ("Ctrl+C / Ctrl+X", "Copy / cut a selected draft range"),
        ("Ctrl+Z / Ctrl+Y", "Undo / redo draft edits"),
        (
            "Mouse drag",
            "Select composer or transcript text; release to copy",
        ),
        ("Click attachment", "Open a text or image preview"),
        ("Preview E/C/Esc", "Expand / copy text / close preview"),
        (
            "$websearch / $subagents",
            "Session toggles; bare means on, =off disables",
        ),
        (
            "$subagents-model / $rag / $links",
            "Use =[item1, item2]; keep until changed",
        ),
        (
            "/plan /goal",
            "Add planning or goal directives to a submission",
        ),
        ("?", "Open shortcuts when the draft is empty"),
        ("Tab", "Complete command / next form field"),
        ("Shift+Tab", "Previous field / toggle interaction mode"),
        (
            "Shift+Up/Down",
            "Increase / decrease active model effort in composer",
        ),
        ("Ctrl+O", "Expand / collapse all inline tool details"),
        ("Ctrl+S", "Save provider form in memory"),
        ("Ctrl+C", "Interrupt / clear / confirm exit"),
        ("Esc/Escape", "Dismiss active UI / interrupt work"),
        ("PageUp/Down", "Scroll conversation or popup details"),
        ("F2", "Open warnings and session status"),
        (
            "F3 / /agents",
            "Focus agent list; Enter views a conversation",
        ),
        (
            "D in stats",
            "Toggle labeled sample data (also Usage overview)",
        ),
        (
            "Settings / Usage",
            "Highlight Strip / Ticks / Segments; Enter applies and saves",
        ),
        ("Settings Tab", "Switch category / control focus"),
        ("Settings Left", "At first variant, return to categories"),
        (
            "Settings Enter",
            "Apply highlighted choice; arrows / clicks only highlight",
        ),
        (
            "Usage Ctrl+O",
            "Expand / collapse cache and subagent details",
        ),
        (
            "Left/Right",
            "Model effort / settings values / stats tabs / diff files",
        ),
    ];
    lines.push(line("", t.base()));
    lines.push(line("Keyboard", t.bold()));
    for (keys, description) in shortcuts {
        if format!("{keys} {description}")
            .to_lowercase()
            .contains(&query)
        {
            lines.push(line(format!("{keys:<14}{description}"), t.base()));
        }
    }
    lines.push(line("", t.base()));
    lines.push(line(
        "UI PREVIEW: no inference or tools; UI preferences save locally.",
        t.warning(),
    ));
    scroll_text(
        frame,
        Rect::new(
            area.x,
            area.y + 2,
            area.width,
            area.height.saturating_sub(2),
        ),
        app,
        lines,
    );
}

pub fn dump(app: &App, width: u16, height: u16) -> std::io::Result<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height))?;
    terminal.draw(|f| render(f, app))?;
    let buffer = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..height {
        let mut text = String::new();
        let mut x = 0;
        while x < width {
            let symbol = buffer[(x, y)].symbol();
            text.push_str(symbol);
            x += UnicodeWidthStr::width(symbol).max(1) as u16;
        }
        out.push_str(text.trim_end());
        out.push('\n');
    }
    Ok(out)
}

/// Replace decorative UI punctuation without discarding non-ASCII user text.
pub fn normalize_ascii_buffer(buffer: &mut ratatui::buffer::Buffer, ascii: bool) {
    if ascii {
        for cell in &mut buffer.content {
            if cell.symbol() == "·" {
                cell.set_symbol(".");
            }
        }
    }
}

pub fn stats_sheet(app: &App, width: u16, height: u16) -> Rect {
    content_column(popup_area(app, Rect::new(0, 0, width, height)))
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use crate::{app::Entry, composer::Composer};
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Arc;

    #[test]
    fn rich_tokens_keep_foreground_focus_and_copy_selection_uses_dark_background() {
        let mut app = App::default();
        app.input.set("@src/main.rs /help");
        let decorations = token_decorations(&app.input.text);
        let mention = rich_style(&app, 1, &decorations);
        let slash = rich_style(&app, 13, &decorations);
        assert_eq!(mention.fg, Some(Color::Rgb(249, 226, 175)));
        assert_eq!(slash.fg, Some(Color::Rgb(118, 185, 0)));
        let selected = mention.patch(app.theme.copy_selection(
            mention.fg.unwrap(),
            Color::Rgb(0, 0, 0),
            false,
        ));
        assert_eq!(selected.fg, mention.fg);
        assert_eq!(selected.bg, Some(Color::Rgb(75, 68, 53)));
        assert!(!selected.add_modifier.contains(Modifier::UNDERLINED));
        assert!(!selected.add_modifier.contains(Modifier::REVERSED));

        let text = "url https://host/path and /dir/file";
        let decorations = token_decorations(text);
        assert!(
            lexical_style(&app, text.find("/path").unwrap(), &decorations)
                .unwrap()
                .add_modifier
                .contains(Modifier::UNDERLINED)
        );
        assert!(lexical_style(&app, text.find("/dir").unwrap(), &decorations).is_none());
    }

    #[test]
    fn completion_popup_has_click_targets_and_eight_row_limit() {
        let mut app = App::default();
        app.input.set("/");
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut map = UiMap::default();
        terminal
            .draw(|frame| map = render_interactive(frame, &app))
            .unwrap();
        let rows = map
            .targets
            .iter()
            .filter(|target| matches!(target.action, HitAction::Suggestion(_)))
            .count();
        assert!(app.completion_open());
        assert!((1..=8).contains(&rows));
        assert!(
            map.targets
                .iter()
                .any(|target| matches!(target.action, HitAction::Suggestion(0)))
        );
    }

    #[test]
    fn submitted_source_map_excludes_prompt_and_preserves_whitespace() {
        let mut app = App::default();
        let mut draft = Composer::default();
        draft.set("  code\nsecond line");
        let mut entry = Entry::new(EntryKind::User, "you", &["  code", "second line"]);
        entry.draft = Some(draft.clone());
        app.entries.push(entry);
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut map = UiMap::default();
        terminal
            .draw(|frame| map = render_interactive(frame, &app))
            .unwrap();
        let region = map.region(&TextRegionId::Entry(0)).unwrap();
        assert_eq!(region.source.as_ref(), "  code\nsecond line");
        let cell = region
            .cells
            .iter()
            .find(|cell| cell.byte_offset == 2)
            .unwrap();
        assert_eq!(
            map.text_position(cell.x, cell.y),
            Some((TextRegionId::Conversation, 2))
        );
        assert_eq!(draft.copy_range(2..6), "code");
    }

    #[test]
    fn composer_wheel_scroll_keeps_source_offsets_aligned() {
        let mut app = App::default();
        let text = (0..16)
            .map(|i| format!("    code line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.input.set(&text);
        let mut terminal = Terminal::new(TestBackend::new(80, 22)).unwrap();
        let mut bottom_map = UiMap::default();
        terminal
            .draw(|frame| bottom_map = render_interactive(frame, &app))
            .unwrap();
        let bottom = bottom_map.region(&TextRegionId::Composer).unwrap();
        assert!(!bottom.cells.iter().any(|cell| cell.byte_offset == 0));
        let last = text.len() - 1;
        assert!(bottom.cells.iter().any(|cell| cell.byte_offset == last));

        app.composer_scroll = usize::MAX;
        let mut scrolled_map = UiMap::default();
        terminal
            .draw(|frame| scrolled_map = render_interactive(frame, &app))
            .unwrap();
        let scrolled = scrolled_map.region(&TextRegionId::Composer).unwrap();
        assert!(scrolled.cells.iter().any(|cell| cell.byte_offset == 0));
        assert!(!scrolled.cells.iter().any(|cell| cell.byte_offset == last));
        let cell = scrolled
            .cells
            .iter()
            .find(|cell| cell.byte_offset == 5)
            .unwrap();
        assert_eq!(
            scrolled_map.text_position(cell.x, cell.y),
            Some((TextRegionId::Composer, 5))
        );
        assert_eq!(
            scrolled_map.wheel_region(cell.x, cell.y),
            Some(TextRegionId::Composer)
        );
    }

    #[test]
    fn image_preview_uses_halfblocks_without_fake_copy_and_close_hit() {
        let mut app = App::default();
        app.preview = Some(Arc::new(crate::composer::Attachment {
            id: 7,
            label: "Image 1".into(),
            payload: AttachmentPayload::Image {
                width: 1,
                height: 2,
                rgba: vec![255, 0, 0, 255, 0, 0, 255, 255],
            },
        }));
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut map = UiMap::default();
        terminal
            .draw(|frame| map = render_interactive(frame, &app))
            .unwrap();
        assert!(
            !map.targets
                .iter()
                .any(|target| target.action == HitAction::PreviewCopy)
        );
        assert!(
            map.targets
                .iter()
                .any(|target| target.action == HitAction::PreviewClose)
        );
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .any(|cell| cell.symbol() == "▀")
        );
    }

    #[test]
    fn text_preview_maps_source_for_selection_and_copy_control() {
        let mut app = App::default();
        app.preview = Some(Arc::new(crate::composer::Attachment {
            id: 9,
            label: "pasted.txt".into(),
            payload: AttachmentPayload::Text("  first\n    code".into()),
        }));
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut map = UiMap::default();
        terminal
            .draw(|frame| map = render_interactive(frame, &app))
            .unwrap();
        let id = TextRegionId::AttachmentPreview(9);
        let region = map.region(&id).unwrap();
        assert_eq!(region.source.as_ref(), "  first\n    code");
        let cell = region
            .cells
            .iter()
            .find(|cell| cell.byte_offset == 4)
            .unwrap();
        assert_eq!(map.text_position(cell.x, cell.y), Some((id, 4)));
        assert!(
            map.targets
                .iter()
                .any(|target| target.action == HitAction::PreviewCopy)
        );
        assert_eq!(
            map.wheel_region(cell.x, cell.y),
            Some(TextRegionId::AttachmentPreview(9))
        );
    }

    #[test]
    fn assistant_code_source_map_preserves_indent_and_skips_ui_labels() {
        let mut app = App::default();
        app.entries.push(Entry::new(
            EntryKind::Assistant,
            "assistant",
            &["Before", "```rust", "    let x = 1;", "```", "After"],
        ));
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut map = UiMap::default();
        terminal
            .draw(|frame| map = render_interactive(frame, &app))
            .unwrap();
        let region = map.region(&TextRegionId::Entry(0)).unwrap();
        assert!(region.source.contains("    let x = 1;"));
        assert!(!region.source.contains("rust"));
        let offset = region.source.find("    let x").unwrap();
        let cell = region
            .cells
            .iter()
            .find(|cell| cell.byte_offset == offset)
            .unwrap();
        assert_eq!(
            map.text_position(cell.x, cell.y),
            Some((TextRegionId::Conversation, offset))
        );
    }

    #[test]
    fn draft_attachment_chip_is_clickable() {
        let mut app = App::default();
        app.input
            .insert_attachment(Arc::new(crate::composer::Attachment {
                id: 42,
                label: "image.png".into(),
                payload: AttachmentPayload::Image {
                    width: 1,
                    height: 1,
                    rgba: vec![0, 0, 0, 255],
                },
            }))
            .unwrap();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut map = UiMap::default();
        terminal
            .draw(|frame| map = render_interactive(frame, &app))
            .unwrap();
        let target = map
            .targets
            .iter()
            .find(|target| target.action == HitAction::Attachment(42))
            .unwrap();
        assert_eq!(
            map.hit(target.rect.x, target.rect.y),
            Some(&HitAction::Attachment(42))
        );
    }

    #[test]
    fn composer_displays_tabs_as_spaces_and_keeps_literal_offsets() {
        let mut app = App::default();
        app.input.set("a\tb");
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut map = UiMap::default();
        terminal
            .draw(|frame| map = render_interactive(frame, &app))
            .unwrap();
        let region = map.region(&TextRegionId::Composer).unwrap();
        let a = region
            .cells
            .iter()
            .find(|cell| cell.byte_offset == 0)
            .unwrap();
        let b = region
            .cells
            .iter()
            .find(|cell| cell.byte_offset == 2)
            .unwrap();
        assert_eq!(b.x - a.x, 4);
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(a.x + 1, a.y)].symbol(), " ");
        assert_eq!(buffer[(a.x + 2, a.y)].symbol(), " ");
        assert_eq!(buffer[(a.x + 3, a.y)].symbol(), " ");
        assert_eq!(
            map.text_position(b.x, b.y),
            Some((TextRegionId::Composer, 2))
        );
    }
}
