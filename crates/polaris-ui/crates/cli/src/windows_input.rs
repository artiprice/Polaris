//! Bounded, safe Windows console input through Crossterm's Win32 wrappers.
use crate::{
    bracketed_paste::BracketedPasteDecoder,
    input::{InputStopToken, NormalizedInput, ShiftEnterCapability, TerminalInput},
    input_decoder::{KeyDecoder, NativeKeyRecord},
};
use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent as CrosstermMouseEvent, MouseEventKind,
};
use crossterm_winapi::{
    Console, ConsoleMode, ControlKeyState, EventFlags, Handle, HandleType, InputRecord,
};
use std::{
    io, thread,
    time::{Duration, Instant},
};

const ENABLE_ECHO_INPUT: u32 = 0x0004;
const ENABLE_LINE_INPUT: u32 = 0x0002;
const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
const ENABLE_MOUSE_INPUT: u32 = 0x0010;
const ENABLE_WINDOW_INPUT: u32 = 0x0008;
const ENABLE_QUICK_EDIT_MODE: u32 = 0x0040;
const ENABLE_EXTENDED_FLAGS: u32 = 0x0080;
const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
const SHIFT_PRESSED: u32 = 0x0010;
const LEFT_CTRL_PRESSED: u32 = 0x0008;
const RIGHT_CTRL_PRESSED: u32 = 0x0004;
const LEFT_ALT_PRESSED: u32 = 0x0002;
const RIGHT_ALT_PRESSED: u32 = 0x0001;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MouseButtons {
    left: bool,
    middle: bool,
    right: bool,
}

impl From<crossterm_winapi::ButtonState> for MouseButtons {
    fn from(buttons: crossterm_winapi::ButtonState) -> Self {
        Self {
            left: buttons.left_button(),
            middle: buttons.middle_button(),
            right: buttons.right_button(),
        }
    }
}

/// Reads native Windows console events without racing Crossterm's event reader.
#[derive(Debug)]
pub struct WindowsTerminalInput {
    console: Console,
    mode: ConsoleMode,
    original_mode: Option<u32>,
    stop: InputStopToken,
    decoder: KeyDecoder,
    paste_decoder: BracketedPasteDecoder,
    mouse_buttons: MouseButtons,
}

impl WindowsTerminalInput {
    /// Opens the standard input console, enables virtual-terminal and mouse input, and saves its exact mode.
    pub fn new() -> io::Result<Self> {
        let handle = Handle::new(HandleType::InputHandle)?;
        let mode = ConsoleMode::from(handle.clone());
        let original_mode = mode.mode()?;
        configure_native_input_mode(&mode, original_mode)?;
        Ok(Self {
            console: Console::from(handle),
            mode,
            original_mode: Some(original_mode),
            stop: InputStopToken::default(),
            decoder: KeyDecoder::default(),
            paste_decoder: BracketedPasteDecoder::default(),
            mouse_buttons: MouseButtons::default(),
        })
    }

    /// Reads the current standard input console mode without changing it.
    ///
    /// This exposes the Win32 mode bits for diagnostics that must verify exact mode restoration.
    /// # Errors
    /// Returns an error if the input console handle or its mode cannot be read.
    pub fn console_input_mode_bits() -> io::Result<u32> {
        let handle = Handle::new(HandleType::InputHandle)?;
        ConsoleMode::from(handle).mode()
    }

    fn read_available(&mut self) -> io::Result<Option<NormalizedInput>> {
        if self.console.number_of_console_input_events()? == 0 {
            return Ok(None);
        }
        let record = self.console.read_single_input_event()?;
        let event = match record {
            InputRecord::KeyEvent(key) => {
                let native = NativeKeyRecord {
                    key_down: key.key_down,
                    repeat_count: key.repeat_count,
                    virtual_key_code: key.virtual_key_code,
                    unicode_char: key.u_char,
                    control_key_state: control_state_bits(key.control_key_state),
                };
                self.decoder.decode(native).map(|key| {
                    trace_key_category(&key);
                    NormalizedInput::Key(key)
                })
            }
            InputRecord::MouseEvent(mouse) => mouse_kind(
                mouse.event_flags,
                mouse.button_state,
                &mut self.mouse_buttons,
            )
            .map(|kind| {
                NormalizedInput::Mouse(CrosstermMouseEvent {
                    kind,
                    column: console_coordinate(mouse.mouse_position.x),
                    row: console_coordinate(mouse.mouse_position.y),
                    modifiers: key_modifiers(mouse.control_key_state),
                })
            }),
            InputRecord::WindowBufferSizeEvent(size) => Some(NormalizedInput::Resize {
                columns: console_coordinate(size.size.x),
                rows: console_coordinate(size.size.y),
            }),
            InputRecord::FocusEvent(focus) => Some(NormalizedInput::Focus {
                focused: focus.set_focus,
            }),
            InputRecord::MenuEvent(_) => None,
        };
        Ok(event)
    }
}

