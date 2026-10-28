use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use polaris::{
    app::{App, Screen},
    input::{KeyboardDecoder, normalize_key},
};
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn sequence(text: &str) -> Vec<KeyEvent> {
    let mut decoder = KeyboardDecoder::default();
    text.chars()
        .flat_map(|ch| decoder.feed(key(KeyCode::Char(ch))))
        .collect()
}

#[test]
fn native_and_encoded_shift_tab_forms_toggle_mode_without_typing() {
    for event in [
        key(KeyCode::BackTab),
        KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::Char('\t'), KeyModifiers::SHIFT),
        sequence("\x1b[Z")[0],
        sequence("\x1b[1;2Z")[0],
        sequence("\x1b[9;2u")[0],
    ] {
        let mut app = App::default();
        app.input.set("draft");
        app.key(event);
        assert_eq!(app.mode_label(), "plan");
        app.key(event);
        assert_eq!(app.mode_label(), "auto");
        assert_eq!(app.input.text, "draft");
    }
}

#[test]
fn held_shift_restores_enter_and_tab_when_console_records_lost_the_modifier() {
    for code in [KeyCode::Enter, KeyCode::Char('\r')] {
        let mut app = App::default();
        app.input.set("first");
        app.key(normalize_key(key(code), true));
        assert_eq!(app.input.text, "first\n");
        assert!(!app.working);
        app.paste("second");
        app.key(normalize_key(key(code), false));
        assert!(app.working);
        assert_eq!(app.entries[0].lines, ["first", "second"]);
    }
    for code in [KeyCode::Tab, KeyCode::Char('\t')] {
        let mut app = App::default();
        app.key(normalize_key(key(code), true));
        assert_eq!(app.mode_label(), "plan");
    }
    assert_eq!(
        normalize_key(key(KeyCode::Char('x')), true),
        key(KeyCode::Char('x'))
    );
}

#[test]
fn encoded_shift_enter_and_escape_control_characters_reach_the_app() {
    let mut app = App::default();
    app.input.set("first");
    for event in sequence("\x1b[13;2u") {
        app.key(event);
    }
    assert_eq!(app.input.text, "first\n");
    assert!(!app.working);
    app.command("/status");
    for event in sequence("\x1b[27u") {
        app.key(event);
    }
    assert_eq!(app.screen, Screen::Home);
}

#[test]
fn escape_and_return_close_information_popups_and_restore_drafts() {
    for command in ["/status", "/help", "/usage", "/stats", "/diff"] {
        for code in [
            KeyCode::Esc,
            KeyCode::Char('\x1b'),
            KeyCode::Enter,
            KeyCode::Char('\r'),
        ] {
            let mut app = App::default();
            app.input.set("keep this draft");
            app.command(command);
            let release = KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Release);
            app.key(release);
            assert_ne!(
                app.screen,
                Screen::Home,
                "key release must not close {command}"
            );
            app.key(key(code));
            assert_eq!(app.screen, Screen::Home, "{command} {code:?}");
            assert_eq!(app.input.text, "keep this draft");
        }
    }
}

#[test]
fn model_return_selects_and_escape_backs_out_of_nested_provider_picker() {
    let mut app = App::default();
    app.command("/model");
    app.key(key(KeyCode::Down));
    let model = app.model_rows()[app.selected].1.clone();
    app.key(key(KeyCode::Char('\r')));
    assert_eq!(app.screen, Screen::Home);
    assert_eq!(app.active_model, model);
    app.command("/model");
    app.key(key(KeyCode::Char('p')));
    assert_eq!(app.screen, Screen::ProviderPicker);
    app.key(key(KeyCode::Esc));
    assert_eq!(app.screen, Screen::Models);
    app.key(key(KeyCode::Esc));
    assert_eq!(app.screen, Screen::Home);
}

