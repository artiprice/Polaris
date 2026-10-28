#[path = "support/selection.rs"]
mod selection;
use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
use polaris::{
    app::{App, Entry, EntryKind},
    completion::FileIndex,
    interaction::{HitAction, TextRegionId, UiMap, ViewSelection},
    runtime::{MouseController, selection_text},
    theme::{Theme, ThemeMode},
    ui,
};
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::Buffer,
    style::{Color, Modifier},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const GOLD: Color = Color::Rgb(249, 226, 175);
fn draw(app: &App) -> (UiMap, Buffer) {
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut map = UiMap::default();
    terminal
        .draw(|f| map = ui::render_interactive(f, app))
        .unwrap();
    (map, terminal.backend().buffer().clone())
}
fn at<'a>(
    map: &UiMap,
    buffer: &'a Buffer,
    id: TextRegionId,
    offset: usize,
) -> &'a ratatui::buffer::Cell {
    let cell = map
        .region(&id)
        .unwrap()
        .cells
        .iter()
        .find(|c| c.byte_offset == offset)
        .unwrap();
    &buffer[(cell.x, cell.y)]
}

#[test]
fn mentions_are_yellow_in_draft_and_sent_prompt_without_surface_changes() {
    let mut app = App::default();
    app.input.set("Read @\"src/a file.rs\" with /plan please");
    let (map, buffer) = draw(&app);
    assert_eq!(at(&map, &buffer, TextRegionId::Composer, 5).fg, GOLD);
    let command = app.input.text.find("/plan").unwrap();
    assert_eq!(
        at(&map, &buffer, TextRegionId::Composer, command).fg,
        Color::Rgb(118, 185, 0)
    );
    app.submit();
    app.working = false;
    let (map, buffer) = draw(&app);
    let mention = at(&map, &buffer, TextRegionId::Conversation, 5);
    assert_eq!(mention.fg, GOLD);
    assert_eq!(mention.bg, Color::Rgb(20, 20, 20));
    assert!(!mention.modifier.contains(Modifier::REVERSED));
}