impl TerminalInput for WindowsTerminalInput {
    fn poll(&mut self, timeout: Duration) -> io::Result<Option<NormalizedInput>> {
        let started = Instant::now();
        loop {
            if self.stop.is_stopped() {
                return Ok(None);
            }
            if let Some(event) = self.paste_decoder.pop() {
                return Ok(Some(event));
            }
            if let Some(event) = self.read_available()? {
                self.paste_decoder
                    .push(event)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                if let Some(ready) = self.paste_decoder.pop() {
                    return Ok(Some(ready));
                }
                continue;
            }
            self.paste_decoder.flush_expired_escape();
            if let Some(event) = self.paste_decoder.pop() {
                return Ok(Some(event));
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Ok(None);
            }
            thread::sleep(remaining.min(crate::input::NATIVE_POLL_QUANTUM));
        }
    }

    fn shift_enter_capability(&self) -> ShiftEnterCapability {
        ShiftEnterCapability::Native
    }

    fn stop_token(&self) -> InputStopToken {
        self.stop.clone()
    }

    fn close(&mut self) -> io::Result<()> {
        self.stop.stop();
        restore_input_mode_snapshot(&self.mode, &mut self.original_mode)
    }
}

impl Drop for WindowsTerminalInput {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            tracing::error!(%error, "failed to restore original Windows console input mode");
        }
    }
}

trait InputModeAccess {
    fn mode(&self) -> io::Result<u32>;
    fn set_mode(&self, mode: u32) -> io::Result<()>;
}

impl InputModeAccess for ConsoleMode {
    fn mode(&self) -> io::Result<u32> {
        ConsoleMode::mode(self)
    }

    fn set_mode(&self, mode: u32) -> io::Result<()> {
        ConsoleMode::set_mode(self, mode)
    }
}

fn requested_native_input_mode(original: u32) -> u32 {
    (original
        & !(ENABLE_ECHO_INPUT
            | ENABLE_LINE_INPUT
            | ENABLE_PROCESSED_INPUT
            | ENABLE_QUICK_EDIT_MODE))
        | ENABLE_VIRTUAL_TERMINAL_INPUT
        | ENABLE_MOUSE_INPUT
        | ENABLE_WINDOW_INPUT
        | ENABLE_EXTENDED_FLAGS
}

fn native_input_mode_is_ready(actual: u32) -> bool {
    actual & ENABLE_VIRTUAL_TERMINAL_INPUT != 0
        && actual & ENABLE_MOUSE_INPUT != 0
        && actual & ENABLE_WINDOW_INPUT != 0
        && actual & ENABLE_EXTENDED_FLAGS != 0
        && actual & ENABLE_QUICK_EDIT_MODE == 0
        && actual & (ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT) == 0
}

fn restore_native_input_mode(mode: &impl InputModeAccess, original: u32) -> io::Result<()> {
    mode.set_mode(original)?;
    let actual = mode.mode()?;
    if actual != original {
        return Err(io::Error::other(format!(
            "Windows console input mode restoration mismatch: expected {original:#010x}, observed {actual:#010x}"
        )));
    }
    Ok(())
}

fn restore_input_mode_snapshot(
    mode: &impl InputModeAccess,
    original: &mut Option<u32>,
) -> io::Result<()> {
    if let Some(snapshot) = *original {
        restore_native_input_mode(mode, snapshot)?;
        *original = None;
    }
    Ok(())
}

fn restore_mode_after_failure(
    mode: &impl InputModeAccess,
    original: u32,
    cause: io::Error,
) -> io::Error {
    match restore_native_input_mode(mode, original) {
        Ok(()) => cause,
        Err(restore) => io::Error::new(
            cause.kind(),
            format!(
                "{cause}; restoring and verifying original console input mode failed: {restore}"
            ),
        ),
    }
}

