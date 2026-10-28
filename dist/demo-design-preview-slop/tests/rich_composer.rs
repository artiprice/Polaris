#[path = "support/selection.rs"]
mod selection;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use polaris::{
    app::{App, ClipboardAction, ClipboardPurpose, Screen},
    clipboard::{ClipboardOperationResult, ClipboardPayload, ClipboardResult},
    composer::{AttachmentPayload, Composer},
    interaction::{HitAction, TextRegionId, UiMap},
    runtime::MouseController,
    ui,
};
use ratatui::{Terminal, backend::TestBackend};
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn draw(app: &App) -> UiMap {
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut map = UiMap::default();
    terminal
        .draw(|f| map = ui::render_interactive(f, app))
        .unwrap();
    map
}
fn paste_result(token: u64, text: &str) -> ClipboardResult {
    ClipboardResult {
        token,
        result: Ok(ClipboardOperationResult::Paste(ClipboardPayload::Text(
            text.into(),
        ))),
    }
}
#[test]
fn attachment_lifetime_numbering_undo_submission() {
    let mut app = App::default();
    app.paste("small\r\npaste");
    assert_eq!(app.input.text, "small\npaste");
    assert!(app.input.spans.is_empty());
    app.input.clear();
    app.paste(&"x".repeat(501));
    app.paste_image(1, 1, vec![255, 0, 0, 255]);
    assert_eq!(app.input.text, "[Pasted Content 501 chars][Image 1]");
    app.input.backspace();
    assert_eq!(app.input.text, "[Pasted Content 501 chars]");
    app.input.undo();
    assert_eq!(app.input.spans.len(), 2);
    app.submit();
    assert_eq!(app.entries[0].draft.as_ref().unwrap().spans.len(), 2);
    assert_eq!(app.submissions[0].attachment_ids.len(), 2);
    let id = app.entries[0].draft.as_ref().unwrap().spans[0]
        .attachment
        .id;
    app.open_attachment(id);
    assert!(matches!(
        app.preview.as_ref().unwrap().payload,
        AttachmentPayload::Text(_)
    ));
    app.key(key(KeyCode::Esc));
    assert!(app.preview.is_none());
    app.working = false;
    app.paste(&"z".repeat(501));
    assert_eq!(app.input.text, "[Pasted Content 501 chars]");
}
#[test]
fn directives_apply_on_send_and_ui_commands_still_open() {
    let mut app = App::default();
    app.input.set("/plan /goal improve @\"src/a file.rs\"");
    assert_eq!(app.mode_label(), "auto");
    app.submit();
    assert_eq!(app.mode_label(), "plan");
    let request = &app.submissions[0];
    assert!(request.planning && request.goal);
    assert_eq!(request.files, ["src/a file.rs"]);
    app.working = false;
    app.input.set("/status");
    app.submit();
    assert_eq!(app.screen, Screen::Status);
    app.key(key(KeyCode::Enter));
    assert_eq!(app.screen, Screen::Home);
    let mut draft = Composer::default();
    draft.set(&format!(
        "Read https://x/plan /src/goal and {} /plan {}",
        char::from(96),
        char::from(96)
    ));
    let request = polaris::app::parse_submission(&draft);
    assert!(!request.planning && !request.goal);
}
#[test]
fn async_paste_keeps_original_position_and_cancels_after_clear() {
    let mut app = App::default();
    app.input.set("first ");
    app.request_paste();
    let ClipboardAction::Paste(token) = app.clipboard_actions.pop().unwrap() else {
        panic!()
    };
    app.input.insert("later");
    app.clipboard_result(paste_result(token, "PASTED "));
    assert_eq!(app.input.text, "first PASTED later");
    assert_eq!(app.input.cursor, app.input.text.len());
    app.request_paste();
    let ClipboardAction::Paste(token) = app.clipboard_actions.pop().unwrap() else {
        panic!()
    };
    app.input.clear();
    app.clipboard_result(paste_result(token, "must not appear"));
    assert_eq!(app.input.text, "");
    assert!(app.toast_text().unwrap().contains("canceled"));
}
#[test]
fn pending_paste_survives_popup_navigation() {
    let mut app = App::default();
    app.input.set("draft ");
    app.request_paste();
    let ClipboardAction::Paste(token) = app.clipboard_actions.pop().unwrap() else {
        panic!()
    };
    app.command("/model");
    app.clipboard_result(paste_result(token, "text"));
    assert_eq!(app.input.text, "draft text");
    assert_eq!(app.screen, Screen::Models);
}
#[test]
fn inline_completion_does_not_prematurely_submit() {
    let mut app = App::default();
    app.input.set("Please /pl");
    assert!(app.completion_open());
    app.key(key(KeyCode::Enter));
    assert_eq!(app.input.text, "Please /plan ");
    assert!(!app.working);
    app.key(key(KeyCode::Enter));
    assert!(app.working && app.submissions[0].planning);
}
#[test]
fn mouse_copy_is_clean_and_confirmation_requires_clipboard_success() {
    let mut app = App::default();
    app.input.set("hello world");
    let map = draw(&app);
    let region = map.region(&TextRegionId::Composer).unwrap();
    let from = region.cells.iter().find(|c| c.byte_offset == 0).unwrap();
    let end = region.cells.iter().find(|c| c.byte_offset == 11).unwrap();
    let mut mouse = MouseController::default();
    for (kind, x, y) in [
        (MouseEventKind::Down(MouseButton::Left), from.x, from.y),
        (MouseEventKind::Drag(MouseButton::Left), end.x, end.y),
        (MouseEventKind::Up(MouseButton::Left), end.x, end.y),
    ] {
        mouse.event(
            MouseEvent {
                kind,
                column: x,
                row: y,
                modifiers: KeyModifiers::NONE,
            },
            &map,
            &mut app,
        );
    }
    assert!(app.toast_text().is_none());
    let ClipboardAction::Copy(token, text) = app.clipboard_actions.pop().unwrap() else {
        panic!()
    };
    assert_eq!(text, "hello world");
    assert!(matches!(
        app.clipboard_pending.get(&token),
        Some(ClipboardPurpose::Copy(2))
    ));
    app.clipboard_result(ClipboardResult {
        token,
        result: Ok(ClipboardOperationResult::Copy),
    });
    assert_eq!(app.toast_text(), Some("Copied 2 words"));
    app.toast.as_mut().unwrap().1 = std::time::Instant::now() - std::time::Duration::from_millis(1);
    assert!(app.toast_text().is_none());
}
#[test]
fn clicking_marker_opens_preview_then_expansion_is_undoable() {
    let mut app = App::default();
    app.paste(&"x".repeat(501));
    let map = draw(&app);
    let target = map
        .targets
        .iter()
        .find(|t| matches!(t.action, HitAction::Attachment(_)))
        .unwrap();
    assert!(matches!(
        map.hit(target.rect.x, target.rect.y),
        Some(HitAction::Attachment(_))
    ));
    let mut mouse = MouseController::default();
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        mouse.event(
            MouseEvent {
                kind,
                column: target.rect.x,
                row: target.rect.y,
                modifiers: KeyModifiers::NONE,
            },
            &map,
            &mut app,
        );
    }
    assert!(app.preview.is_some());
    app.expand_preview();
    assert_eq!(app.input.text, "x".repeat(501));
    assert!(app.preview.is_none());
    app.input.undo();
    assert_eq!(app.input.text, "[Pasted Content 501 chars]");
}
#[test]
fn clipboard_failure_never_claims_success() {
    let mut app = App::default();
    app.request_copy("a b".into());
    let ClipboardAction::Copy(token, _) = app.clipboard_actions.pop().unwrap() else {
        panic!()
    };
    app.clipboard_result(ClipboardResult {
        token,
        result: Err("busy".into()),
    });
    assert!(app.toast_text().unwrap().contains("busy"));
    assert!(!app.toast_text().unwrap().contains("Copied"));
}

