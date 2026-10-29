//! Bounded bracketed-paste recognition for native Windows key records.
use crate::input::{InputError, MAX_PASTE_BYTES, NormalizedInput};
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const START_MARKER: &str = "\u{1b}[200~";
const END_MARKER: &str = "\u{1b}[201~";
const MAX_ESCAPE_SEQUENCE_BYTES: usize = 64;
const ESCAPE_SEQUENCE_TIMEOUT: Duration = Duration::from_millis(40);

/// Converts native input records into ordinary events or one bounded literal paste.
#[derive(Debug, Default)]
pub(crate) struct BracketedPasteDecoder {
    candidate: String,
    pending_marker_keys: VecDeque<NormalizedInput>,
    paste_end_candidate: String,
    paste: String,
    in_paste: bool,
    discarding_paste: bool,
    ready: VecDeque<NormalizedInput>,
    candidate_started: Option<Instant>,
}

impl BracketedPasteDecoder {
    /// Accepts one normalized native record while keeping marker buffers bounded.
    pub(crate) fn push(&mut self, event: NormalizedInput) -> Result<(), InputError> {
        if self.in_paste {
            return self.push_paste(event);
        }
        self.push_outside_paste(event);
        Ok(())
    }

    /// Returns the next complete event, if one is ready.
    pub(crate) fn pop(&mut self) -> Option<NormalizedInput> {
        self.ready.pop_front()
    }

    /// Releases an incomplete Escape sequence after a bounded wait for the rest of its bytes.
    pub(crate) fn flush_expired_escape(&mut self) {
        let expired = self
            .candidate_started
            .is_some_and(|started| started.elapsed() >= ESCAPE_SEQUENCE_TIMEOUT);
        if expired {
            self.flush_start_candidate();
        }
    }

    fn push_outside_paste(&mut self, event: NormalizedInput) {
        if !self.candidate.is_empty()
            && matches!(
                &event,
                NormalizedInput::Key(key) if key.kind == KeyEventKind::Release
            )
        {
            // Console input can pair each character of a VT sequence with a key-up record.
            // Keep the sequence candidate intact; the TUI ignores key-up events separately.
            return;
        }
        let Some(character) = marker_character(&event) else {
            self.flush_start_candidate();
            self.ready.push_back(event);
            return;
        };
        if self.candidate.is_empty() && character != '\u{1b}' {
            self.ready.push_back(event);
            return;
        }
        if self.candidate.is_empty() {
            self.candidate_started = Some(Instant::now());
        }
        let mut proposed = self.candidate.clone();
        proposed.push(character);
        if START_MARKER.starts_with(&proposed) {
            self.candidate = proposed;
            self.pending_marker_keys.push_back(event);
            if self.candidate == START_MARKER {
                self.candidate.clear();
                self.pending_marker_keys.clear();
                self.candidate_started = None;
                self.in_paste = true;
            }
            return;
        }

        if proposed.starts_with("\u{1b}[") {
            if proposed.len() > MAX_ESCAPE_SEQUENCE_BYTES {
                self.flush_start_candidate();
                return;
            }
            if proposed.len() > 2 && is_csi_final(character) {
                if let Some(decoded) = decode_csi(&proposed) {
                    self.ready.push_back(decoded);
                }
                self.candidate.clear();
                self.pending_marker_keys.clear();
                self.candidate_started = None;
                return;
            }
            self.candidate = proposed;
            self.pending_marker_keys.push_back(event);
            return;
        }

        if proposed == "\u{1b}" {
            self.candidate = proposed;
            self.pending_marker_keys.push_back(event);
        } else {
            self.flush_start_candidate();
            if character != '\u{1b}' {
                self.ready.push_back(event);
            }
        }
    }

    fn push_paste(&mut self, event: NormalizedInput) -> Result<(), InputError> {
        let Some(character) = paste_character(&event) else {
            return Ok(());
        };
        if self.discarding_paste {
            self.discard_oversized_paste_character(character);
            return Ok(());
        }
        let mut proposed = self.paste_end_candidate.clone();
        proposed.push(character);
        if END_MARKER.starts_with(&proposed) {
            self.paste_end_candidate = proposed;
            if self.paste_end_candidate == END_MARKER {
                self.paste_end_candidate.clear();
                self.in_paste = false;
                self.discarding_paste = false;
                self.ready
                    .push_back(NormalizedInput::Paste(std::mem::take(&mut self.paste)));
            }
            return Ok(());
        }

        let mut pending = std::mem::take(&mut self.paste_end_candidate);
        pending.push(character);
        for retained in pending.chars() {
            if retained == '\u{1b}' {
                self.paste_end_candidate.push(retained);
            } else if !self.paste_end_candidate.is_empty() {
                let prefix = std::mem::take(&mut self.paste_end_candidate);
                self.append_paste(&prefix)?;
                self.append_paste_char(retained)?;
            } else {
                self.append_paste_char(retained)?;
            }
        }
        if self.paste_end_candidate == END_MARKER {
            self.paste_end_candidate.clear();
            self.in_paste = false;
            self.discarding_paste = false;
            self.ready
                .push_back(NormalizedInput::Paste(std::mem::take(&mut self.paste)));
        }
        Ok(())
    }

