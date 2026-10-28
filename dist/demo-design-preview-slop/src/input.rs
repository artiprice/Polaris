//! Session-local console modes and enhanced terminal keyboard decoding.
//!
//! Preserve terminal escape sequences through ConPTY, including C0 records
//! with no virtual key code. Restore the original console mode on exit.
#[cfg(windows)]
#[path = "input_windows.rs"]
mod windows;
#[cfg(windows)]
pub use windows::TerminalInput;

#[cfg(not(windows))]
pub struct TerminalInput;
#[cfg(not(windows))]
impl TerminalInput {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self)
    }
    pub fn poll(&mut self, timeout: std::time::Duration) -> std::io::Result<bool> {
        crossterm::event::poll(timeout)
    }
    pub fn read(&mut self) -> std::io::Result<crossterm::event::Event> {
        crossterm::event::read()
    }
}
#[cfg(windows)]
pub struct NativeConsoleInput {
    console: std::fs::File,
    original: u32,
}

#[cfg(windows)]
impl NativeConsoleInput {
    pub fn enter() -> std::io::Result<Self> {
        use std::{fs::OpenOptions, os::windows::io::AsRawHandle};
        use winapi::um::{
            consoleapi::{GetConsoleMode, SetConsoleMode},
            wincon::ENABLE_VIRTUAL_TERMINAL_INPUT,
        };

        let console = OpenOptions::new().read(true).write(true).open("CONIN$")?;
        let mut original = 0;
        // SAFETY: the owned CONIN$ handle stays live through both calls, and
        // GetConsoleMode receives a valid writable u32.
        unsafe {
            if GetConsoleMode(console.as_raw_handle().cast(), &mut original) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            if SetConsoleMode(
                console.as_raw_handle().cast(),
                original | ENABLE_VIRTUAL_TERMINAL_INPUT,
            ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(Self { console, original })
    }
}

#[cfg(windows)]
impl Drop for NativeConsoleInput {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: self.console owns this still-live console handle.
        unsafe {
            winapi::um::consoleapi::SetConsoleMode(
                self.console.as_raw_handle().cast(),
                self.original,
            );
        }
    }
}

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::time::{Duration, Instant};

/// Normalize console control characters and the two Shift+Tab event forms.
/// The supplied Shift snapshot only applies to Enter/Tab; it cannot turn
/// ordinary text into shifted characters or change unrelated key bindings.
pub fn normalize_key(mut key: KeyEvent, shift_down: bool) -> KeyEvent {
    key.code = match key.code {
        KeyCode::Char('\r') => KeyCode::Enter,
        KeyCode::Char('\t') => KeyCode::Tab,
        KeyCode::Char('\x1b') => KeyCode::Esc,
        code => code,
    };
    if shift_down
        && key.kind != KeyEventKind::Release
        && matches!(key.code, KeyCode::Enter | KeyCode::Tab)
    {
        key.modifiers.insert(KeyModifiers::SHIFT);
    }
    if key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT) {
        key.code = KeyCode::BackTab;
    }
    key
}

/// ConPTY hosts can collapse shifted Enter/Tab into ordinary key records.
/// Sample only the held Shift state when one of those received events needs it.
/// Do not poll or record general keyboard input, and do not use the unreliable
/// "pressed since last call" bit returned by GetAsyncKeyState.
pub fn recover_console_shift(key: KeyEvent) -> KeyEvent {
    #[cfg(windows)]
    let shift_down = if key.kind != KeyEventKind::Release
        && !key.modifiers.contains(KeyModifiers::SHIFT)
        && matches!(
            key.code,
            KeyCode::Enter | KeyCode::Tab | KeyCode::Char('\r' | '\t')
        ) {
        // SAFETY: this queries a single virtual key, with no pointers or hooks.
        unsafe { winapi::um::winuser::GetAsyncKeyState(winapi::um::winuser::VK_SHIFT) < 0 }
    } else {
        false
    };
    #[cfg(not(windows))]
    let shift_down = false;
    normalize_key(key, shift_down)
}

