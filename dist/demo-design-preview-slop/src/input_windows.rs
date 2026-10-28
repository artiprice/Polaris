//! Read native records without discarding terminal control characters.
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io, mem,
    os::windows::io::AsRawHandle,
    time::{Duration, Instant},
};
use winapi::um::{
    consoleapi::ReadConsoleInputW,
    synchapi::WaitForSingleObject,
    wincon::{self, INPUT_RECORD, KEY_EVENT, KEY_EVENT_RECORD, WINDOW_BUFFER_SIZE_EVENT},
};

pub struct TerminalInput {
    console: File,
    original_mode: u32,
    mouse_buttons: u32,
    pending: VecDeque<Event>,
    surrogate: Option<u16>,
}
impl TerminalInput {
    pub fn new() -> io::Result<Self> {
        use winapi::um::{
            consoleapi::{GetConsoleMode, SetConsoleMode},
            wincon::{ENABLE_EXTENDED_FLAGS, ENABLE_MOUSE_INPUT, ENABLE_QUICK_EDIT_MODE},
        };
        let console = OpenOptions::new().read(true).write(true).open("CONIN$")?;
        let mut original_mode = 0;
        // SAFETY: the owned console handle is live and original_mode is writable.
        unsafe {
            if GetConsoleMode(console.as_raw_handle().cast(), &mut original_mode) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mode = (original_mode | ENABLE_MOUSE_INPUT | ENABLE_EXTENDED_FLAGS)
                & !ENABLE_QUICK_EDIT_MODE;
            if SetConsoleMode(console.as_raw_handle().cast(), mode) == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Self {
            console,
            original_mode,
            mouse_buttons: 0,
            pending: VecDeque::new(),
            surrogate: None,
        })
    }
    pub fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        if !self.pending.is_empty() {
            return Ok(true);
        }
        let start = Instant::now();
        let mut ignored = 0;
        loop {
            let remaining = timeout.saturating_sub(start.elapsed());
            let millis = remaining.as_millis().min(u32::MAX as u128 - 1) as u32;
            // SAFETY: the File owns this live console handle for the call.
            let status =
                unsafe { WaitForSingleObject(self.console.as_raw_handle().cast(), millis) };
            match status {
                258 => return Ok(false),
                0 => {}
                _ => return Err(io::Error::last_os_error()),
            }
            let mut records: [INPUT_RECORD; 128] = unsafe { mem::zeroed() };
            let mut count = 0;
            // SAFETY: records and count are valid output buffers. After the wait,
            // ReadConsoleInputW returns the currently queued records without
            // waiting for the whole array to fill.
            if unsafe {
                ReadConsoleInputW(
                    self.console.as_raw_handle().cast(),
                    records.as_mut_ptr(),
                    records.len() as u32,
                    &mut count,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            for record in records.iter().take(count as usize) {
                match record.EventType {
                    KEY_EVENT => {
                        // SAFETY: EventType identifies the active KEY_EVENT_RECORD union member.
                        let key = unsafe { *record.Event.KeyEvent() };
                        if let Some(decoded) = translate_key(key, &mut self.surrogate) {
                            for _ in 0..key.wRepeatCount.max(1) {
                                self.pending.push_back(Event::Key(decoded));
                            }
                        }
                    }
                    wincon::MOUSE_EVENT => {
                        // SAFETY: EventType identifies the active MOUSE_EVENT_RECORD union member.
                        let mouse = unsafe { *record.Event.MouseEvent() };
                        self.pending.extend(
                            translate_mouse(mouse, &mut self.mouse_buttons)
                                .into_iter()
                                .map(Event::Mouse),
                        );
                    }
                    WINDOW_BUFFER_SIZE_EVENT => {
                        // SAFETY: EventType identifies the active resize union member.
                        let size = unsafe { record.Event.WindowBufferSizeEvent().dwSize };
                        self.pending
                            .push_back(Event::Resize(size.X.max(0) as u16, size.Y.max(0) as u16));
                    }
                    _ => {}
                }
            }
            if !self.pending.is_empty() {
                return Ok(true);
            }
            // A zero-time poll must look past releases and other ignored records.
            // Otherwise batching stops after every physical key release.
            ignored += 1;
            if ignored >= 256 {
                return Ok(false);
            }
        }
    }
    pub fn read(&mut self) -> io::Result<Event> {
        self.pending
            .pop_front()
            .ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock, "poll input before read"))
    }
}

impl Drop for TerminalInput {
    fn drop(&mut self) {
        // SAFETY: self.console owns this live handle; restore the exact mode captured in new().
        unsafe {
            winapi::um::consoleapi::SetConsoleMode(
                self.console.as_raw_handle().cast(),
                self.original_mode,
            );
        }
    }
}

