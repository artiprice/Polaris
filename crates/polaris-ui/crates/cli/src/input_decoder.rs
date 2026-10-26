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
        } else if record.key_down && record.unicode_char != 0 {
            self.pending_alt_sequence = false;
        } else if !record.key_down
            && is_alt_virtual_key(record.virtual_key_code)
            && record.unicode_char == 0
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
mod tests {
    use super::{
        ALT_VIRTUAL_KEY, KeyDecoder, LEFT_ALT_VIRTUAL_KEY, NativeKeyRecord, RIGHT_ALT_VIRTUAL_KEY,
        SHIFT_PRESSED,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    fn record(virtual_key_code: u16, unicode_char: u16, state: u32) -> NativeKeyRecord {
        NativeKeyRecord {
            key_down: true,
            repeat_count: 1,
            virtual_key_code,
            unicode_char,
            control_key_state: state,
        }
    }

    #[test]
    fn delayed_unicode_on_alt_release_becomes_one_character_press() {
        for alt_virtual_key in [ALT_VIRTUAL_KEY, LEFT_ALT_VIRTUAL_KEY, RIGHT_ALT_VIRTUAL_KEY] {
            let mut decoder = KeyDecoder::default();
            let alt_down = NativeKeyRecord {
                key_down: true,
                repeat_count: 1,
                virtual_key_code: alt_virtual_key,
                unicode_char: 0,
                control_key_state: 0x0002,
            };
            assert_eq!(decoder.decode(alt_down), None);

            let alt_release_with_character = NativeKeyRecord {
                key_down: false,
                repeat_count: 1,
                virtual_key_code: alt_virtual_key,
                unicode_char: 0x0024,
                control_key_state: 0,
            };
            assert_eq!(
                decoder.decode(alt_release_with_character),
                Some(KeyEvent::new_with_kind(
                    KeyCode::Char(char::from(0x24_u8)),
                    KeyModifiers::NONE,
                    KeyEventKind::Press,
                ))
            );
        }

        let mut decoder = KeyDecoder::default();
        let alt_down = NativeKeyRecord {
            key_down: true,
            repeat_count: 1,
            virtual_key_code: ALT_VIRTUAL_KEY,
            unicode_char: 0,
            control_key_state: 0x0002,
        };
        assert_eq!(decoder.decode(alt_down), None);
        let alt_char_down = NativeKeyRecord {
            key_down: true,
            repeat_count: 1,
            virtual_key_code: 0,
            unicode_char: 0x0024,
            control_key_state: 0x0003,
        };
        assert_eq!(
            decoder.decode(alt_char_down).map(|key| key.kind),
            Some(KeyEventKind::Press)
        );
        let alt_char_release = NativeKeyRecord {
            key_down: false,
            repeat_count: 1,
            virtual_key_code: ALT_VIRTUAL_KEY,
            unicode_char: 0x0024,
            control_key_state: 0,
        };
        assert_eq!(
            decoder.decode(alt_char_release).map(|key| key.kind),
            Some(KeyEventKind::Release)
        );

        let normal_release = NativeKeyRecord {
            key_down: false,
            repeat_count: 1,
            virtual_key_code: 0,
            unicode_char: 0x0061,
            control_key_state: 0,
        };
        assert_eq!(
            decoder.decode(normal_release).map(|key| key.kind),
            Some(KeyEventKind::Release)
        );
    }

    #[test]
    fn c0_shift_enter_keeps_shift_and_enter() {
        let mut decoder = KeyDecoder::default();
        let key = decoder.decode(record(0, 0x000A, 0x0010));
        assert_eq!(
            key.map(|event| (event.code, event.modifiers)),
            Some((KeyCode::Enter, KeyModifiers::SHIFT))
        );
    }

    #[test]
    fn c0_control_enter_is_not_shift_enter() {
        let mut decoder = KeyDecoder::default();
        let key = decoder.decode(record(0x000D, 0x000A, 0x0008));
        assert_eq!(
            key.map(|event| (event.code, event.modifiers)),
            Some((KeyCode::Enter, KeyModifiers::CONTROL))
        );
    }

    #[test]
    fn ambiguous_c0_lf_is_conservatively_control_enter() {
        let mut decoder = KeyDecoder::default();
        let key = decoder.decode(record(0, 0x000A, 0));
        assert_eq!(
            key.map(|event| (event.code, event.modifiers)),
            Some((KeyCode::Enter, KeyModifiers::CONTROL))
        );
    }

    #[test]
    fn control_j_keeps_its_character_identity() {
        let mut decoder = KeyDecoder::default();
        let key = decoder.decode(record(0x004A, 0x000A, 0x0008));
        assert_eq!(
            key.map(|event| (event.code, event.modifiers)),
            Some((KeyCode::Char('j'), KeyModifiers::CONTROL))
        );
    }

    #[test]
    fn native_del_character_is_backspace() {
        let mut decoder = KeyDecoder::default();
        let key = decoder.decode(record(0x0008, 0x007F, 0));
        assert_eq!(key.map(|event| event.code), Some(KeyCode::Backspace));
    }

    #[test]
    fn native_backspace_and_delete_virtual_keys_remain_distinct() {
        let mut decoder = KeyDecoder::default();
        assert_eq!(
            decoder.decode(record(0x0008, 0, 0)).map(|event| event.code),
            Some(KeyCode::Backspace)
        );
        assert_eq!(
            decoder.decode(record(0x002E, 0, 0)).map(|event| event.code),
            Some(KeyCode::Delete)
        );
    }

    #[test]
    fn virtual_key_zero_control_character_is_preserved() {
        let mut decoder = KeyDecoder::default();
        let key = decoder.decode(record(0, 0x0003, 0x0008));
        assert_eq!(
            key.map(|event| (event.code, event.modifiers)),
            Some((KeyCode::Char('c'), KeyModifiers::CONTROL))
        );
    }

    #[test]
    fn c0_control_c_is_inferred_when_native_flags_are_absent() {
        let mut decoder = KeyDecoder::default();
        let key = decoder.decode(record(0, 0x0003, 0));
        assert_eq!(
            key.map(|event| (event.code, event.modifiers)),
            Some((KeyCode::Char('c'), KeyModifiers::CONTROL))
        );
    }

    #[test]
    fn shifted_dollar_key_preserves_its_unicode_character() {
        let mut decoder = KeyDecoder::default();
        let mut dollar = record(0x0034, u16::from(b'$'), SHIFT_PRESSED);
        dollar.repeat_count = 1;
        assert_eq!(
            decoder
                .decode(dollar)
                .map(|event| (event.code, event.modifiers)),
            Some((KeyCode::Char('$'), KeyModifiers::SHIFT))
        );
    }

    #[test]
    fn native_repeat_and_release_phases_are_retained() {
        let mut decoder = KeyDecoder::default();
        let mut repeated = record(0x0D, 0, 0);
        repeated.repeat_count = 4;
        assert_eq!(
            decoder.decode(repeated).map(|event| event.kind),
            Some(KeyEventKind::Repeat)
        );
        let mut released = record(0x0D, 0, 0);
        released.key_down = false;
        assert_eq!(
            decoder.decode(released).map(|event| event.kind),
            Some(KeyEventKind::Release)
        );
    }

    #[test]
    fn adjacent_surrogates_form_one_scalar() {
        let mut decoder = KeyDecoder::default();
        assert_eq!(decoder.decode(record(0, 0xD83D, 0)), None);
        assert_eq!(
            decoder.decode(record(0, 0xDE80, 0)).map(|event| event.code),
            Some(KeyCode::Char('🚀'))
        );
    }
}
