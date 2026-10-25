use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use polaris_ui_cli::input::{self, NormalizedInput};
use std::error::Error;
type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn normalizes_supported_crossterm_events() -> TestResult {
    let event = Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SHIFT));
    assert!(matches!(
        input::normalize_event(event)?,
        NormalizedInput::Key(_)
    ));
    assert!(matches!(
        input::normalize_event(Event::Resize(80, 24))?,
        NormalizedInput::Resize {
            columns: 80,
            rows: 24
        }
    ));
    assert!(matches!(
        input::normalize_event(Event::FocusGained)?,
        NormalizedInput::Focus { focused: true }
    ));
    Ok(())
}

#[test]
fn rejects_paste_over_one_mibibyte() -> TestResult {
    assert!(input::normalize_event(Event::Paste("x".repeat(1024 * 1024))).is_ok());
    assert!(input::normalize_event(Event::Paste("x".repeat(1024 * 1024 + 1))).is_err());
    Ok(())
}

#[test]
fn only_pressed_escape_and_control_c_are_cancel_priority() -> TestResult {
    let escape = NormalizedInput::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(escape.is_cancel_priority());
    let control_c = NormalizedInput::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(control_c.is_cancel_priority());
    let mut release = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    assert!(!NormalizedInput::Key(release).is_cancel_priority());
    Ok(())
}