/// Decode CSI-u when a Windows console forwards enhanced terminal keys as
/// individual key records. On Unix, Crossterm normally decodes these first.
#[derive(Default)]
pub struct KeyboardDecoder {
    pending: Vec<KeyEvent>,
    since: Option<Instant>,
    paste: Option<PasteBuffer>,
    pasted: Vec<String>,
    paste_errors: Vec<String>,
    mouse: Vec<MouseEvent>,
    mode_replies: Vec<(u16, u16)>,
}

const MAX_BRACKETED_PASTE_BYTES: usize = 1024 * 1024;

#[derive(Default)]
struct PasteBuffer {
    text: String,
    tail: String,
    overflowed: bool,
}
impl PasteBuffer {
    fn push(&mut self, ch: char) -> bool {
        const END: &str = "\x1b[201~";
        self.tail.push(ch);
        if self.tail.ends_with(END) {
            self.tail.truncate(self.tail.len() - END.len());
            return true;
        }
        while self.tail.len() > END.len() {
            let first = self.tail.remove(0);
            if self.text.len() + first.len_utf8() <= MAX_BRACKETED_PASTE_BYTES {
                self.text.push(first);
            } else {
                self.overflowed = true;
            }
        }
        false
    }
    fn finish(mut self) -> Result<String, ()> {
        for ch in self.tail.chars() {
            if self.text.len() + ch.len_utf8() <= MAX_BRACKETED_PASTE_BYTES {
                self.text.push(ch);
            } else {
                self.overflowed = true;
            }
        }
        if self.overflowed {
            Err(())
        } else {
            Ok(self.text)
        }
    }
}
impl KeyboardDecoder {
    pub fn is_pasting(&self) -> bool {
        self.paste.is_some()
    }
    pub fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub fn take_pastes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pasted)
    }
    pub fn take_paste_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.paste_errors)
    }
    pub fn take_mode_replies(&mut self) -> Vec<(u16, u16)> {
        std::mem::take(&mut self.mode_replies)
    }
    pub fn take_mouse(&mut self) -> Vec<MouseEvent> {
        std::mem::take(&mut self.mouse)
    }
    pub fn feed(&mut self, key: KeyEvent) -> Vec<KeyEvent> {
        if key.kind == KeyEventKind::Release {
            return vec![];
        }
        if let Some(paste) = &mut self.paste {
            let ch = match key.code {
                KeyCode::Char(ch)
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && ch.is_ascii_alphabetic() =>
                {
                    (ch.to_ascii_lowercase() as u8 & 0x1f) as char
                }
                KeyCode::Char(ch) => ch,
                KeyCode::Esc => '\x1b',
                KeyCode::Enter => '\r',
                KeyCode::Tab | KeyCode::BackTab => '\t',
                KeyCode::Backspace => '\x08',
                _ => return vec![],
            };
            if paste.push(ch) {
                match self.paste.take().unwrap().finish() {
                    Ok(text) => self.pasted.push(text),
                    Err(()) => self.paste_errors.push(format!(
                        "Bracketed paste exceeds the {} byte limit; nothing was pasted.",
                        MAX_BRACKETED_PASTE_BYTES
                    )),
                }
            }
            return vec![];
        }
        let key = normalize_key(key, false);
        if key.kind == KeyEventKind::Release {
            return vec![];
        }
        if self.pending.is_empty() {
            if key.code == KeyCode::Esc && key.modifiers.is_empty() {
                self.pending.push(key);
                self.since = Some(Instant::now());
                return vec![];
            }
            return vec![key];
        }
        let valid = if self.pending.len() == 1 {
            matches!(key.code, KeyCode::Char('[' | 'O'))
        } else {
            matches!(key.code, KeyCode::Char(ch) if ch.is_ascii_alphanumeric() || matches!(ch, '<' | ';' | ':' | '~' | '?' | '$'))
        };
        if !valid || self.pending.len() > 64 {
            let mut out = self.drain();
            out.extend(self.feed(key));
            return out;
        }
        self.pending.push(key);
        self.since = Some(Instant::now());
        if self.pending.len() > 2
            && matches!(key.code, KeyCode::Char(ch) if ch.is_ascii_alphabetic() || ch == '~')
        {
            let sequence: String = self
                .pending
                .iter()
                .skip(2)
                .filter_map(|key| {
                    if let KeyCode::Char(ch) = key.code {
                        Some(ch)
                    } else {
                        None
                    }
                })
                .collect();
            if let Some(reply) = sequence
                .strip_prefix('?')
                .and_then(|s| s.strip_suffix("$y"))
                && let Some((mode, state)) = reply.split_once(';')
                && let (Ok(mode), Ok(state)) = (mode.parse::<u16>(), state.parse::<u16>())
            {
                self.drain();
                self.mode_replies.push((mode, state));
                return vec![];
            }
            if sequence == "200~" {
                self.drain();
                self.paste = Some(PasteBuffer::default());
                return vec![];
            }
            if sequence.starts_with('<') {
                self.drain();
                if let Some(mouse) = decode_sgr_mouse(&sequence) {
                    self.mouse.push(mouse);
                }
                return vec![];
            }
            if let Some(decoded) = decode_enhanced_key(&sequence) {
                self.drain();
                return vec![decoded];
            }
            return self.drain();
        }
        vec![]
    }
    fn drain(&mut self) -> Vec<KeyEvent> {
        self.since = None;
        std::mem::take(&mut self.pending)
    }
    pub fn expire(&mut self) -> Vec<KeyEvent> {
        if self
            .since
            .is_some_and(|since| since.elapsed() >= Duration::from_millis(80))
        {
            self.drain()
        } else {
            vec![]
        }
    }
}