fn translate_mouse(mouse: wincon::MOUSE_EVENT_RECORD, buttons: &mut u32) -> Vec<MouseEvent> {
    use wincon::{
        FROM_LEFT_1ST_BUTTON_PRESSED, FROM_LEFT_2ND_BUTTON_PRESSED, MOUSE_HWHEELED, MOUSE_MOVED,
        MOUSE_WHEELED, RIGHTMOST_BUTTON_PRESSED,
    };
    let state = mouse.dwButtonState;
    let changed_down = state & !*buttons;
    let changed_up = *buttons & !state;
    *buttons = state;
    let mut modifiers = KeyModifiers::NONE;
    if mouse.dwControlKeyState & 0x10 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if mouse.dwControlKeyState & 0x03 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if mouse.dwControlKeyState & 0x0c != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    let column = mouse.dwMousePosition.X.max(0) as u16;
    let row = mouse.dwMousePosition.Y.max(0) as u16;
    let mut events = Vec::new();
    let button_events = [
        (FROM_LEFT_1ST_BUTTON_PRESSED, MouseButton::Left),
        (RIGHTMOST_BUTTON_PRESSED, MouseButton::Right),
        (FROM_LEFT_2ND_BUTTON_PRESSED, MouseButton::Middle),
    ];
    if mouse.dwEventFlags == MOUSE_WHEELED || mouse.dwEventFlags == MOUSE_HWHEELED {
        let delta = (state >> 16) as i16;
        if delta != 0 {
            let kind = match (mouse.dwEventFlags, delta > 0) {
                (MOUSE_WHEELED, true) => MouseEventKind::ScrollUp,
                (MOUSE_WHEELED, false) => MouseEventKind::ScrollDown,
                (MOUSE_HWHEELED, true) => MouseEventKind::ScrollRight,
                _ => MouseEventKind::ScrollLeft,
            };
            events.push(MouseEvent {
                kind,
                column,
                row,
                modifiers,
            });
        }
    } else if mouse.dwEventFlags == MOUSE_MOVED {
        let held = button_events
            .iter()
            .find(|(flag, _)| state & *flag != 0)
            .map(|(_, button)| *button);
        let kind = held
            .map(MouseEventKind::Drag)
            .unwrap_or(MouseEventKind::Moved);
        events.push(MouseEvent {
            kind,
            column,
            row,
            modifiers,
        });
    } else {
        for (flag, button) in button_events {
            if changed_down & flag != 0 {
                events.push(MouseEvent {
                    kind: MouseEventKind::Down(button),
                    column,
                    row,
                    modifiers,
                });
            }
            if changed_up & flag != 0 {
                events.push(MouseEvent {
                    kind: MouseEventKind::Up(button),
                    column,
                    row,
                    modifiers,
                });
            }
        }
    }
    events
}