#[test]
fn demo_toggle_has_its_own_key_so_return_can_close_usage_and_stats() {
    for command in ["/usage", "/stats"] {
        let mut app = App::default();
        app.command(command);
        app.key(key(KeyCode::Char('d')));
        assert!(if command == "/stats" {
            app.stats.demo
        } else {
            app.metrics_demo
        });
        if command == "/stats" {
            assert!(!app.metrics_demo);
        }
        assert_ne!(app.screen, Screen::Home);
        app.key(key(KeyCode::Char('d')));
        assert!(!app.metrics_demo && !app.stats.demo);
        app.key(key(KeyCode::Enter));
        assert_eq!(app.screen, Screen::Home);
    }
}

#[test]
fn shift_arrows_adjust_effort_immediately_without_editing_or_opening_a_popup() {
    for (up, down) in [
        (
            KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
        ),
        (sequence("\x1b[1;2A")[0], sequence("\x1b[1;2B")[0]),
        (sequence("\x1b[57352;2u")[0], sequence("\x1b[57353;2u")[0]),
    ] {
        let mut app = App::default();
        app.input.set("one\ntwo");
        app.input.anchor = Some(1);
        app.composer_scroll = 2;
        let draft = app.input.clone();
        app.key(up);
        assert_eq!(app.effort_label(), "xhigh");
        app.key(up);
        assert_eq!(app.screen, Screen::Home);
        assert_eq!(app.effort_label(), "max");
        assert_eq!(app.input, draft);
        assert_eq!(app.composer_scroll, 2);
        assert!(
            polaris::ui::dump(&app, 80, 24)
                .unwrap()
                .lines()
                .last()
                .unwrap()
                .ends_with("[ max ]")
        );
        app.key(down);
        assert_eq!(app.effort_label(), "xhigh");
        app.key(down);
        assert_eq!(app.effort_label(), "high");
        let repeat =
            KeyEvent::new_with_kind(KeyCode::Down, KeyModifiers::SHIFT, KeyEventKind::Repeat);
        for _ in 0..8 {
            app.key(repeat);
        }
        assert_eq!(app.effort_label(), "low");
        for _ in 0..8 {
            app.key(up);
        }
        assert_eq!(app.effort_label(), "max");
        assert_eq!(app.input, draft);
        assert!(app.notice.is_empty());
        assert!(app.submissions.is_empty());
    }
}

#[test]
fn direct_effort_is_shared_with_picker_and_remembered_by_provider_and_model() {
    fn choose(app: &mut App, provider: usize, model: &str) {
        app.command("/model");
        app.model_provider = provider;
        app.selected = app
            .model_rows()
            .iter()
            .position(|(_, m)| m.as_deref() == Some(model))
            .unwrap();
        app.key(key(KeyCode::Enter));
        assert_eq!(app.screen, Screen::Home);
    }
    let mut app = App::default();
    let first = app.active_model.clone().unwrap();
    app.input.set("/");
    app.selected = 2; // Completion focus must not select a different model's preference.
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    assert_eq!(app.selected, 2);
    assert_eq!(app.input.text, "/");
    app.command("/model");
    app.selected = app
        .model_rows()
        .iter()
        .position(|(_, m)| m.as_ref() == Some(&first))
        .unwrap();
    assert_eq!(app.picker_effort(), 3);
    let second = app
        .model_rows()
        .iter()
        .find_map(|(_, m)| m.as_ref().filter(|m| **m != first).cloned())
        .unwrap();
    app.key(key(KeyCode::Esc));
    choose(&mut app, 0, &second);
    assert_eq!(app.effort_label(), "high");
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    assert_eq!(app.effort_label(), "medium");
    choose(&mut app, 0, &first);
    assert_eq!(app.effort_label(), "xhigh");
    app.providers[1].models.push(first.clone());
    choose(&mut app, 1, &first);
    assert_eq!(app.effort_label(), "high");
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    choose(&mut app, 0, &first);
    assert_eq!(app.effort_label(), "xhigh");
    choose(&mut app, 0, &second);
    assert_eq!(app.effort_label(), "medium");
}