fn decode_navigation_key(sequence: &str) -> Option<KeyEvent> {
    let final_byte = sequence.chars().last()?;
    let body = &sequence[..sequence.len() - final_byte.len_utf8()];
    let mut fields = body.split(';');
    let number = fields.next().unwrap_or("");
    let modifier = fields
        .next()
        .unwrap_or("1")
        .parse::<u8>()
        .ok()?
        .checked_sub(1)?;
    if fields.next().is_some() {
        return None;
    }
    let code = match final_byte {
        'A' => KeyCode::Up,
        'B' => KeyCode::Down,
        'C' => KeyCode::Right,
        'D' => KeyCode::Left,
        'H' => KeyCode::Home,
        'F' => KeyCode::End,
        'Z' => KeyCode::BackTab,
        'P' => KeyCode::F(1),
        'Q' => KeyCode::F(2),
        'R' => KeyCode::F(3),
        'S' => KeyCode::F(4),
        '~' => match number {
            "1" | "7" => KeyCode::Home,
            "2" => KeyCode::Insert,
            "3" => KeyCode::Delete,
            "4" | "8" => KeyCode::End,
            "5" => KeyCode::PageUp,
            "6" => KeyCode::PageDown,
            "11" => KeyCode::F(1),
            "12" => KeyCode::F(2),
            "13" => KeyCode::F(3),
            "14" => KeyCode::F(4),
            "15" => KeyCode::F(5),
            "17" => KeyCode::F(6),
            "18" => KeyCode::F(7),
            "19" => KeyCode::F(8),
            "20" => KeyCode::F(9),
            "21" => KeyCode::F(10),
            "23" => KeyCode::F(11),
            "24" => KeyCode::F(12),
            _ => return None,
        },
        _ => return None,
    };
    let mut modifiers = KeyModifiers::NONE;
    if modifier & 1 != 0 || final_byte == 'Z' {
        modifiers |= KeyModifiers::SHIFT;
    }
    if modifier & 2 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if modifier & 4 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    Some(KeyEvent::new(code, modifiers))
}

