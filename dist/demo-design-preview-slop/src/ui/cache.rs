//! One active transcript/width generation; source text is shared with hit maps.
use super::*;
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

pub(super) struct EntryLayout {
    pub entry: Entry,
    expanded: bool,
    pub lines: Vec<Line<'static>>,
    pub counts: Vec<usize>,
    pub descriptors: Vec<DisplaySourceLine>,
    pub source: Arc<str>,
    pub decorations: Vec<TokenDecoration>,
    wrapped: RefCell<HashMap<usize, Rc<Vec<crate::composer::LayoutRow>>>>,
}
impl EntryLayout {
    pub fn source_rows(&self, index: usize, width: u16) -> Rc<Vec<crate::composer::LayoutRow>> {
        self.wrapped
            .borrow_mut()
            .entry(index)
            .or_insert_with(|| {
                Rc::new(transcript_source_rows(
                    &self.descriptors[index].display,
                    width,
                ))
            })
            .clone()
    }
}
pub(super) struct Transcript {
    pub entries: Vec<Rc<EntryLayout>>,
    pub starts: Vec<usize>,
    pub source_offsets: Vec<usize>,
    pub source: Arc<str>,
    pub lines: Vec<Line<'static>>,
    pub total_rows: usize,
}
#[derive(Default)]
struct Cache {
    key: Option<(ThemeMode, bool, bool, bool, u16, Option<u64>)>,
    transcript: Option<Rc<Transcript>>,
    builds: usize,
}
thread_local! { static CACHE: RefCell<Cache> = RefCell::default(); }
pub(super) fn transcript(app: &App, width: u16) -> Rc<Transcript> {
    CACHE.with(|slot| {
        let mut cache = slot.borrow_mut();
        let key = (
            app.theme.mode,
            app.theme.color,
            app.theme.rgb,
            app.ascii,
            width,
            app.agents.viewed,
        );
        let same = cache.key == Some(key);
        let entries = app.visible_entries();
        if same
            && let Some(old) = &cache.transcript
            && old.entries.len() == entries.len()
            && old
                .entries
                .iter()
                .zip(entries)
                .enumerate()
                .all(|(i, (a, b))| a.entry == *b && a.expanded == app.tool_expanded(i))
        {
            return old.clone();
        }
        let old = same.then(|| cache.transcript.clone()).flatten();
        let mut layouts = Vec::with_capacity(entries.len());
        for (i, entry) in entries.iter().enumerate() {
            let expanded = app.tool_expanded(i);
            if let Some(cached) = old.as_ref().and_then(|t| t.entries.get(i))
                && cached.entry == *entry
                && cached.expanded == expanded
            {
                layouts.push(cached.clone());
                continue;
            }
            let lines = entry_lines(entry, app, expanded);
            let counts = lines
                .iter()
                .map(|l| {
                    Paragraph::new(l.clone())
                        .wrap(Wrap { trim: false })
                        .line_count(width)
                })
                .collect();
            let mut descriptors = display_source_lines(entry, app, expanded);
            let mut source = String::new();
            for (j, d) in descriptors.iter_mut().enumerate() {
                if j > 0 {
                    source.push('\n');
                }
                d.source_start = source.len();
                source.push_str(&d.source);
            }
            let decorations = token_decorations(&source);
            layouts.push(Rc::new(EntryLayout {
                entry: entry.clone(),
                expanded,
                lines,
                counts,
                descriptors,
                source: source.into(),
                decorations,
                wrapped: RefCell::default(),
            }));
            cache.builds += 1;
        }
        let mut source = String::new();
        let mut source_offsets = Vec::with_capacity(entries.len());
        let mut starts = Vec::with_capacity(entries.len());
        let mut total_rows = 0;
        let mut lines = vec![];
        for (i, e) in layouts.iter().enumerate() {
            if i > 0 {
                source.push_str("\n\n");
            }
            source_offsets.push(source.len());
            source.push_str(&e.source);
            starts.push(total_rows);
            total_rows += e.counts.iter().sum::<usize>();
            lines.extend(e.lines.iter().cloned());
        }
        let prepared = Rc::new(Transcript {
            entries: layouts,
            starts,
            source_offsets,
            source: source.into(),
            lines,
            total_rows,
        });
        cache.key = Some(key);
        cache.transcript = Some(prepared.clone());
        prepared
    })
}

