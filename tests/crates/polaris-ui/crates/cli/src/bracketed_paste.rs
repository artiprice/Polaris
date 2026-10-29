// Unit tests of crates/polaris-ui/crates/cli/src/bracketed_paste.rs (`tests`); compiled into that module via #[path].
use super::BracketedPasteDecoder;
use crate::input::{InputError, NormalizedInput};
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

fn key(code: KeyCode) -> NormalizedInput {
    NormalizedInput::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn send(decoder: &mut BracketedPasteDecoder, text: &str) -> Result<(), InputError> {
    for character in text.chars() {
        let code = if character == '\u{1b}' {
            KeyCode::Esc
        } else {
            KeyCode::Char(character)
        };
        decoder.push(key(code))?;
    }
    Ok(())
}

fn send_with_key_releases(
    decoder: &mut BracketedPasteDecoder,
    text: &str,
) -> Result<(), InputError> {
    for character in text.chars() {
        let code = if character == '\u{1b}' {
            KeyCode::Esc
        } else {
            KeyCode::Char(character)
        };
        decoder.push(NormalizedInput::Key(KeyEvent::new_with_kind(
            code,
            KeyModifiers::NONE,
            KeyEventKind::Press,
        )))?;
        decoder.push(NormalizedInput::Key(KeyEvent::new_with_kind(
            code,
            KeyModifiers::NONE,
            KeyEventKind::Release,
        )))?;
    }
    Ok(())
}

#[test]
fn native_release_records_do_not_break_csi_u_or_bracketed_paste() -> Result<(), InputError> {
    let mut decoder = BracketedPasteDecoder::default();
    send_with_key_releases(&mut decoder, "\u{1b}[13;2u")?;
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT,
        )))
    );

    let mut paste = BracketedPasteDecoder::default();
    send_with_key_releases(&mut paste, "\u{1b}[200~middle\u{1b}[201~")?;
    assert_eq!(
        paste.pop(),
        Some(NormalizedInput::Paste("middle".to_owned()))
    );
    Ok(())
}

#[test]
fn bracketed_payload_is_one_literal_paste_including_escape_sequences() -> Result<(), InputError> {
    let mut decoder = BracketedPasteDecoder::default();
    send(&mut decoder, "\u{1b}[200~hello\u{1b}[31m\u{1b}[201~")?;
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Paste("hello\u{1b}[31m".to_owned()))
    );
    assert_eq!(decoder.pop(), None);
    Ok(())
}

#[test]
fn bracketed_payload_preserves_leading_dollar_sigils() -> Result<(), InputError> {
    let mut decoder = BracketedPasteDecoder::default();
    send(&mut decoder, "\u{1b}[200~$websearch(on)\u{1b}[201~")?;
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Paste("$websearch(on)".to_owned()))
    );
    Ok(())
}

#[test]
fn arrow_and_function_key_escape_sequences_are_decoded() {
    let mut decoder = BracketedPasteDecoder::default();
    for code in [KeyCode::Esc, KeyCode::Char('['), KeyCode::Char('A')] {
        let result = decoder.push(key(code));
        assert!(result.is_ok());
    }
    assert_eq!(decoder.pop(), Some(key(KeyCode::Up)));

    let mut decoder = BracketedPasteDecoder::default();
    assert!(send(&mut decoder, "\u{1b}[15~").is_ok());
    assert_eq!(decoder.pop(), Some(key(KeyCode::F(5))));
}

#[test]
fn csi_u_enter_and_modified_arrows_preserve_modifiers() {
    let mut decoder = BracketedPasteDecoder::default();
    for character in "\u{1b}[13;2u".chars() {
        let event = if character == '\u{1b}' {
            key(KeyCode::Esc)
        } else {
            key(KeyCode::Char(character))
        };
        assert!(decoder.push(event).is_ok());
    }
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT
        )))
    );

    let mut decoder = BracketedPasteDecoder::default();
    for character in "\u{1b}[1;2A".chars() {
        let event = if character == '\u{1b}' {
            key(KeyCode::Esc)
        } else {
            key(KeyCode::Char(character))
        };
        assert!(decoder.push(event).is_ok());
    }
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Key(KeyEvent::new(
            KeyCode::Up,
            KeyModifiers::SHIFT
        )))
    );
}

#[test]
fn csi_u_scalar_modifiers_phases_and_focus_are_preserved() -> Result<(), InputError> {
    let mut decoder = BracketedPasteDecoder::default();
    send(&mut decoder, "\u{1b}[36;2u\u{1b}[122;5u\u{1b}[122;1:3u")?;
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Key(KeyEvent::new(
            KeyCode::Char('$'),
            KeyModifiers::SHIFT,
        )))
    );
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Key(KeyEvent::new(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL,
        )))
    );
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Key(KeyEvent::new_with_kind(
            KeyCode::Char('z'),
            KeyModifiers::NONE,
            crossterm::event::KeyEventKind::Release,
        )))
    );

    let mut decoder = BracketedPasteDecoder::default();
    send(
        &mut decoder,
        "\u{1b}[2;2~\u{1b}[9;2u\u{1b}[Z\u{1b}[I\u{1b}[O",
    )?;
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Key(KeyEvent::new(
            KeyCode::Insert,
            KeyModifiers::SHIFT,
        )))
    );
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        )))
    );
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::SHIFT,
        )))
    );
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Focus { focused: true })
    );
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Focus { focused: false })
    );
    Ok(())
}