fn decode_enhanced_key(sequence: &str) -> Option<KeyEvent> {
    if let Some(key) = decode_navigation_key(sequence) {
        return Some(key);
    }
    let (body, legacy) = if let Some(body) = sequence.strip_suffix('u') {
        (body, false)
    } else {
        (sequence.strip_suffix('~')?, true)
    };
    let fields: Vec<_> = body.split(';').collect();
    let (codepoint, modifier) = if legacy {
        // xterm modifyOtherKeys, also used by manually configured terminals.
        if fields.len() != 3 || fields[0] != "27" {
            return None;
        }
        (fields[2].parse::<u32>().ok()?, fields[1])
    } else {
        (
            fields.first()?.split(':').next()?.parse::<u32>().ok()?,
            fields.get(1).copied().unwrap_or("1"),
        )
    };
    let mut parts = modifier.split(':');
    let bits = parts.next()?.parse::<u8>().ok()?.checked_sub(1)?;
    let kind = match parts.next().unwrap_or("1") {
        "1" => KeyEventKind::Press,
        "2" => KeyEventKind::Repeat,
        "3" => KeyEventKind::Release,
        _ => return None,
    };
    let mut modifiers = KeyModifiers::NONE;
    for (bit, flag) in [
        (1, KeyModifiers::SHIFT),
        (2, KeyModifiers::ALT),
        (4, KeyModifiers::CONTROL),
        (8, KeyModifiers::SUPER),
        (16, KeyModifiers::HYPER),
        (32, KeyModifiers::META),
    ] {
        if bits & bit != 0 {
            modifiers |= flag;
        }
    }
    let code = match codepoint {
        13 | 57345 | 57414 => KeyCode::Enter,
        9 | 57346 if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        9 | 57346 => KeyCode::Tab,
        27 | 57344 => KeyCode::Esc,
        127 | 57347 => KeyCode::Backspace,
        57348 | 57425 => KeyCode::Insert,
        57349 | 57426 => KeyCode::Delete,
        57350 | 57417 => KeyCode::Left,
        57351 | 57418 => KeyCode::Right,
        57352 | 57419 => KeyCode::Up,
        57353 | 57420 => KeyCode::Down,
        57354 | 57421 => KeyCode::PageUp,
        57355 | 57422 => KeyCode::PageDown,
        57356 | 57423 => KeyCode::Home,
        57357 | 57424 => KeyCode::End,
        57358 => KeyCode::CapsLock,
        57359 => KeyCode::ScrollLock,
        57360 => KeyCode::NumLock,
        57361 => KeyCode::PrintScreen,
        57362 => KeyCode::Pause,
        57363 => KeyCode::Menu,
        57399..=57408 => KeyCode::Char(char::from_u32('0' as u32 + codepoint - 57399)?),
        57409 => KeyCode::Char('.'),
        57410 => KeyCode::Char('/'),
        57411 => KeyCode::Char('*'),
        57412 => KeyCode::Char('-'),
        57413 => KeyCode::Char('+'),
        57415 => KeyCode::Char('='),
        57416 => KeyCode::Char(','),
        57364..=57398 => KeyCode::F((codepoint - 57363) as u8),
        code => KeyCode::Char(char::from_u32(code)?),
    };
    Some(KeyEvent::new_with_kind(code, modifiers, kind))
}

