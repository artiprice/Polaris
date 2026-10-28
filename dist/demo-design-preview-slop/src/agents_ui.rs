//! Terminal presentation for agent activity; shares the application's theme and geometry.
use crate::{
    agents::{AgentAction, AgentId, AgentStatus},
    app::App,
    interaction::{HitAction, UiMap},
    theme::Theme,
    ui,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
fn clip(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.into();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for g in s.graphemes(true) {
        let n = g.width();
        if used + n >= width {
            break;
        }
        out.push_str(g);
        used += n;
    }
    out.push('~');
    out
}
fn pad(s: &str, width: usize) -> String {
    let s = clip(s, width);
    format!("{}{}", s, " ".repeat(width.saturating_sub(s.width())))
}
fn status_ink(t: Theme, status: AgentStatus) -> Style {
    match status {
        AgentStatus::Working => t.selected(),
        AgentStatus::Waiting | AgentStatus::Paused => t.goal_status(),
        AgentStatus::Failed => t.error(),
        _ => t.muted(),
    }
}
fn mark(app: &App, status: AgentStatus) -> &'static str {
    if app.ascii {
        return match status {
            AgentStatus::Working => "*",
            AgentStatus::Completed => "+",
            AgentStatus::Failed => "!",
            AgentStatus::Queued => "o",
            _ => "-",
        };
    }
    match status {
        AgentStatus::Working => "◯",
        AgentStatus::Completed => "✓",
        AgentStatus::Failed => "!",
        AgentStatus::Queued => "○",
        AgentStatus::Paused => "Ⅱ",
        AgentStatus::Waiting => "◌",
        AgentStatus::Cancelled => "−",
    }
}
/// Number of child rows; Main is pinned separately. Short approvals retain a summary only.
pub fn child_rows(app: &App, height: u16) -> usize {
    if !app.agents.expanded || (height < 24 && app.approval) {
        return 0;
    }
    let cap = if height >= 32 {
        4
    } else if height >= 24 {
        2
    } else {
        1
    };
    app.agents.records.len().min(cap)
}
pub fn dock_height(app: &App, height: u16) -> u16 {
    if !app.agents.visible() {
        return 0;
    }
    if !app.agents.expanded || (height < 24 && app.approval) {
        return 2;
    }
    let children = child_rows(app, height);
    3 + children as u16 + u16::from(app.agents.records.len() > children)
}
pub fn scroll_offset(app: &App, height: u16) -> usize {
    let n = child_rows(app, height);
    let max = app.agents.records.len().saturating_sub(n);
    let mut offset = app.agents.offset.min(max);
    if app.agents.focused
        && let Some(i) = app
            .agents
            .selected
            .and_then(|id| app.agents.records.iter().position(|a| a.id == id))
    {
        if i < offset {
            offset = i;
        } else if i >= offset + n {
            offset = (i + 1).saturating_sub(n).min(max);
        }
    }
    offset
}
pub fn wheel(app: &mut App, up: bool, height: u16) {
    let max = app
        .agents
        .records
        .len()
        .saturating_sub(child_rows(app, height));
    app.agents.offset = if up {
        app.agents.offset.saturating_sub(1)
    } else {
        (app.agents.offset + 1).min(max)
    };
    // Wheel navigation scrolls the roster without taking editor focus or selecting a thread.
    app.agents.focused = false;
}
fn put(frame: &mut Frame, r: Rect, line: impl Into<Line<'static>>, style: Style) {
    if r.width > 0 && r.height > 0 {
        frame.render_widget(Paragraph::new(line.into()).style(style), r);
    }
}
pub fn banner(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    let Some(agent) = app.agents.viewed.and_then(|id| app.agents.get(id)) else {
        return;
    };
    if area.height == 0 {
        return;
    }
    let button = Rect::new(
        area.right().saturating_sub(10),
        area.y,
        10.min(area.width),
        1,
    );
    ui::render_control(frame, button, "Main", app.theme.selected(), app, false);
    map.add_target(button, HitAction::Agent(AgentAction::Open(None)));
    let label = format!(
        "Viewing {}{} / prompt to Main",
        agent.name,
        if app.agents.demo { " [demo]" } else { "" }
    );
    put(
        frame,
        Rect::new(area.x, area.y, area.width.saturating_sub(11), 1),
        clip(&label, area.width.saturating_sub(11) as usize),
        app.theme.muted(),
    );
    if area.height > 1 {
        let elapsed = agent
            .elapsed()
            .map(|s| crate::work_summary::elapsed_label(std::time::Duration::from_secs(s)))
            .unwrap_or_else(|| "time unavailable".into());
        let tokens = agent
            .tokens
            .map(|n| format!("{n} tokens"))
            .unwrap_or_else(|| "tokens unavailable".into());
        let source = if app.agents.demo { " [demo]" } else { "" };
        let metadata = format!(
            "{}{source} / {elapsed} / {tokens} / {} / {}",
            agent.status.label(),
            agent.model.as_deref().unwrap_or("model unavailable"),
            agent.effort.as_deref().unwrap_or("effort unavailable")
        );
        put(
            frame,
            Rect::new(area.x, area.y + 1, area.width, 1),
            clip(&metadata, area.width as usize),
            status_ink(app.theme, agent.status),
        );
    }
}
fn roster_row(frame: &mut Frame, r: Rect, app: &App, id: Option<AgentId>, map: &mut UiMap) {
    let agent = id.and_then(|id| app.agents.get(id));
    let status = agent.map_or(
        if app.working {
            AgentStatus::Working
        } else {
            AgentStatus::Completed
        },
        |a| a.status,
    );
    let viewed = app.agents.viewed == id;
    let focused = app.agents.focused && app.agents.selected == id;
    let hovered = app.mouse_position.is_some_and(|p| r.contains(p.into()));
    let overlay = app.theme.control(hovered, focused);
    let base = app.theme.base().patch(overlay);
    let name = agent.map_or("Main", |a| a.name.as_str());
    let task = agent.map_or(
        if app.working {
            "Coordinating implementation"
        } else {
            "Main conversation"
        },
        |a| a.task.as_str(),
    );
    let symbol = if id.is_none() {
        if app.ascii { "*" } else { "●" }
    } else {
        mark(app, status)
    };
    let name_width = if r.width >= 92 { 16 } else { 12 };
    let elapsed = agent
        .and_then(|a| a.elapsed())
        .or_else(|| (id.is_none() && app.working).then(|| app.working_elapsed().as_secs()));
    let time = if r.width >= 72 {
        elapsed
            .map(|s| crate::work_summary::elapsed_label(std::time::Duration::from_secs(s)))
            .unwrap_or_else(|| "-".into())
    } else {
        String::new()
    };
    let tokens = if r.width >= 92 {
        agent
            .and_then(|a| a.tokens)
            .map(|n| {
                if n >= 1000 {
                    format!("{}.{:01}k tok", n / 1000, n % 1000 / 100)
                } else {
                    format!("{n} tok")
                }
            })
            .unwrap_or_else(|| "- tok".into())
    } else {
        String::new()
    };
    let state = if id.is_none() && !app.working {
        if app.entries.is_empty() {
            "Idle"
        } else {
            "Main"
        }
    } else {
        status.label()
    };
    let metadata = format!(
        "{}{}{}",
        pad(state, 10),
        if time.is_empty() {
            String::new()
        } else {
            format!(" {}", pad(&time, 8))
        },
        if tokens.is_empty() {
            String::new()
        } else {
            format!(" {}", pad(&tokens, 11))
        }
    );
    let task_width = usize::from(r.width).saturating_sub(name_width + metadata.width() + 5);
    let line = Line::from(vec![
        Span::styled(
            if viewed { "> " } else { "  " },
            if viewed {
                app.theme.selected().patch(overlay)
            } else {
                base
            },
        ),
        Span::styled(
            format!("{symbol} "),
            status_ink(app.theme, status).patch(overlay),
        ),
        Span::styled(
            pad(name, name_width),
            if viewed {
                app.theme.selected().patch(overlay)
            } else {
                base
            },
        ),
        Span::styled(pad(task, task_width), base),
        Span::styled(
            format!(" {metadata}"),
            status_ink(app.theme, status).patch(overlay),
        ),
    ]);
    put(frame, r, " ".repeat(r.width as usize), base);
    put(frame, r, line, base);
    map.add_target(r, HitAction::Agent(AgentAction::Open(id)));
}
pub fn render_dock(frame: &mut Frame, area: Rect, app: &App, map: &mut UiMap) {
    if area.height == 0 || area.width < 8 {
        return;
    }
    map.agent_rect = Some(area);
    put(
        frame,
        Rect::new(area.x, area.y, area.width, 1),
        if app.ascii { "-" } else { "─" }.repeat(area.width as usize),
        app.theme.rule(),
    );
    let inner = Rect::new(
        area.x + 2,
        area.y + 1,
        area.width.saturating_sub(4),
        area.height.saturating_sub(1),
    );
    let expanded = area.height > 2;
    let label = if expanded { "collapse" } else { "expand" };
    let button_width = label.width() as u16 + 4;
    let button = Rect::new(
        inner.right().saturating_sub(button_width),
        inner.y,
        button_width,
        1,
    );
    let title = format!(
        "Agents{} / {}",
        if app.agents.demo { " [demo]" } else { "" },
        app.agents.summary()
    );
    put(
        frame,
        Rect::new(
            inner.x,
            inner.y,
            inner.width.saturating_sub(button_width + 1),
            1,
        ),
        clip(
            &title,
            inner.width.saturating_sub(button_width + 1) as usize,
        ),
        if app
            .agents
            .records
            .iter()
            .any(|a| a.status == AgentStatus::Failed)
        {
            app.theme.error()
        } else {
            app.theme.muted()
        },
    );
    ui::render_control(frame, button, label, app.theme.muted(), app, false);
    map.add_target(button, HitAction::Agent(AgentAction::Toggle));
    if !expanded {
        return;
    }
    roster_row(
        frame,
        Rect::new(inner.x, inner.y + 1, inner.width, 1),
        app,
        None,
        map,
    );
    let rows = child_rows(app, frame.area().height);
    let offset = scroll_offset(app, frame.area().height);
    for (n, agent) in app
        .agents
        .records
        .iter()
        .skip(offset)
        .take(rows)
        .enumerate()
    {
        roster_row(
            frame,
            Rect::new(inner.x, inner.y + 2 + n as u16, inner.width, 1),
            app,
            Some(agent.id),
            map,
        );
    }
    if app.agents.records.len() > rows {
        put(
            frame,
            Rect::new(inner.x, area.bottom() - 1, inner.width, 1),
            format!(
                "{}-{} / {} agents / F3 focus / Enter view",
                offset + 1,
                (offset + rows).min(app.agents.records.len()),
                app.agents.records.len()
            ),
            app.theme.muted(),
        );
    }
}