#[test]
fn selected_code_copy_preserves_real_whitespace_without_decorations() {
    let mut app = App::default();
    let fence = char::from(96).to_string().repeat(3);
    app.entries.push(polaris::app::Entry::new(
        polaris::app::EntryKind::Assistant,
        "polaris",
        &[
            "Here is code:",
            &format!("{fence}rust"),
            "    let text = \"hello\";",
            "",
            "    finish();",
            &fence,
        ],
    ));
    let map = draw(&app);
    let id = TextRegionId::Entry(0);
    let source = map
        .region(&id)
        .expect("assistant text must be selectable")
        .source
        .clone();
    assert!(source.contains("    let text = \"hello\";\n\n    finish();"));
    assert!(!source.contains("code / rust"));
    assert!(!source.contains("◆"));
    app.view_selection = Some(polaris::interaction::ViewSelection {
        region: id,
        anchor: 0,
        end: source.len(),
    });
    assert_eq!(
        polaris::runtime::selection_text(&app, &map),
        source.as_ref()
    );
}
#[test]
fn tabs_survive_edit_and_copy_and_undo_cancels_pending_paste() {
    let mut app = App::default();
    app.paste("\tlet x = 1;");
    assert_eq!(app.input.text, "\tlet x = 1;");
    app.input.anchor = Some(0);
    assert_eq!(app.input.selected_text(), "\tlet x = 1;");
    app.input.anchor = None;
    app.input.insert("x");
    app.request_paste();
    let ClipboardAction::Paste(token) = app.clipboard_actions.pop().unwrap() else {
        panic!()
    };
    app.input.undo();
    app.clipboard_result(paste_result(token, "unwanted"));
    assert!(!app.input.text.contains("unwanted"));
    assert!(app.toast_text().unwrap().contains("canceled"));
}
#[test]
fn worker_image_result_drives_attachment_and_preview() {
    use polaris::clipboard::{ClipboardBackend, ClipboardImage, ClipboardService};
    struct Fake;
    impl ClipboardBackend for Fake {
        fn get_image(&mut self) -> Result<Option<ClipboardImage>, String> {
            Ok(Some(ClipboardImage {
                width: 2,
                height: 2,
                rgba: vec![255; 16],
            }))
        }
        fn get_text(&mut self) -> Result<Option<String>, String> {
            panic!("image must take priority")
        }
        fn set_text(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }
    let service = ClipboardService::with_backend(Fake).unwrap();
    let mut app = App::default();
    app.key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
    let ClipboardAction::Paste(token) = app.clipboard_actions.pop().unwrap() else {
        panic!()
    };
    service.send_paste(token).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Some(result) = service.try_recv() {
            app.clipboard_result(result);
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(app.input.text, "[Image 1]");
    assert_eq!(app.input.spans.len(), 1);
    app.open_attachment(app.input.spans[0].attachment.id);
    assert!(draw(&app).preview_rect.is_some());
}

#[test]
fn paste_from_native_control_records_preserves_crlf_and_tabs() {
    use polaris::input::KeyboardDecoder;
    let mut decoder = KeyboardDecoder::default();
    for ch in "\x1b[200~a".chars() {
        decoder.feed(key(KeyCode::Char(ch)));
    }
    decoder.feed(key(KeyCode::Enter));
    decoder.feed(key(KeyCode::Char('\n')));
    decoder.feed(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
    decoder.feed(key(KeyCode::Char('b')));
    for ch in "\x1b[201~".chars() {
        decoder.feed(key(KeyCode::Char(ch)));
    }
    let payload = decoder.take_pastes();
    assert_eq!(payload, ["a\r\n\tb"]);
    let mut app = App::default();
    app.paste(&payload[0]);
    assert_eq!(app.input.text, "a\n\tb");
    assert!(!app.working);
}
#[test]
fn new_composer_selection_shades_only_selected_text_and_respects_no_color() {
    use ratatui::style::{Color, Modifier};
    for mode in [
        polaris::theme::ThemeMode::Dark,
        polaris::theme::ThemeMode::Terminal,
    ] {
        for color in [true, false] {
            let mut app = App::default();
            app.theme.mode = mode;
            app.theme.color = color;
            app.input.set("@\"src/a file.rs\" /plan text");
            let capture = |app: &App| {
                let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
                terminal.draw(|f| ui::render(f, app)).unwrap();
                terminal.backend().buffer().clone()
            };
            let before = capture(&app);
            app.input.anchor = Some(0);
            let after = capture(&app);
            for (a, b) in before.content.iter().zip(&after.content) {
                assert!(a.bg == b.bg || (color && b.bg == selection::background(app.theme, a)));
                assert!(!b.modifier.contains(Modifier::REVERSED));
                if !color {
                    assert_eq!(a.fg, Color::Reset);
                    assert_eq!(b.fg, Color::Reset);
                }
            }
            let selected = &after[(6, 20)];
            assert_eq!(
                selected.bg,
                selection::background(app.theme, &before[(6, 20)])
            );
            assert!(!selected.modifier.contains(Modifier::UNDERLINED));
            if color {
                assert_eq!(selected.fg, before[(6, 20)].fg);
            } else {
                assert!(selected.modifier.contains(Modifier::BOLD));
            }
        }
    }
}