fn decode_sgr_mouse(sequence: &str) -> Option<MouseEvent> {
    let final_byte = sequence.chars().last()?;
    if !matches!(final_byte, 'M' | 'm') {
        return None;
    }
    let mut fields = sequence[1..sequence.len() - 1].split(';');
    let code = fields.next()?.parse::<u16>().ok()?;
    let column = fields.next()?.parse::<u16>().ok()?.saturating_sub(1);
    let row = fields.next()?.parse::<u16>().ok()?.saturating_sub(1);
    if fields.next().is_some() {
        return None;
    }
    let mut modifiers = KeyModifiers::NONE;
    if code & 4 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if code & 8 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if code & 16 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    let button = match code & 3 {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::Left,
    };
    let kind = if code & 64 != 0 {
        match code & 3 {
            0 => MouseEventKind::ScrollUp,
            1 => MouseEventKind::ScrollDown,
            2 => MouseEventKind::ScrollLeft,
            _ => MouseEventKind::ScrollRight,
        }
    } else if code & 32 != 0 {
        if code & 3 == 3 {
            MouseEventKind::Moved
        } else {
            MouseEventKind::Drag(button)
        }
    } else if final_byte == 'm' || code & 3 == 3 {
        MouseEventKind::Up(button)
    } else {
        MouseEventKind::Down(button)
    };
    Some(MouseEvent {
        kind,
        column,
        row,
        modifiers,
    })
}