#[test]
fn unknown_complete_csi_is_discarded_and_paste_keeps_escape_bytes() -> Result<(), InputError> {
    let mut decoder = BracketedPasteDecoder::default();
    send(&mut decoder, "\u{1b}[999zq")?;
    assert_eq!(decoder.pop(), Some(key(KeyCode::Char('q'))));
    assert_eq!(decoder.pop(), None);

    let mut decoder = BracketedPasteDecoder::default();
    send(
        &mut decoder,
        "\u{1b}[200~\u{1b}[999z\u{1b}[36;2u\u{1b}[201~",
    )?;
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Paste("\u{1b}[999z\u{1b}[36;2u".to_owned()))
    );
    Ok(())
}

#[test]
fn csi_u_control_enter_and_sgr_hover_are_events_not_text() {
    let mut decoder = BracketedPasteDecoder::default();
    assert!(send(&mut decoder, "\u{1b}[13;5u").is_ok());
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL
        )))
    );

    let mut decoder = BracketedPasteDecoder::default();
    assert!(send(&mut decoder, "\u{1b}[<35;40;24M").is_ok());
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 39,
            row: 23,
            modifiers: KeyModifiers::NONE,
        }))
    );
}

#[test]
fn csi_u_control_j_remains_distinct_from_control_enter() {
    let mut decoder = BracketedPasteDecoder::default();
    assert!(send(&mut decoder, "\u{1b}[106;5u").is_ok());
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Key(KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::CONTROL
        )))
    );
}

#[test]
fn sgr_motion_with_pressed_buttons_is_drag() {
    for (button, expected) in [
        (32, MouseButton::Left),
        (33, MouseButton::Middle),
        (34, MouseButton::Right),
    ] {
        let mut decoder = BracketedPasteDecoder::default();
        let sequence = format!("\u{1b}[<{button};40;24M");
        assert!(send(&mut decoder, &sequence).is_ok());
        assert_eq!(
            decoder.pop(),
            Some(NormalizedInput::Mouse(MouseEvent {
                kind: MouseEventKind::Drag(expected),
                column: 39,
                row: 23,
                modifiers: KeyModifiers::NONE,
            }))
        );
    }

    let mut decoder = BracketedPasteDecoder::default();
    assert!(send(&mut decoder, "\u{1b}[<35;40;24M").is_ok());
    assert_eq!(
        decoder.pop(),
        Some(NormalizedInput::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 39,
            row: 23,
            modifiers: KeyModifiers::NONE,
        }))
    );
}

#[test]
fn oversized_native_paste_is_rejected_and_decoder_recovers() -> Result<(), InputError> {
    let mut decoder = BracketedPasteDecoder::default();
    send(&mut decoder, "\u{1b}[200~")?;
    for _ in 0..crate::input::MAX_PASTE_BYTES {
        decoder.push(key(KeyCode::Char('x')))?;
    }
    assert_eq!(
        decoder.push(key(KeyCode::Char('y'))),
        Err(InputError::PasteTooLarge)
    );
    assert_eq!(decoder.pop(), None);
    send(&mut decoder, "\u{1b}[201~")?;
    assert_eq!(decoder.pop(), None);
    decoder.push(key(KeyCode::Char('a')))?;
    assert_eq!(decoder.pop(), Some(key(KeyCode::Char('a'))));
    Ok(())
}

#[test]
fn oversized_paste_discards_tail_until_end_marker_then_recovers() -> Result<(), InputError> {
    let mut decoder = BracketedPasteDecoder::default();
    send(&mut decoder, "\u{1b}[200~")?;
    for _ in 0..crate::input::MAX_PASTE_BYTES {
        decoder.push(key(KeyCode::Char('x')))?;
    }
    assert_eq!(
        decoder.push(key(KeyCode::Char('y'))),
        Err(InputError::PasteTooLarge)
    );
    for character in "\n/quit\n".chars() {
        let result = if character == '\n' {
            decoder.push(NormalizedInput::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )))
        } else {
            decoder.push(key(KeyCode::Char(character)))
        };
        assert_eq!(result, Ok(()));
    }
    send(&mut decoder, "\u{1b}[201~")?;
    assert_eq!(
        decoder.pop(),
        None,
        "oversized tail must not become a paste event"
    );
    decoder.push(key(KeyCode::Char('x')))?;
    assert_eq!(decoder.pop(), Some(key(KeyCode::Char('x'))));
    assert_eq!(decoder.pop(), None);
    Ok(())
}