#[test]
fn file_completion_keyboard_focus_and_real_mouse_hover_have_distinct_foreground_tones() {
    let root = std::env::current_dir()
        .unwrap()
        .join("target")
        .join(format!(
            "style-files-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    std::fs::create_dir_all(&root).unwrap();
    for path in ["alpha.rs", "beta.rs"] {
        std::fs::write(root.join(path), "").unwrap();
    }
    let mut app = App::default();
    app.file_index = Some(FileIndex::start(root.clone()));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !app.file_index.as_ref().unwrap().is_ready() {
        app.poll_index();
        assert!(Instant::now() < deadline, "file index stalled");
        std::thread::sleep(Duration::from_millis(1));
    }
    app.input.set("@");
    let (map, before) = draw(&app);
    let row0 = map
        .targets
        .iter()
        .find(|t| t.action == HitAction::Suggestion(0))
        .unwrap()
        .rect;
    let row1 = map
        .targets
        .iter()
        .find(|t| t.action == HitAction::Suggestion(1))
        .unwrap()
        .rect;
    assert_eq!(before[(row0.x, row0.y)].fg, GOLD);
    assert_eq!(before[(row1.x, row1.y)].fg, Color::Rgb(133, 133, 133));
    MouseController::default().event(
        MouseEvent {
            kind: MouseEventKind::Moved,
            column: row1.x,
            row: row1.y,
            modifiers: KeyModifiers::NONE,
        },
        &map,
        &mut app,
    );
    let (_, after) = draw(&app);
    assert_eq!(after[(row1.x, row1.y)].fg, Color::Rgb(255, 253, 230));
    assert_eq!(after[(row0.x, row0.y)].fg, GOLD);
    app.mouse_position = None;
    let (_, left) = draw(&app);
    assert_eq!(left, before, "leaving hover restores the original tones");
    for (a, b) in before.content.iter().zip(&after.content) {
        assert_eq!(a.bg, b.bg);
    }
    assert_eq!(
        app.selected, 0,
        "hover must not apply or move the keyboard selection"
    );
    for path in ["alpha.rs", "beta.rs"] {
        std::fs::remove_file(root.join(path)).unwrap();
    }
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn web_links_are_underlined_in_drafts_history_markdown_and_transcript_even_without_color() {
    let text =
        "Read https://example.com/a(b), www.nvidia.com and example.else; keep src/main.rs plain.";
    for color in [true, false] {
        let mut app = App::default();
        app.theme.color = color;
        app.input.set(text);
        let (map, buffer) = draw(&app);
        for link in ["https://", "www.", "example.else"] {
            let cell = at(
                &map,
                &buffer,
                TextRegionId::Composer,
                text.find(link).unwrap(),
            );
            assert!(cell.modifier.contains(Modifier::UNDERLINED), "{link}");
        }
        assert!(
            !at(
                &map,
                &buffer,
                TextRegionId::Composer,
                text.find("src/").unwrap()
            )
            .modifier
            .contains(Modifier::UNDERLINED)
        );
        app.submit();
        app.working = false;
        app.entries.push(Entry::new(
            EntryKind::Assistant,
            "polaris",
            &["Read **https://example.org/docs**."],
        ));
        let (map, buffer) = draw(&app);
        let source = &map.region(&TextRegionId::Conversation).unwrap().source;
        let sent = at(
            &map,
            &buffer,
            TextRegionId::Conversation,
            source.find("https://").unwrap(),
        );
        assert!(sent.modifier.contains(Modifier::UNDERLINED));
        assert_eq!(
            sent.bg,
            if color {
                Color::Rgb(20, 20, 20)
            } else {
                Color::Reset
            }
        );
        let answer = at(
            &map,
            &buffer,
            TextRegionId::Conversation,
            source.find("https://example.org").unwrap(),
        );
        assert!(
            answer
                .modifier
                .contains(Modifier::UNDERLINED | Modifier::BOLD)
        );
        app.command("/transcript");
        let (_, transcript) = draw(&app);
        let start = transcript
            .content
            .windows("https://example.org".len())
            .position(|cells| {
                cells.iter().map(|c| c.symbol()).collect::<String>() == "https://example.org"
            })
            .unwrap();
        assert!(
            transcript.content[start]
                .modifier
                .contains(Modifier::UNDERLINED)
        );
        if !color {
            assert!(
                transcript
                    .content
                    .iter()
                    .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
            );
        }
    }
}

#[test]
fn link_and_mention_selection_shades_only_selected_source_and_preserves_copy() {
    let mut app = App::default();
    let text = "漢字 @main.rs https://example.com and example.else";
    app.input.set(text);
    app.submit();
    app.working = false;
    let (_, before) = draw(&app);
    app.view_selection = Some(ViewSelection {
        region: TextRegionId::Conversation,
        anchor: 0,
        end: text.len(),
    });
    let (map, after) = draw(&app);
    assert_eq!(selection_text(&app, &map), text);
    let region = map.region(&TextRegionId::Conversation).unwrap();
    for (index, (a, b)) in before.content.iter().zip(&after.content).enumerate() {
        let width = after.area.width as usize;
        let (x, y) = ((index % width) as u16, (index / width) as u16);
        let selected = region
            .cells
            .iter()
            .any(|c| c.x == x && c.y == y && c.byte_offset < text.len());
        assert_eq!(
            b.bg,
            if selected {
                selection::background(app.theme, a)
            } else {
                a.bg
            }
        );
        assert!(!b.modifier.contains(Modifier::REVERSED));
    }
    let cell = at(
        &map,
        &after,
        TextRegionId::Conversation,
        text.find("https://").unwrap(),
    );
    assert_eq!(cell.fg, Color::Rgb(232, 232, 232));
    assert!(cell.modifier.contains(Modifier::UNDERLINED));
}

#[test]
fn mode_palette_has_rgb_ansi_and_no_color_fallbacks() {
    let mut theme = Theme::default();
    assert_eq!(theme.plan_mode().fg, Some(GOLD));
    assert_eq!(theme.auto_mode().fg, Some(Color::Rgb(245, 194, 231)));
    assert_eq!(theme.mention().bg, None);
    for mode in [ThemeMode::Dark, ThemeMode::Terminal] {
        theme.mode = mode;
        theme.rgb = false;
        assert_eq!(theme.plan_mode().fg, Some(Color::Yellow));
        assert_eq!(theme.auto_mode().fg, Some(Color::LightMagenta));
        assert_eq!(theme.mention().fg, Some(Color::Yellow));
        theme.color = false;
        assert_eq!(theme.plan_mode().fg, Some(Color::Reset));
        assert_eq!(theme.auto_mode().fg, Some(Color::Reset));
        theme.color = true;
    }
}

#[test]
fn command_hover_has_a_distinct_tone_even_on_keyboard_selection_with_portable_fallbacks() {
    for (rgb, color, mode) in [
        (true, true, ThemeMode::Dark),
        (false, true, ThemeMode::Dark),
        (true, true, ThemeMode::Terminal),
        (true, false, ThemeMode::Dark),
    ] {
        let mut app = App::default();
        app.theme.rgb = rgb;
        app.theme.color = color;
        app.theme.mode = mode;
        app.input.set("/");
        let (map, before) = draw(&app);
        let target = map
            .targets
            .iter()
            .find(|t| t.action == HitAction::Suggestion(0))
            .unwrap()
            .rect;
        app.mouse_position = Some((target.x, target.y));
        let (_, hovered) = draw(&app);
        let a = &before[(target.x, target.y)];
        let b = &hovered[(target.x, target.y)];
        if color {
            assert_ne!(a.fg, b.fg);
            assert_eq!(
                b.fg,
                if rgb && mode == ThemeMode::Dark {
                    Color::Rgb(219, 245, 176)
                } else {
                    Color::White
                }
            );
        } else {
            assert!(b.modifier.contains(Modifier::BOLD));
            assert!(!b.modifier.contains(Modifier::UNDERLINED));
        }
        for (a, b) in before.content.iter().zip(&hovered.content) {
            assert_eq!(a.bg, b.bg);
        }
        assert_eq!(app.selected, 0);
        app.mouse_position = None;
        assert_eq!(draw(&app).1, before);
    }
}