    fn append_paste(&mut self, text: &str) -> Result<(), InputError> {
        for character in text.chars() {
            self.append_paste_char(character)?;
        }
        Ok(())
    }

    fn append_paste_char(&mut self, character: char) -> Result<(), InputError> {
        let next_len = self
            .paste
            .len()
            .checked_add(character.len_utf8())
            .ok_or(InputError::PasteTooLarge)?;
        if next_len > MAX_PASTE_BYTES {
            self.paste.clear();
            self.paste_end_candidate.clear();
            self.discarding_paste = true;
            return Err(InputError::PasteTooLarge);
        }
        self.paste.push(character);
        Ok(())
    }

    fn flush_start_candidate(&mut self) {
        self.candidate.clear();
        self.candidate_started = None;
        self.ready.append(&mut self.pending_marker_keys);
    }

    fn discard_oversized_paste_character(&mut self, character: char) {
        let mut proposed = std::mem::take(&mut self.paste_end_candidate);
        proposed.push(character);
        if END_MARKER.starts_with(&proposed) {
            if proposed == END_MARKER {
                self.in_paste = false;
                self.discarding_paste = false;
            } else {
                self.paste_end_candidate = proposed;
            }
        } else if character == '\u{1b}' {
            self.paste_end_candidate.push(character);
        }
    }
}

fn marker_character(event: &NormalizedInput) -> Option<char> {
    let NormalizedInput::Key(key) = event else {
        return None;
    };
    if key.modifiers != KeyModifiers::NONE || key.kind != KeyEventKind::Press {
        return None;
    }
    match key.code {
        KeyCode::Esc => Some('\u{1b}'),
        KeyCode::Char(character) => Some(character),
        _ => None,
    }
}

fn paste_character(event: &NormalizedInput) -> Option<char> {
    let NormalizedInput::Key(key) = event else {
        return None;
    };
    if key.kind == KeyEventKind::Release {
        return None;
    }
    match key.code {
        KeyCode::Char(character) => Some(character),
        KeyCode::Enter => Some('\n'),
        KeyCode::Tab => Some('\t'),
        KeyCode::Esc => Some('\u{1b}'),
        KeyCode::Backspace => Some('\u{7f}'),
        _ => None,
    }
}

fn is_csi_final(character: char) -> bool {
    ('\x40'..='\x7e').contains(&character)
}

fn decode_csi(sequence: &str) -> Option<NormalizedInput> {
    let body = sequence.strip_prefix("\u{1b}[")?;
    let final_byte = body.chars().last()?;
    let parameters = body.get(..body.len().checked_sub(final_byte.len_utf8())?)?;
    if final_byte == 'u' {
        return decode_csi_u(parameters);
    }
    if final_byte == 'Z' {
        return key_event_with_kind(KeyCode::BackTab, KeyModifiers::SHIFT, KeyEventKind::Press);
    }
    if final_byte == 'I' {
        return Some(NormalizedInput::Focus { focused: true });
    }
    if final_byte == 'O' {
        return Some(NormalizedInput::Focus { focused: false });
    }
    let private = parameters.starts_with('<');
    let values = parameters
        .trim_start_matches(['<', '?', '>', '!'])
        .split(';')
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .split(':')
                .next()
                .and_then(|number| number.parse::<usize>().ok())
        })
        .collect::<Option<Vec<_>>>()?;
    let first = values
        .first()
        .copied()
        .filter(|value| *value != 0)
        .unwrap_or(1);
    let second = values
        .get(1)
        .copied()
        .filter(|value| *value != 0)
        .unwrap_or(1);

    let event = match final_byte {
        'A' => key_event(KeyCode::Up, modifier_bits(second)),
        'B' => key_event(KeyCode::Down, modifier_bits(second)),
        'C' => key_event(KeyCode::Right, modifier_bits(second)),
        'D' => key_event(KeyCode::Left, modifier_bits(second)),
        'H' => key_event(KeyCode::Home, modifier_bits(second)),
        'F' => key_event(KeyCode::End, modifier_bits(second)),
        '~' => match first {
            1 | 7 => key_event(KeyCode::Home, KeyModifiers::NONE),
            2 => key_event(KeyCode::Insert, modifier_bits(second)),
            3 => key_event(KeyCode::Delete, modifier_bits(second)),
            4 | 8 => key_event(KeyCode::End, modifier_bits(second)),
            5 => key_event(KeyCode::PageUp, modifier_bits(second)),
            6 => key_event(KeyCode::PageDown, modifier_bits(second)),
            11 => key_event(KeyCode::F(1), modifier_bits(second)),
            12 => key_event(KeyCode::F(2), modifier_bits(second)),
            13 => key_event(KeyCode::F(3), modifier_bits(second)),
            14 => key_event(KeyCode::F(4), modifier_bits(second)),
            15 => key_event(KeyCode::F(5), modifier_bits(second)),
            17 => key_event(KeyCode::F(6), modifier_bits(second)),
            18 => key_event(KeyCode::F(7), modifier_bits(second)),
            19 => key_event(KeyCode::F(8), modifier_bits(second)),
            20 => key_event(KeyCode::F(9), modifier_bits(second)),
            21 => key_event(KeyCode::F(10), modifier_bits(second)),
            23 => key_event(KeyCode::F(11), modifier_bits(second)),
            24 => key_event(KeyCode::F(12), modifier_bits(second)),
            _ => None,
        },
        'M' | 'm' if private => decode_sgr_mouse(&values, final_byte),
        _ => None,
    }?;
    Some(event)
}

