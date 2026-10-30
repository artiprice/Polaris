//! Pure decoder for Windows console key records.
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

const SHIFT_PRESSED: u32 = 0x0010;
const ALT_STATE_MASK: u32 = 0x0003;
const ALT_VIRTUAL_KEY: u16 = 0x0012;
const LEFT_ALT_VIRTUAL_KEY: u16 = 0x00A4;
const RIGHT_ALT_VIRTUAL_KEY: u16 = 0x00A5;
const LEFT_CTRL_PRESSED: u32 = 0x0008;
const RIGHT_CTRL_PRESSED: u32 = 0x0004;
const LEFT_ALT_PRESSED: u32 = 0x0002;
const RIGHT_ALT_PRESSED: u32 = 0x0001;
const VK_J: u16 = 0x004A;

/// The fields of a native key record needed by the application decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NativeKeyRecord {
    pub key_down: bool,
    pub repeat_count: u16,
    pub virtual_key_code: u16,
    pub unicode_char: u16,
    pub control_key_state: u32,
}

/// Holds a pending high surrogate between adjacent native key records.
#[derive(Debug, Default)]
pub(crate) struct KeyDecoder {
    pending_high_surrogate: Option<u16>,
    /// Tracks Alt compositions whose Unicode result is delivered on a key-up record.
    pending_alt_sequence: bool,
}

impl KeyDecoder {
    /// Converts one native key record while preserving modifiers and event phase.
    pub(crate) fn decode(&mut self, record: NativeKeyRecord) -> Option<KeyEvent> {
        // ConPTY can attach translated text to the Alt key-up instead of its paired key-down.
        let delayed_alt_character =
            !record.key_down && record.unicode_char != 0 && self.pending_alt_sequence;
        if record.key_down
            && (is_alt_virtual_key(record.virtual_key_code)
                || (record.unicode_char == 0 && record.control_key_state & ALT_STATE_MASK != 0))
        {
            self.pending_alt_sequence = true;
        } else if (record.key_down && record.unicode_char != 0)
            || (!record.key_down
                && is_alt_virtual_key(record.virtual_key_code)
                && record.unicode_char == 0)
        {
            self.pending_alt_sequence = false;
        }

        let mut phase = if record.key_down {
            if record.repeat_count > 1 {
                KeyEventKind::Repeat
            } else {
                KeyEventKind::Press
            }
        } else {
            KeyEventKind::Release
        };
        if delayed_alt_character {
            self.pending_alt_sequence = false;
            phase = KeyEventKind::Press;
        }
        let mut modifiers = modifiers(record.control_key_state);
        if is_c0_control_letter(record.unicode_char) {
            modifiers.insert(KeyModifiers::CONTROL);
        }
        if record.unicode_char == 0x000A && record.control_key_state & SHIFT_PRESSED == 0 {
            modifiers.insert(KeyModifiers::CONTROL);
        }
        let code = self.decode_code(record)?;
        Some(KeyEvent::new_with_kind(code, modifiers, phase))
    }

    fn decode_code(&mut self, record: NativeKeyRecord) -> Option<KeyCode> {
        let unit = record.unicode_char;
        if (0xD800..=0xDBFF).contains(&unit) {
            self.pending_high_surrogate = Some(unit);
            return None;
        }
        if (0xDC00..=0xDFFF).contains(&unit) {
            let high = self.pending_high_surrogate.take()?;
            return char::decode_utf16([high, unit])
                .next()
                .and_then(Result::ok)
                .map(KeyCode::Char);
        }
        self.pending_high_surrogate = None;
        if unit != 0 {
            if unit == 0x000A {
                if record.control_key_state & SHIFT_PRESSED != 0 {
                    return Some(KeyCode::Enter);
                }
                return if record.virtual_key_code == VK_J {
                    Some(KeyCode::Char('j'))
                } else {
                    // LF without a Shift bit is ambiguous in console records. Preserve Enter so
                    // Ctrl+Enter stays inert; only the J virtual key proves Ctrl+J.
                    Some(KeyCode::Enter)
                };
            }
            return c0_or_scalar(unit);
        }
        virtual_key(record.virtual_key_code)
    }
}

fn is_alt_virtual_key(code: u16) -> bool {
    matches!(
        code,
        ALT_VIRTUAL_KEY | LEFT_ALT_VIRTUAL_KEY | RIGHT_ALT_VIRTUAL_KEY
    )
}

fn is_c0_control_letter(unit: u16) -> bool {
    (0x0001..=0x001A).contains(&unit) && !matches!(unit, 0x0008 | 0x0009 | 0x000A | 0x000D)
}

fn modifiers(state: u32) -> KeyModifiers {
    let mut result = KeyModifiers::empty();
    if state & SHIFT_PRESSED != 0 {
        result.insert(KeyModifiers::SHIFT);
    }
    if state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0 {
        result.insert(KeyModifiers::CONTROL);
    }
    if state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0 {
        result.insert(KeyModifiers::ALT);
    }
    result
}

fn c0_or_scalar(unit: u16) -> Option<KeyCode> {
    match unit {
        0x0000 => Some(KeyCode::Char(' ')),
        0x0008 => Some(KeyCode::Backspace),
        0x007F => Some(KeyCode::Backspace),
        0x0009 => Some(KeyCode::Tab),
        0x000A | 0x000D => Some(KeyCode::Enter),
        0x001B => Some(KeyCode::Esc),
        0x0001..=0x001A => {
            let offset = u32::from(unit).checked_sub(1)?;
            char::from_u32(u32::from('a').checked_add(offset)?).map(KeyCode::Char)
        }
        _ => char::from_u32(u32::from(unit)).map(KeyCode::Char),
    }
}

fn virtual_key(code: u16) -> Option<KeyCode> {
    match code {
        0x08 => Some(KeyCode::Backspace),
        0x09 => Some(KeyCode::Tab),
        0x0D => Some(KeyCode::Enter),
        0x1B => Some(KeyCode::Esc),
        0x20 => Some(KeyCode::Char(' ')),
        0x21 => Some(KeyCode::PageUp),
        0x22 => Some(KeyCode::PageDown),
        0x23 => Some(KeyCode::End),
        0x24 => Some(KeyCode::Home),
        0x25 => Some(KeyCode::Left),
        0x26 => Some(KeyCode::Up),
        0x27 => Some(KeyCode::Right),
        0x28 => Some(KeyCode::Down),
        0x2D => Some(KeyCode::Insert),
        0x2E => Some(KeyCode::Delete),
        0x70 => Some(KeyCode::F(1)),
        0x71 => Some(KeyCode::F(2)),
        0x72 => Some(KeyCode::F(3)),
        0x73 => Some(KeyCode::F(4)),
        0x74 => Some(KeyCode::F(5)),
        0x75 => Some(KeyCode::F(6)),
        0x76 => Some(KeyCode::F(7)),
        0x77 => Some(KeyCode::F(8)),
        0x78 => Some(KeyCode::F(9)),
        0x79 => Some(KeyCode::F(10)),
        0x7A => Some(KeyCode::F(11)),
        0x7B => Some(KeyCode::F(12)),
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-ui/crates/cli/src/input_decoder.rs"]
mod tests;