struct Pane {
    transcript: Rc<Transcript>,
    area: Rect,
    scroll: usize,
    selection: Option<crate::interaction::ViewSelection>,
    launch: std::collections::BTreeMap<usize, Vec<u64>>,
    hover: Option<HitAction>,
    buffer: ratatui::buffer::Buffer,
    map: UiMap,
}
thread_local! { static PANE: RefCell<Option<Pane>> = const { RefCell::new(None) }; }
fn hovered(map: &UiMap, app: &App) -> Option<HitAction> {
    app.mouse_position.and_then(|(x, y)| map.hit(x, y)).cloned()
}
pub(super) fn paint(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    map: &mut UiMap,
    scroll: usize,
    prepared: &Rc<Transcript>,
) {
    PANE.with(|slot| {
        let mut pane = slot.borrow_mut();
        let hit = pane.as_ref().and_then(|p| hovered(&p.map, app));
        let reuse = pane.as_ref().is_some_and(|p| {
            Rc::ptr_eq(&p.transcript, prepared)
                && p.area == area
                && p.scroll == scroll
                && p.selection == app.view_selection
                && p.launch == app.agents.launch_entries
                && p.hover == hit
        });
        if !reuse {
            let mut mapped = UiMap::default();
            fill_line_backgrounds(frame, area, &prepared.lines, scroll);
            frame.render_widget(
                Paragraph::new(prepared.lines.clone())
                    .wrap(Wrap { trim: false })
                    .scroll((scroll.min(u16::MAX as usize) as u16, 0)),
                area,
            );
            map_entry_sources(frame, area, app, &mut mapped, scroll, prepared);
            let mut buffer = ratatui::buffer::Buffer::empty(area);
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    buffer[(x, y)] = frame.buffer_mut()[(x, y)].clone();
                }
            }
            *pane = Some(Pane {
                transcript: prepared.clone(),
                area,
                scroll,
                selection: app.view_selection.clone(),
                launch: app.agents.launch_entries.clone(),
                hover: hovered(&mapped, app),
                buffer,
                map: mapped,
            });
        }
        let p = pane.as_ref().unwrap();
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                frame.buffer_mut()[(x, y)] = p.buffer[(x, y)].clone();
            }
        }
        map.targets.extend(p.map.targets.iter().cloned());
        map.text_regions.extend(p.map.text_regions.iter().cloned());
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composer_and_hover_reuse_entries_but_content_width_theme_and_expansion_invalidate() {
        let mut app = App::default();
        for _ in 0..1000 {
            app.entries.push(Entry::new(
                EntryKind::Assistant,
                "",
                &["text with example.com and Unicode 界"],
            ));
        }
        let first = transcript(&app, 116);
        app.input.set("draft");
        app.input.cursor = 2;
        app.mouse_position = Some((3, 4));
        assert!(Rc::ptr_eq(&first, &transcript(&app, 116)));
        app.entries
            .push(Entry::new(EntryKind::Tool, "Read", &["body"]));
        let appended = transcript(&app, 116);
        assert!(Rc::ptr_eq(&first.entries[999], &appended.entries[999]));
        app.entries[0].lines[0] = "changed".into();
        let changed = transcript(&app, 116);
        assert!(!Rc::ptr_eq(&appended.entries[0], &changed.entries[0]));
        assert!(Rc::ptr_eq(&appended.entries[999], &changed.entries[999]));
        app.toggle_tool_details(1000);
        let expanded = transcript(&app, 116);
        assert!(!Rc::ptr_eq(&changed.entries[1000], &expanded.entries[1000]));
        assert!(!Rc::ptr_eq(
            &expanded.entries[0],
            &transcript(&app, 75).entries[0]
        ));
        app.theme.color = false;
        assert!(!Rc::ptr_eq(
            &changed.entries[0],
            &transcript(&app, 116).entries[0]
        ));
    }
}

struct EditorLayout {
    text: String,
    width: usize,
    rows: Rc<Vec<crate::composer::LayoutRow>>,
}
thread_local! { static EDITOR: RefCell<Option<EditorLayout>> = const { RefCell::new(None) }; }
pub(super) fn editor(text: &str, width: usize) -> Rc<Vec<crate::composer::LayoutRow>> {
    EDITOR.with(|slot| {
        let mut cached = slot.borrow_mut();
        if let Some(c) = cached.as_ref()
            && c.width == width
            && c.text == text
        {
            return c.rows.clone();
        }
        let rows = Rc::new(crate::composer::layout_rows(text, width));
        *cached = Some(EditorLayout {
            text: text.into(),
            width,
            rows: rows.clone(),
        });
        rows
    })
}

#[cfg(test)]
mod editor_tests {
    use super::*;
    #[test]
    fn editor_size_and_painter_share_wrapping_and_only_new_text_or_width_rebuilds() {
        let a = editor("first 界\nsecond\tline", 70);
        assert!(Rc::ptr_eq(&a, &editor("first 界\nsecond\tline", 70)));
        assert!(!Rc::ptr_eq(&a, &editor("first 界\nsecond\tline", 40)));
        assert!(!Rc::ptr_eq(&a, &editor("changed", 70)));
    }
}