fn decode_csi_u(parameters: &str) -> Option<NormalizedInput> {
    let mut fields = parameters.split(';');
    let code_field = fields.next()?;
    let code_point = code_field.split(':').next()?.parse::<u32>().ok()?;
    let modifier_field = fields.next().unwrap_or("1");
    let mut modifier_parts = modifier_field.split(':');
    let encoded_modifiers = match modifier_parts.next() {
        Some(value) if !value.is_empty() => value.parse::<usize>().ok()?,
        _ => 1,
    };
    let event_type = match modifier_parts.next() {
        Some(value) => value.parse::<u8>().ok()?,
        None => 1,
    };
    let modifiers = modifier_bits(encoded_modifiers);
    let code = match code_point {
        8 | 127 => KeyCode::Backspace,
        9 if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        9 => KeyCode::Tab,
        13 => KeyCode::Enter,
        27 => KeyCode::Esc,
        value => KeyCode::Char(char::from_u32(value)?),
    };
    let kind = match event_type {
        2 => KeyEventKind::Repeat,
        3 => KeyEventKind::Release,
        _ => KeyEventKind::Press,
    };
    key_event_with_kind(code, modifiers, kind)
}

fn key_event(code: KeyCode, modifiers: KeyModifiers) -> Option<NormalizedInput> {
    key_event_with_kind(code, modifiers, KeyEventKind::Press)
}

fn key_event_with_kind(
    code: KeyCode,
    modifiers: KeyModifiers,
    kind: KeyEventKind,
) -> Option<NormalizedInput> {
    Some(NormalizedInput::Key(KeyEvent::new_with_kind(
        code, modifiers, kind,
    )))
}

fn modifier_bits(encoded: usize) -> KeyModifiers {
    let bits = encoded.saturating_sub(1);
    let mut modifiers = KeyModifiers::empty();
    if bits & 1 != 0 {
        modifiers.insert(KeyModifiers::SHIFT);
    }
    if bits & 2 != 0 {
        modifiers.insert(KeyModifiers::ALT);
    }
    if bits & 4 != 0 {
        modifiers.insert(KeyModifiers::CONTROL);
    }
    modifiers
}

fn decode_sgr_mouse(values: &[usize], final_byte: char) -> Option<NormalizedInput> {
    let encoded = values.first().copied()?;
    let column = values.get(1)?.checked_sub(1)?;
    let row = values.get(2)?.checked_sub(1)?;
    let button = encoded & 3;
    let motion = encoded & 32 != 0;
    let scroll = encoded & 64 != 0;
    let kind = if scroll {
        match button {
            0 => MouseEventKind::ScrollUp,
            1 => MouseEventKind::ScrollDown,
            2 => MouseEventKind::ScrollLeft,
            _ => MouseEventKind::ScrollRight,
        }
    } else if motion && button != 3 {
        MouseEventKind::Drag(mouse_button(button))
    } else if motion {
        MouseEventKind::Moved
    } else if final_byte == 'm' || encoded & 3 == 3 {
        MouseEventKind::Up(mouse_button(button))
    } else {
        MouseEventKind::Down(mouse_button(button))
    };
    let mut modifiers = KeyModifiers::empty();
    if encoded & 4 != 0 {
        modifiers.insert(KeyModifiers::SHIFT);
    }
    if encoded & 8 != 0 {
        modifiers.insert(KeyModifiers::ALT);
    }
    if encoded & 16 != 0 {
        modifiers.insert(KeyModifiers::CONTROL);
    }
    Some(NormalizedInput::Mouse(MouseEvent {
        kind,
        column: u16::try_from(column).ok()?,
        row: u16::try_from(row).ok()?,
        modifiers,
    }))
}

fn mouse_button(button: usize) -> MouseButton {
    match button {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::Left,
    }
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-ui/crates/cli/src/bracketed_paste.rs"]
mod tests;
