use crate::{
    app::{App, Screen},
    composer::AttachmentPayload,
    interaction::{HitAction, TextRegionId, UiMap, ViewSelection},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::{
    ops::Range,
    time::{Duration, Instant},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(450);
#[derive(Default)]
pub struct MouseController {
    drag: Option<Drag>,
    last_click: Option<Click>,
}
struct Click {
    region: TextRegionId,
    x: u16,
    y: u16,
    at: Instant,
    word: Range<usize>,
    source_len: usize,
}
struct Drag {
    region: Option<TextRegionId>,
    anchor: usize,
    x: u16,
    y: u16,
    moved: bool,
    action: Option<HitAction>,
    started: Instant,
    word_anchor: Option<Range<usize>>,
    clicked_word: Option<Range<usize>>,
    source_len: usize,
}
impl MouseController {
    pub fn event(&mut self, event: MouseEvent, map: &UiMap, app: &mut App) {
        self.event_at(event, map, app, Instant::now());
    }
    fn event_at(&mut self, event: MouseEvent, map: &UiMap, app: &mut App, now: Instant) {
        let (x, y) = (event.column, event.row);
        app.mouse_position = Some((x, y));
        if app.screen == Screen::Stats && matches!(event.kind, MouseEventKind::Moved) {
            crate::stats::hover(app, map.hit(x, y));
        }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let pos = map.text_position(x, y).or_else(|| {
                    let region = map.region(&TextRegionId::Conversation)?;
                    if map.hit(x, y).is_some()
                        || x < region.rect.x
                        || x > region.rect.right()
                        || y < region.rect.y
                        || y >= region.rect.bottom()
                    {
                        return None;
                    }
                    region
                        .cells
                        .iter()
                        .min_by_key(|cell| (cell.y.abs_diff(y), cell.x.abs_diff(x)))
                        .map(|cell| (TextRegionId::Conversation, cell.byte_offset))
                });
                let (region, anchor) = pos.map_or((None, 0), |(r, p)| (Some(r), p));
                app.view_selection = None;
                if region == Some(TextRegionId::Composer) {
                    app.input.cursor = app.input.snap(anchor);
                    app.input.anchor = None;
                } else if region.is_some() {
                    app.input.anchor = None;
                }
                let action = map.hit(x, y).cloned();
                let source_len = region
                    .as_ref()
                    .and_then(|id| map.region(id))
                    .map_or(0, |r| r.source.len());
                let clicked_word = region.as_ref().and_then(|id| {
                    if !matches!(action, None | Some(HitAction::Composer)) {
                        return None;
                    }
                    let r = map.region(id)?;
                    // End-insertion cells and blank padding must not create words.
                    let on_glyph = r.cells.iter().any(|c| {
                        c.y == y
                            && c.byte_offset == anchor
                            && x >= c.x
                            && r.source
                                .get(anchor..)
                                .and_then(|s| s.graphemes(true).next())
                                .is_some_and(|g| {
                                    g != "\n" && x < c.x + UnicodeWidthStr::width(g).max(1) as u16
                                })
                    });
                    if on_glyph {
                        word_at(app, map, id, anchor)
                    } else {
                        None
                    }
                });
                let word_anchor = self
                    .last_click
                    .take()
                    .filter(|click| {
                        region.as_ref() == Some(&click.region)
                            && click.source_len == source_len
                            && click.x.abs_diff(x) <= 1
                            && click.y == y
                            && clicked_word.as_ref() == Some(&click.word)
                            && now
                                .checked_duration_since(click.at)
                                .is_some_and(|elapsed| elapsed <= DOUBLE_CLICK_INTERVAL)
                    })
                    .and(clicked_word.clone());
                if let (Some(id), Some(word)) = (&region, &word_anchor) {
                    set_selection(app, id, word.start, word.end);
                }
                self.drag = Some(Drag {
                    region,
                    anchor,
                    x,
                    y,
                    moved: false,
                    action,
                    started: now,
                    word_anchor,
                    clicked_word,
                    source_len,
                });
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(d) = &mut self.drag {
                    d.moved |= x != d.x || y != d.y;
                    if let Some(id) = &d.region
                        && let Some(region) = map.region(id)
                    {
                        let cx = x.clamp(region.rect.x, region.rect.right());
                        let cy = y.clamp(region.rect.y, region.rect.bottom().saturating_sub(1));
                        let end = region.position(cx, cy).or_else(|| {
                            // Keep a continuous selection through blank message separators.
                            region
                                .cells
                                .iter()
                                .min_by_key(|cell| (cell.y.abs_diff(cy), cell.x.abs_diff(cx)))
                                .map(|cell| cell.byte_offset)
                        });
                        if let Some(end) = end {
                            let (anchor, end) = if let Some(word) = &d.word_anchor {
                                let current = word_at(app, map, id, end).unwrap_or(end..end);
                                if current.start < word.start {
                                    (word.end, current.start)
                                } else {
                                    (word.start, current.end.max(word.end))
                                }
                            } else {
                                (d.anchor, end)
                            };
                            set_selection(app, id, anchor, end);
                        }
                        if y < region.rect.y {
                            match id {
                                TextRegionId::Composer => {
                                    app.input
                                        .key_at_width(
                                            KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT),
                                            app.editor_width,
                                        )
                                        .ok();
                                    if let Some(word) = &d.word_anchor {
                                        let end = word_at(app, map, id, app.input.cursor)
                                            .map_or(app.input.cursor, |r| r.start);
                                        set_selection(app, id, word.end, end);
                                    }
                                }
                                TextRegionId::Entry(_) | TextRegionId::Conversation => {
                                    app.scroll = app.scroll.saturating_add(1)
                                }
                                _ => app.preview_scroll = app.preview_scroll.saturating_sub(1),
                            }
                        } else if y >= region.rect.bottom() {
                            match id {
                                TextRegionId::Composer => {
                                    app.input
                                        .key_at_width(
                                            KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
                                            app.editor_width,
                                        )
                                        .ok();
                                    if let Some(word) = &d.word_anchor {
                                        let end = word_at(app, map, id, app.input.cursor)
                                            .map_or(app.input.cursor, |r| r.end);
                                        set_selection(app, id, word.start, end);
                                    }
                                }
                                TextRegionId::Entry(_) | TextRegionId::Conversation => {
                                    app.scroll = app.scroll.saturating_sub(1)
                                }
                                _ => app.preview_scroll = app.preview_scroll.saturating_add(1),
                            }
                        }
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(d) = self.drag.take() {
                    if d.moved || d.word_anchor.is_some() {
                        self.last_click = None;
                        app.request_copy(selection_text(app, map));
                    } else {
                        self.last_click = match (d.region, d.clicked_word) {
                            (Some(region), Some(word)) => Some(Click {
                                region,
                                x: d.x,
                                y: d.y,
                                at: d.started,
                                word,
                                source_len: d.source_len,
                            }),
                            _ => None,
                        };
                        if let Some(action) = d.action
                            && map.hit(x, y) == Some(&action)
                        {
                            activate(action, app);
                        }
                    }
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                self.last_click = None;
                let up = event.kind == MouseEventKind::ScrollUp;
                if app.preview.is_some() {
                    app.preview_scroll = if up {
                        app.preview_scroll.saturating_sub(3)
                    } else {
                        app.preview_scroll.saturating_add(3)
                    };
                } else if app.screen == Screen::Home
                    && map.agent_rect.is_some_and(|r| r.contains((x, y).into()))
                {
                    crate::agents_ui::wheel(app, up, map.agent_rect.unwrap().bottom() + 2);
                } else if app.screen == Screen::Settings {
                    app.settings.reveal_selection = false;
                    app.scroll = if up {
                        app.scroll.saturating_sub(3)
                    } else {
                        app.scroll.saturating_add(3)
                    };
                } else if matches!(
                    app.screen,
                    Screen::Models
                        | Screen::Providers
                        | Screen::ProviderPicker
                        | Screen::Sessions
                        | Screen::Preview
                        | Screen::ProviderEdit
                ) {
                    app.key(KeyEvent::new(
                        if up { KeyCode::Up } else { KeyCode::Down },
                        KeyModifiers::NONE,
                    ));
                } else if app.screen != Screen::Home {
                    app.scroll = if up {
                        app.scroll.saturating_sub(3)
                    } else {
                        app.scroll.saturating_add(3)
                    };
                } else if app.completion_open()
                    && map
                        .completion_rect
                        .is_some_and(|rect| rect.contains((x, y).into()))
                {
                    let len = app.suggestions().len();
                    if len > 0 {
                        let selected = app.selected.min(len - 1);
                        app.selected = if up {
                            selected.saturating_sub(1)
                        } else {
                            selected.saturating_add(1).min(len - 1)
                        };
                    }
                } else if map.wheel_region(x, y) == Some(TextRegionId::Composer) {
                    app.composer_scroll = if up {
                        app.composer_scroll.saturating_add(3)
                    } else {
                        app.composer_scroll.saturating_sub(3)
                    };
                } else {
                    app.scroll = if up {
                        app.scroll.saturating_add(3)
                    } else {
                        app.scroll.saturating_sub(3)
                    };
                }
            }
            _ => {}
        }
    }
}
fn set_selection(app: &mut App, id: &TextRegionId, anchor: usize, end: usize) {
    if *id == TextRegionId::Composer {
        app.input.anchor = Some(app.input.snap(anchor));
        app.input.cursor = app.input.snap(end);
    } else {
        app.view_selection = Some(ViewSelection {
            region: id.clone(),
            anchor,
            end,
        });
    }
}
fn word_at(app: &App, map: &UiMap, id: &TextRegionId, byte: usize) -> Option<Range<usize>> {
    let region = map.region(id)?;
    if *id == TextRegionId::Composer {
        if let Some(span) = app.input.spans.iter().find(|s| s.range.contains(&byte)) {
            return Some(span.range.clone());
        }
    } else if let Some((range, _)) = region
        .replacements
        .iter()
        .find(|(range, _)| range.contains(&byte))
    {
        return Some(range.clone());
    } else if *id == TextRegionId::Conversation {
        let mut start = 0;
        for (i, entry) in app.visible_entries().iter().enumerate() {
            let source = map.region(&TextRegionId::Entry(i))?;
            if let Some(draft) = &entry.draft {
                for span in &draft.spans {
                    let range = (start + span.range.start)..(start + span.range.end);
                    if range.contains(&byte) {
                        return Some(range);
                    }
                }
            }
            start += source.source.len() + 2;
        }
    } else if let TextRegionId::Entry(i) = id
        && let Some(span) = app
            .visible_entries()
            .get(*i)
            .and_then(|e| e.draft.as_ref())
            .and_then(|d| d.spans.iter().find(|s| s.range.contains(&byte)))
    {
        return Some(span.range.clone());
    }
    word_range(&region.source, byte)
}
fn word_range(text: &str, byte: usize) -> Option<Range<usize>> {
    text.split_word_bound_indices()
        .find(|(start, part)| *start <= byte && byte < *start + part.len())
        .map(|(start, part)| start..start + part.len())
}

fn activate(action: HitAction, app: &mut App) {
    match action {
        HitAction::Composer => app.agents.focused = false,
        HitAction::Agent(action) => crate::agents::activate(app, action),
        HitAction::Usage(action) => crate::usage::activate(app, action),
        HitAction::Stats(action) => crate::stats::activate(app, action),
        HitAction::Settings(action) => crate::settings::activate(app, action),
        HitAction::ToLatest => app.scroll = 0,
        HitAction::ToggleTool(index) => app.toggle_tool_details(index),
        HitAction::Suggestion(i) => app.accept_completion(i),
        HitAction::Attachment(id) => app.open_attachment(id),
        HitAction::PreviewClose => {
            app.preview = None;
            app.view_selection = None;
        }
        HitAction::PreviewExpand => app.expand_preview(),
        HitAction::PreviewCopy => {
            if let Some(a) = &app.preview
                && let AttachmentPayload::Text(s) = &a.payload
            {
                app.request_copy(s.clone());
            }
        }
        HitAction::Approval(i) => {
            app.approval_selected = i;
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
        HitAction::FooterHelp => app.command("/help"),
        HitAction::FooterModel => app.command("/model"),
        HitAction::FooterMode => app.mode = (app.mode + 1) % 2,
        HitAction::PopupSelect(i) => {
            app.selected = i;
            app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
        HitAction::PopupField(i) => app.selected = i,
        HitAction::PopupTab(i) => app.tab = i,
        HitAction::PopupEffortValue(value) => {
            let current = app.picker_effort();
            for _ in 0..current.abs_diff(value) {
                app.key(KeyEvent::new(
                    if value > current {
                        KeyCode::Right
                    } else {
                        KeyCode::Left
                    },
                    KeyModifiers::NONE,
                ));
            }
        }
        HitAction::PopupEffort(increase) => app.key(KeyEvent::new(
            if increase {
                KeyCode::Right
            } else {
                KeyCode::Left
            },
            KeyModifiers::NONE,
        )),
    }
}

pub fn selection_text(app: &App, map: &UiMap) -> String {
    if app.input.selection().is_some() && app.view_selection.is_none() {
        return app.input.selected_text();
    }
    if let Some(s) = &app.view_selection {
        let range = s.anchor.min(s.end)..s.anchor.max(s.end);
        if let TextRegionId::Entry(index) = s.region
            && let Some(draft) = app
                .visible_entries()
                .get(index)
                .and_then(|e| e.draft.as_ref())
        {
            return draft.copy_range(range);
        }
        return map
            .region(&s.region)
            .map(|r| r.copy_range(range))
            .unwrap_or_default();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::ClipboardAction, interaction::TextRegion};
    use ratatui::layout::Rect;
    fn fixture(text: &str) -> (App, UiMap) {
        let mut app = App::default();
        app.input.set(text);
        let mut region = TextRegion::new(TextRegionId::Composer, Rect::new(2, 2, 70, 5), text);
        for (y, row) in crate::composer::layout_rows(text, 70).iter().enumerate() {
            for &(x, byte) in &row.cells {
                region.push_cell(x as u16 + 2, y as u16 + 2, byte);
            }
            let end = row.cells.last().map_or(0, |(x, byte)| {
                x + UnicodeWidthStr::width(text[*byte..].graphemes(true).next().unwrap_or(""))
            });
            region.push_cell(end as u16 + 2, y as u16 + 2, row.end);
        }
        let mut map = UiMap::default();
        map.add_target(region.rect, HitAction::Composer);
        map.add_text_region(region);
        (app, map)
    }
    fn event(
        c: &mut MouseController,
        app: &mut App,
        map: &UiMap,
        kind: MouseEventKind,
        byte: usize,
        at: Instant,
    ) {
        let p = map
            .region(&TextRegionId::Composer)
            .unwrap()
            .cells
            .iter()
            .find(|c| c.byte_offset == byte)
            .unwrap();
        c.event_at(
            MouseEvent {
                kind,
                column: p.x,
                row: p.y,
                modifiers: KeyModifiers::NONE,
            },
            map,
            app,
            at,
        );
    }
    fn double_down(c: &mut MouseController, app: &mut App, map: &UiMap, byte: usize, at: Instant) {
        event(
            c,
            app,
            map,
            MouseEventKind::Down(MouseButton::Left),
            byte,
            at,
        );
        event(
            c,
            app,
            map,
            MouseEventKind::Up(MouseButton::Left),
            byte,
            at + Duration::from_millis(10),
        );
        event(
            c,
            app,
            map,
            MouseEventKind::Down(MouseButton::Left),
            byte,
            at + Duration::from_millis(120),
        );
    }
    #[test]
    fn unicode_word_ranges_keep_combining_marks_emoji_punctuation_and_spaces_intact() {
        let text = "naïve café é 👩‍💻 alpha,beta  end";
        for (needle, offset, want) in [
            ("café", 2, "café"),
            ("é", 1, "é"),
            ("👩‍💻", 0, "👩‍💻"),
            (",", 0, ","),
            ("  end", 0, "  "),
        ] {
            let range = word_range(text, text.find(needle).unwrap() + offset).unwrap();
            assert_eq!(&text[range], want);
        }
        assert!(word_range(text, text.len()).is_none());
        assert!(word_range("", 0).is_none());
    }
    #[test]
    fn double_click_hold_selects_whole_words_reverses_and_copies_on_release() {
        let (mut app, map) = fixture("alpha beta gamma delta");
        let mut c = MouseController::default();
        let at = Instant::now();
        double_down(&mut c, &mut app, &map, 8, at);
        assert_eq!(app.input.selected_text(), "beta");
        for (byte, want) in [
            (2, "alpha beta"),
            (13, "beta gamma"),
            (8, "beta"),
            (22, "beta gamma delta"),
        ] {
            event(
                &mut c,
                &mut app,
                &map,
                MouseEventKind::Drag(MouseButton::Left),
                byte,
                at + Duration::from_millis(200),
            );
            assert_eq!(app.input.selected_text(), want);
        }
        event(
            &mut c,
            &mut app,
            &map,
            MouseEventKind::Up(MouseButton::Left),
            22,
            at + Duration::from_millis(250),
        );
        assert!(
            matches!(app.clipboard_actions.last(),Some(ClipboardAction::Copy(_,text)) if text=="beta gamma delta")
        );
    }
    #[test]
    fn double_click_without_drag_copies_and_slow_or_distant_clicks_stay_single() {
        let (mut app, map) = fixture("alpha beta gamma");
        let mut c = MouseController::default();
        let at = Instant::now();
        double_down(&mut c, &mut app, &map, 1, at);
        event(
            &mut c,
            &mut app,
            &map,
            MouseEventKind::Up(MouseButton::Left),
            1,
            at + Duration::from_millis(150),
        );
        assert!(
            matches!(app.clipboard_actions.last(),Some(ClipboardAction::Copy(_,text)) if text=="alpha")
        );
        event(
            &mut c,
            &mut app,
            &map,
            MouseEventKind::Down(MouseButton::Left),
            1,
            at + Duration::from_millis(200),
        );
        assert!(app.input.selection().is_none());
        event(
            &mut c,
            &mut app,
            &map,
            MouseEventKind::Up(MouseButton::Left),
            1,
            at + Duration::from_millis(210),
        );
        event(
            &mut c,
            &mut app,
            &map,
            MouseEventKind::Down(MouseButton::Left),
            1,
            at + Duration::from_millis(700),
        );
        assert!(app.input.selection().is_none());
        event(
            &mut c,
            &mut app,
            &map,
            MouseEventKind::Up(MouseButton::Left),
            1,
            at + Duration::from_millis(710),
        );
        event(
            &mut c,
            &mut app,
            &map,
            MouseEventKind::Down(MouseButton::Left),
            12,
            at + Duration::from_millis(730),
        );
        assert!(app.input.selection().is_none());
    }
    #[test]
    fn word_drag_keeps_attachment_chips_atomic_and_cancels_double_click_after_scroll() {
        let (mut app, _) = fixture("before ");
        app.paste(&"x".repeat(501));
        let (_, mut map) = fixture(&app.input.text);
        let mut c = MouseController::default();
        let at = Instant::now();
        double_down(&mut c, &mut app, &map, 2, at);
        event(
            &mut c,
            &mut app,
            &map,
            MouseEventKind::Drag(MouseButton::Left),
            10,
            at + Duration::from_millis(180),
        );
        assert_eq!(
            app.input.selected_text(),
            format!("before {}", "x".repeat(501))
        );
        let (mut app, new_map) = fixture("alpha beta");
        map = new_map;
        c = MouseController::default();
        event(
            &mut c,
            &mut app,
            &map,
            MouseEventKind::Down(MouseButton::Left),
            1,
            at,
        );
        event(
            &mut c,
            &mut app,
            &map,
            MouseEventKind::Up(MouseButton::Left),
            1,
            at + Duration::from_millis(10),
        );
        c.event_at(
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 3,
                row: 2,
                modifiers: KeyModifiers::NONE,
            },
            &map,
            &mut app,
            at + Duration::from_millis(20),
        );
        event(
            &mut c,
            &mut app,
            &map,
            MouseEventKind::Down(MouseButton::Left),
            1,
            at + Duration::from_millis(30),
        );
        assert!(app.input.selection().is_none());
    }
}