#[test]
fn ordinary_arrows_extra_modifiers_and_releases_do_not_adjust_effort() {
    let mut app = App::default();
    app.input.set("one\ntwo");
    let end = app.input.cursor;
    app.key(key(KeyCode::Up));
    assert!(app.input.cursor < end);
    assert_eq!(app.effort_label(), "high");
    app.key(key(KeyCode::Down));
    assert_eq!(app.input.cursor, end);
    for event in [
        KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::Down, KeyModifiers::ALT | KeyModifiers::SHIFT),
        KeyEvent::new_with_kind(KeyCode::Up, KeyModifiers::SHIFT, KeyEventKind::Release),
    ] {
        app.key(event);
        assert_eq!(app.effort_label(), "high");
    }
    app.working = true;
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    assert!(app.working);
    assert_eq!(app.effort_label(), "medium");
}

#[test]
fn modal_popups_previews_and_approval_keep_their_arrow_navigation() {
    let shift_down = KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT);
    let mut app = App::default();
    app.command("/model");
    app.key(shift_down);
    assert_eq!(app.selected, 1);
    assert_eq!(app.effort_label(), "high");
    app.key(key(KeyCode::Esc));
    app.paste_image(1, 1, vec![255; 4]);
    app.open_attachment(app.input.spans[0].attachment.id);
    app.key(shift_down);
    assert_eq!(app.preview_scroll, 1);
    assert_eq!(app.effort_label(), "high");
    app.key(key(KeyCode::Esc));
    app.approval = true;
    app.key(shift_down);
    assert_eq!(app.effort_label(), "high");
}

#[test]
fn direct_effort_handles_missing_models_and_clamps_invalid_indices() {
    let mut app = App::default();
    app.active_model = None;
    app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT));
    assert_eq!(app.effort_label(), "high");
    assert_eq!(app.screen, Screen::Home);
    let mut app = App::default();
    app.effort = usize::MAX;
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT));
    assert_eq!(app.effort_label(), "xhigh");
}

#[test]
fn escape_interrupts_work_after_dismissing_active_ui_and_preserves_the_draft() {
    use polaris::{
        app::EntryKind,
        interaction::{TextRegionId, ViewSelection},
    };
    for context in [
        "home",
        "popup",
        "menu",
        "preview",
        "draft-selection",
        "transcript-selection",
    ] {
        let mut app = App::default();
        app.input.set("Start response");
        app.submit();
        app.input.set("Unsent draft");
        match context {
            "popup" => app.command("/usage"),
            "menu" => app.input.set("/"),
            "preview" => {
                app.paste(&"x".repeat(501));
                let id = app.input.spans[0].attachment.id;
                app.open_attachment(id);
            }
            "draft-selection" => app.input.anchor = Some(0),
            "transcript-selection" => {
                app.view_selection = Some(ViewSelection {
                    region: TextRegionId::Conversation,
                    anchor: 0,
                    end: 3,
                })
            }
            _ => {}
        }
        let draft = app.input.text.clone();
        app.key(KeyEvent {
            kind: KeyEventKind::Release,
            ..key(KeyCode::Esc)
        });
        assert!(app.working, "release must not interrupt");
        app.key(key(KeyCode::Esc));
        if context != "home" {
            assert!(app.working, "first Escape should dismiss {context}");
            app.key(key(KeyCode::Esc));
        }
        assert!(!app.working, "Escape did not interrupt in {context}");
        assert!(app.working_since.is_none());
        assert_eq!(app.input.text, draft);
        assert!(!app.quit);
        let count = app
            .entries
            .iter()
            .filter(|e| e.title == "Interrupted [demo]")
            .count();
        assert_eq!(count, 1);
        app.tick();
        app.key(key(KeyCode::Esc));
        assert_eq!(
            app.entries
                .iter()
                .filter(|e| e.title == "Interrupted [demo]")
                .count(),
            1
        );
        assert!(
            !app.entries.iter().any(|e| e.kind == EntryKind::Completion),
            "interrupt must not claim completed work"
        );
    }
}