fn translate_key(mut key: KEY_EVENT_RECORD, surrogate: &mut Option<u16>) -> Option<KeyEvent> {
    // ConPTY can synthesize printable Unicode (including @) as an Alt-code
    // release. It is committed text, unlike an ordinary physical key release.
    if key.bKeyDown == 0 {
        if key.wVirtualKeyCode == 0x12 && unsafe { *key.uChar.UnicodeChar() } != 0 {
            key.wVirtualKeyCode = 0;
            key.dwControlKeyState = 0;
        } else {
            return None;
        }
    }
    let mut modifiers = KeyModifiers::NONE;
    if key.dwControlKeyState & 0x10 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if key.dwControlKeyState & 0x03 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if key.dwControlKeyState & 0x0c != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    let code = match key.wVirtualKeyCode {
        0x08 => KeyCode::Backspace,
        0x09 => KeyCode::Tab,
        0x0d => KeyCode::Enter,
        0x1b => KeyCode::Esc,
        0x10..=0x12 => return None,
        0x21 => KeyCode::PageUp,
        0x22 => KeyCode::PageDown,
        0x23 => KeyCode::End,
        0x24 => KeyCode::Home,
        0x25 => KeyCode::Left,
        0x26 => KeyCode::Up,
        0x27 => KeyCode::Right,
        0x28 => KeyCode::Down,
        0x2d => KeyCode::Insert,
        0x2e => KeyCode::Delete,
        0x70..=0x87 => KeyCode::F((key.wVirtualKeyCode - 0x6f) as u8),
        _ => {
            // SAFETY: KEY_EVENT_RECORD's UnicodeChar is the UTF-16 input field.
            let unit = unsafe { *key.uChar.UnicodeChar() };
            match unit {
                0 if modifiers.contains(KeyModifiers::CONTROL) => KeyCode::Char(' '),
                0 => return None,
                8 | 127 => KeyCode::Backspace,
                9 => KeyCode::Tab,
                13 => KeyCode::Enter,
                27 => KeyCode::Esc,
                10 => KeyCode::Char('\n'),
                1..=26 => {
                    modifiers |= KeyModifiers::CONTROL;
                    KeyCode::Char(char::from_u32(u32::from(unit) + 96)?)
                }
                0xd800..=0xdbff => {
                    *surrogate = Some(unit);
                    return None;
                }
                0xdc00..=0xdfff => {
                    let high = surrogate.take()?;
                    let ch = char::from_u32(
                        0x10000 + ((u32::from(high) - 0xd800) << 10) + u32::from(unit) - 0xdc00,
                    )?;
                    KeyCode::Char(ch)
                }
                unit => {
                    *surrogate = None;
                    KeyCode::Char(char::from_u32(unit.into())?)
                }
            }
        }
    };
    *surrogate = None;
    Some(super::normalize_key(KeyEvent::new(code, modifiers), false))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(vk: u16, unit: u16, state: u32) -> KEY_EVENT_RECORD {
        let mut record: KEY_EVENT_RECORD = unsafe { mem::zeroed() };
        record.bKeyDown = 1;
        record.wRepeatCount = 1;
        record.wVirtualKeyCode = vk;
        record.dwControlKeyState = state;
        unsafe {
            *record.uChar.UnicodeChar_mut() = unit;
        }
        record
    }
    #[test]
    fn native_mouse_records_report_press_drag_release_and_wheel() {
        use wincon::{FROM_LEFT_1ST_BUTTON_PRESSED, MOUSE_MOVED, MOUSE_WHEELED};
        let mut buttons = 0;
        let mut down: wincon::MOUSE_EVENT_RECORD = unsafe { mem::zeroed() };
        down.dwMousePosition.X = 4;
        down.dwMousePosition.Y = 7;
        down.dwButtonState = FROM_LEFT_1ST_BUTTON_PRESSED;
        let e = translate_mouse(down, &mut buttons);
        assert_eq!(
            e,
            [MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 4,
                row: 7,
                modifiers: KeyModifiers::NONE
            }]
        );
        let mut moved = down;
        moved.dwMousePosition.X = 8;
        moved.dwEventFlags = MOUSE_MOVED;
        assert_eq!(
            translate_mouse(moved, &mut buttons)[0].kind,
            MouseEventKind::Drag(MouseButton::Left)
        );
        let mut up = down;
        up.dwButtonState = 0;
        assert_eq!(
            translate_mouse(up, &mut buttons)[0].kind,
            MouseEventKind::Up(MouseButton::Left)
        );
        let mut wheel: wincon::MOUSE_EVENT_RECORD = unsafe { mem::zeroed() };
        wheel.dwButtonState = (120u32) << 16;
        wheel.dwEventFlags = MOUSE_WHEELED;
        assert_eq!(
            translate_mouse(wheel, &mut buttons)[0].kind,
            MouseEventKind::ScrollUp
        );
    }

    #[test]
    fn control_characters_with_no_virtual_key_are_preserved() {
        for (unit, code) in [(13, KeyCode::Enter), (27, KeyCode::Esc), (9, KeyCode::Tab)] {
            assert_eq!(
                translate_key(record(0, unit, 0), &mut None),
                Some(KeyEvent::new(code, KeyModifiers::NONE))
            );
        }
        assert_eq!(
            translate_key(record(0, 9, 0x10), &mut None).unwrap().code,
            KeyCode::BackTab
        );
        assert_eq!(
            translate_key(record(0, 3, 0), &mut None),
            Some(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
        );
    }

    #[test]
    fn alt_code_release_commits_unicode_without_replaying_ordinary_releases() {
        for unit in [64, 169, 0x0410] {
            let mut committed = record(0x12, unit, 0);
            committed.bKeyDown = 0;
            assert_eq!(
                translate_key(committed, &mut None),
                Some(KeyEvent::new(
                    KeyCode::Char(char::from_u32(unit.into()).unwrap()),
                    KeyModifiers::NONE
                ))
            );
        }
        let mut ordinary = record(0x41, 65, 0);
        ordinary.bKeyDown = 0;
        assert!(translate_key(ordinary, &mut None).is_none());
        let mut modifier = record(0x12, 0, 0);
        modifier.bKeyDown = 0;
        assert!(translate_key(modifier, &mut None).is_none());
        assert!(translate_key(record(0x66, 0, 2), &mut None).is_none());
    }
    #[test]
    fn native_keys_and_utf16_surrogates_keep_their_meaning() {
        assert_eq!(
            translate_key(record(0x0d, 13, 0x10), &mut None).unwrap(),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)
        );
        let mut pending = None;
        assert!(translate_key(record(0, 0xd83e, 0), &mut pending).is_none());
        assert_eq!(
            translate_key(record(0, 0xdd16, 0), &mut pending)
                .unwrap()
                .code,
            KeyCode::Char('🤖')
        );
        assert!(translate_key(record(0, 0xd83e, 0), &mut pending).is_none());
        assert_eq!(
            translate_key(record(0x0d, 13, 0), &mut pending)
                .unwrap()
                .code,
            KeyCode::Enter
        );
        assert!(translate_key(record(0, 0xdd16, 0), &mut pending).is_none());
        let mut release = record(0x0d, 13, 0);
        release.bKeyDown = 0;
        assert!(translate_key(release, &mut None).is_none());
    }
}