/// Enable button, drag, hover events and SGR coordinates.
pub fn push_mouse_protocol(writer: &mut impl std::io::Write) -> std::io::Result<()> {
    writer.write_all(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h")?;
    writer.flush()
}
/// Disable the mouse modes in reverse order, balancing the push operation.
pub fn pop_mouse_protocol(writer: &mut impl std::io::Write) -> std::io::Result<()> {
    writer.write_all(b"\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l")?;
    writer.flush()
}

/// Write the protocol directly: Crossterm 0.28's Windows command rejects it
/// even when the actual terminal (including Alacritty) supports CSI-u.
pub fn push_keyboard_protocol(writer: &mut impl std::io::Write) -> std::io::Result<()> {
    writer.write_all(b"\x1b[>1u")?;
    writer.flush()
}
pub fn pop_keyboard_protocol(writer: &mut impl std::io::Write) -> std::io::Result<()> {
    writer.write_all(b"\x1b[<u")?;
    writer.flush()
}

#[cfg(test)]
mod decoder_tests {
    use super::*;
    fn decode(text: &str) -> Vec<KeyEvent> {
        let mut decoder = KeyboardDecoder::default();
        let mut out = Vec::new();
        for ch in text.chars() {
            out.extend(decoder.feed(KeyEvent::new(
                if ch == '\x1b' {
                    KeyCode::Esc
                } else {
                    KeyCode::Char(ch)
                },
                KeyModifiers::NONE,
            )));
        }
        out
    }
    #[test]
    fn encoded_shift_enter_is_distinct_from_control_enter_and_plain_enter() {
        for (seq, modifier) in [
            ("\x1b[13;2u", KeyModifiers::SHIFT),
            ("\x1b[13;5u", KeyModifiers::CONTROL),
            ("\x1b[13u", KeyModifiers::NONE),
            ("\x1b[27;2;13~", KeyModifiers::SHIFT),
        ] {
            assert_eq!(decode(seq), vec![KeyEvent::new(KeyCode::Enter, modifier)]);
        }
    }
    #[test]
    fn enhanced_shortcuts_and_release_events_preserve_their_meaning() {
        assert_eq!(
            decode("\x1b[99;5u"),
            vec![KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)]
        );
        assert_eq!(
            decode("\x1b[9;2u"),
            vec![KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)]
        );
        assert_eq!(decode("\x1b[13;2:3u")[0].kind, KeyEventKind::Release);
        assert_eq!(
            decode("\x1b[27u"),
            vec![KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)]
        );
        assert_eq!(decode("abc").len(), 3);
        assert_eq!(
            decode("\x1b[57400u"),
            vec![KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)]
        );
        assert_eq!(
            decode("\x1b[57417u"),
            vec![KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)]
        );
        assert_eq!(
            decode("\x1b[57358u"),
            vec![KeyEvent::new(KeyCode::CapsLock, KeyModifiers::NONE)]
        );
    }
    #[test]
    fn escape_is_retained_and_unsupported_sequences_are_not_silently_lost() {
        let mut decoder = KeyboardDecoder::default();
        assert!(
            decoder
                .feed(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
                .is_empty()
        );
        decoder.since = Some(Instant::now() - Duration::from_millis(100));
        assert_eq!(
            decoder.expire(),
            vec![KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)]
        );
        assert_eq!(decode("\x1bx").len(), 2);
        assert_eq!(decode("\x1b[99999999u").len(), 11);
    }
    #[test]
    fn navigation_survives_raw_vt_input() {
        for (seq, expected) in [
            ("\x1b[A", KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            ("\x1bOB", KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            (
                "\x1b[1;5D",
                KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL),
            ),
            (
                "\x1b[3~",
                KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
            ),
            ("\x1bOP", KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)),
        ] {
            assert_eq!(decode(seq), vec![expected]);
        }
    }
    #[test]
    fn bracketed_paste_is_atomic_and_does_not_turn_newlines_into_submissions() {
        let mut decoder = KeyboardDecoder::default();
        let mut keys = Vec::new();
        for ch in "\x1b[200~first\r\nsecond\x1b[13;2u\x1b[201~".chars() {
            keys.extend(decoder.feed(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)));
        }
        assert!(keys.is_empty());
        let pasted = decoder.take_pastes();
        assert_eq!(pasted, ["first\r\nsecond\x1b[13;2u"]);
        assert!(decoder.take_pastes().is_empty());
        let mut app = crate::app::App::default();
        app.paste(&pasted[0]);
        assert!(!app.working);
        assert_eq!(app.input.text, "first\nsecond[13;2u");
        assert_eq!(
            decoder.feed(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            vec![KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)]
        );
    }
    #[test]
    fn paste_limit_accepts_one_mib_and_discards_oversize_without_sticking() {
        let mut decoder = KeyboardDecoder::default();
        let exact = "a".repeat(MAX_BRACKETED_PASTE_BYTES);
        let text = format!("\x1b[200~{exact}\x1b[201~");
        for ch in text.chars() {
            assert!(
                decoder
                    .feed(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                    .is_empty()
            );
        }
        let pasted = decoder.take_pastes();
        assert_eq!(pasted.len(), 1);
        assert_eq!(pasted[0].len(), MAX_BRACKETED_PASTE_BYTES);
        assert!(decoder.take_paste_errors().is_empty());

        let oversized = format!(
            "\x1b[200~{}\x1b[201~",
            "b".repeat(MAX_BRACKETED_PASTE_BYTES + 1)
        );
        for ch in oversized.chars() {
            assert!(
                decoder
                    .feed(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                    .is_empty()
            );
        }
        assert!(decoder.take_pastes().is_empty());
        let errors = decoder.take_paste_errors();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("nothing was pasted"));
        assert_eq!(
            decoder.feed(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)),
            vec![KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE)]
        );
    }
    #[test]
    fn sgr_mouse_click_release_drag_wheel_and_modifiers_decode() {
        let mut decoder = KeyboardDecoder::default();
        for ch in "\x1b[<0;3;4M\x1b[<0;3;4m\x1b[<32;5;6M\x1b[<65;7;8M\x1b[<12;1;2M".chars() {
            decoder.feed(KeyEvent::new(
                if ch == '\x1b' {
                    KeyCode::Esc
                } else {
                    KeyCode::Char(ch)
                },
                KeyModifiers::NONE,
            ));
        }
        let events = decoder.take_mouse();
        assert_eq!(events.len(), 5);
        assert_eq!(events[0].kind, MouseEventKind::Down(MouseButton::Left));
        assert_eq!(events[1].kind, MouseEventKind::Up(MouseButton::Left));
        assert_eq!(events[2].kind, MouseEventKind::Drag(MouseButton::Left));
        assert_eq!(events[3].kind, MouseEventKind::ScrollDown);
        assert_eq!((events[0].column, events[0].row), (2, 3));
        assert_eq!(events[4].modifiers, KeyModifiers::ALT | KeyModifiers::SHIFT);
    }
    #[test]
    fn sgr_mouse_is_ignored_inside_bracketed_paste() {
        let mut decoder = KeyboardDecoder::default();
        for ch in "\x1b[200~\x1b[<0;2;3Mdata\x1b[201~".chars() {
            decoder.feed(KeyEvent::new(
                if ch == '\x1b' {
                    KeyCode::Esc
                } else {
                    KeyCode::Char(ch)
                },
                KeyModifiers::NONE,
            ));
        }
        assert!(decoder.take_mouse().is_empty());
        assert_eq!(decoder.take_pastes(), ["\x1b[<0;2;3Mdata"]);
    }
    #[test]
    fn mouse_protocol_is_balanced() {
        let mut bytes = vec![];
        push_mouse_protocol(&mut bytes).unwrap();
        pop_mouse_protocol(&mut bytes).unwrap();
        assert_eq!(
            bytes,
            b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l"
        );
    }
    #[test]
    fn protocol_is_requested_on_windows_too_and_balanced_on_exit() {
        let mut bytes = vec![];
        push_keyboard_protocol(&mut bytes).unwrap();
        pop_keyboard_protocol(&mut bytes).unwrap();
        assert_eq!(bytes, b"\x1b[>1u\x1b[<u");
    }
}