fn configure_native_input_mode(mode: &impl InputModeAccess, original: u32) -> io::Result<()> {
    let requested = requested_native_input_mode(original);
    if let Err(error) = mode.set_mode(requested) {
        return Err(restore_mode_after_failure(mode, original, error));
    }
    match mode.mode() {
        Ok(actual) if native_input_mode_is_ready(actual) => Ok(()),
        Ok(actual) => Err(restore_mode_after_failure(
            mode,
            original,
            io::Error::other(format!(
                "Windows console rejected required input mode bits: requested {requested:#010x}, observed {actual:#010x}"
            )),
        )),
        Err(error) => Err(restore_mode_after_failure(mode, original, error)),
    }
}

fn control_state_bits(state: ControlKeyState) -> u32 {
    let mut bits = 0;
    for flag in [
        SHIFT_PRESSED,
        LEFT_CTRL_PRESSED,
        RIGHT_CTRL_PRESSED,
        LEFT_ALT_PRESSED,
        RIGHT_ALT_PRESSED,
    ] {
        if state.has_state(flag) {
            bits |= flag;
        }
    }
    bits
}

fn key_modifiers(state: ControlKeyState) -> KeyModifiers {
    let bits = control_state_bits(state);
    let mut modifiers = KeyModifiers::empty();
    if bits & SHIFT_PRESSED != 0 {
        modifiers.insert(KeyModifiers::SHIFT);
    }
    if bits & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0 {
        modifiers.insert(KeyModifiers::CONTROL);
    }
    if bits & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0 {
        modifiers.insert(KeyModifiers::ALT);
    }
    modifiers
}

fn trace_key_category(key: &KeyEvent) {
    let category = match key.code {
        KeyCode::Char(_) => "character",
        KeyCode::Enter => "enter",
        KeyCode::Backspace => "backspace",
        KeyCode::Delete => "delete",
        KeyCode::Left
        | KeyCode::Right
        | KeyCode::Up
        | KeyCode::Down
        | KeyCode::Home
        | KeyCode::End => "navigation",
        _ => "other",
    };
    tracing::trace!(
        category,
        shift = key.modifiers.contains(KeyModifiers::SHIFT),
        control = key.modifiers.contains(KeyModifiers::CONTROL),
        alt = key.modifiers.contains(KeyModifiers::ALT),
        phase = ?key.kind,
        "normalized Windows key event"
    );
}

fn console_coordinate(value: i16) -> u16 {
    u16::try_from(value).unwrap_or_default()
}

fn mouse_kind(
    flags: EventFlags,
    buttons: crossterm_winapi::ButtonState,
    previous: &mut MouseButtons,
) -> Option<MouseEventKind> {
    let current = MouseButtons::from(buttons);
    let last = *previous;
    *previous = current;

    match flags {
        EventFlags::PressOrRelease | EventFlags::DoubleClick => {
            let left_changed = last.left != current.left;
            let middle_changed = last.middle != current.middle;
            let right_changed = last.right != current.right;
            let transition = match (left_changed, middle_changed, right_changed) {
                (true, false, false) => Some((current.left, MouseButton::Left)),
                (false, true, false) => Some((current.middle, MouseButton::Middle)),
                (false, false, true) => Some((current.right, MouseButton::Right)),
                _ => None,
            };
            let (pressed, button) = transition?;
            Some(if pressed {
                MouseEventKind::Down(button)
            } else {
                MouseEventKind::Up(button)
            })
        }
        EventFlags::MouseMoved => Some(match (current.left, current.middle, current.right) {
            (true, false, false) => MouseEventKind::Drag(MouseButton::Left),
            (false, true, false) => MouseEventKind::Drag(MouseButton::Middle),
            (false, false, true) => MouseEventKind::Drag(MouseButton::Right),
            // Crossterm's MouseEventKind carries one drag button, so preserve motion
            // without assigning an ambiguous multi-button chord to one of them.
            _ => MouseEventKind::Moved,
        }),
        EventFlags::MouseWheeled if buttons.scroll_up() => Some(MouseEventKind::ScrollUp),
        EventFlags::MouseWheeled => Some(MouseEventKind::ScrollDown),
        EventFlags::MouseHwheeled if buttons.scroll_up() => Some(MouseEventKind::ScrollRight),
        EventFlags::MouseHwheeled => Some(MouseEventKind::ScrollLeft),
        EventFlags::Unknown => Some(MouseEventKind::Moved),
    }
}

#[cfg(test)]
#[path = "../../../../../tests/crates/polaris-ui/crates/cli/src/windows_input.rs"]
mod tests;
