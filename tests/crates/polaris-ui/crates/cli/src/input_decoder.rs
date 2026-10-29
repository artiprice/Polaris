// Unit tests of crates/polaris-ui/crates/cli/src/input_decoder.rs (`tests`); compiled into that module via #[path].
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