#[cfg(test)]
mod reply_tests {
    use super::*;
    fn feed(d: &mut KeyboardDecoder, text: &str) -> Vec<KeyEvent> {
        text.chars()
            .flat_map(|c| {
                d.feed(KeyEvent::new(
                    if c == '\x1b' {
                        KeyCode::Esc
                    } else {
                        KeyCode::Char(c)
                    },
                    KeyModifiers::NONE,
                ))
            })
            .collect()
    }
    #[test]
    fn split_and_late_terminal_mode_replies_never_become_draft_text() {
        let mut d = KeyboardDecoder::default();
        assert!(feed(&mut d, "\x1b[?2026;").is_empty());
        assert!(d.pending());
        assert!(feed(&mut d, "2$y").is_empty());
        assert_eq!(d.take_mode_replies(), [(2026, 2)]);
        assert!(feed(&mut d, "\x1b[?12;1$y").is_empty());
        assert_eq!(d.take_mode_replies(), [(12, 1)]);
        assert_eq!(
            feed(&mut d, "x"),
            [KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)]
        );
        assert!(!d.pending());
    }
    #[test]
    fn reply_shaped_text_in_paste_is_preserved() {
        let mut d = KeyboardDecoder::default();
        assert!(feed(&mut d, "\x1b[200~\x1b[?12;1$y\x1b[201~").is_empty());
        assert_eq!(d.take_pastes(), ["\x1b[?12;1$y"]);
        assert!(d.take_mode_replies().is_empty());
    }
}

#[cfg(test)]
mod burst_tests {
    use super::*;
    #[test]
    fn one_thousand_instant_mouse_packets_decode_without_any_text() {
        let mut decoder = KeyboardDecoder::default();
        let mut mouse = 0;
        for i in 0..1000 {
            let packet = format!("\x1b[<35;{};1M", 100 + i % 10);
            for ch in packet.chars() {
                let code = if ch == '\x1b' {
                    KeyCode::Esc
                } else {
                    KeyCode::Char(ch)
                };
                assert!(
                    decoder
                        .feed(KeyEvent::new(code, KeyModifiers::NONE))
                        .is_empty()
                );
            }
            mouse += decoder.take_mouse().len();
        }
        assert_eq!(mouse, 1000);
        assert!(!decoder.pending());
        assert!(decoder.expire().is_empty());
    }
}
