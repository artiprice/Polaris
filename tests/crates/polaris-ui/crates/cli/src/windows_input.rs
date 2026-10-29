// Unit tests of crates/polaris-ui/crates/cli/src/windows_input.rs (`tests`); compiled into that module via #[path].
use super::{
    ENABLE_ECHO_INPUT, ENABLE_EXTENDED_FLAGS, ENABLE_LINE_INPUT, ENABLE_MOUSE_INPUT,
    ENABLE_PROCESSED_INPUT, ENABLE_QUICK_EDIT_MODE, ENABLE_VIRTUAL_TERMINAL_INPUT,
    ENABLE_WINDOW_INPUT, InputModeAccess, MouseButtons, configure_native_input_mode, mouse_kind,
    requested_native_input_mode, restore_input_mode_snapshot, restore_native_input_mode,
};
use crossterm::event::{MouseButton, MouseEventKind};
use crossterm_winapi::{ButtonState, EventFlags};
use std::{cell::Cell, cell::RefCell, io};

#[derive(Debug)]
struct FakeMode {
    original: u32,
    current: Cell<u32>,
    calls: RefCell<Vec<u32>>,
    fail_next_set: Cell<bool>,
    reject_mouse: bool,
}

impl FakeMode {
    fn new(original: u32) -> Self {
        Self {
            original,
            current: Cell::new(original),
            calls: RefCell::new(Vec::new()),
            fail_next_set: Cell::new(false),
            reject_mouse: false,
        }
    }
}

impl InputModeAccess for FakeMode {
    fn mode(&self) -> io::Result<u32> {
        let current = self.current.get();
        if self.reject_mouse && current != self.original {
            Ok(current & !ENABLE_MOUSE_INPUT)
        } else {
            Ok(current)
        }
    }

    fn set_mode(&self, mode: u32) -> io::Result<()> {
        self.calls.borrow_mut().push(mode);
        self.current.set(mode);
        if self.fail_next_set.replace(false) {
            return Err(io::Error::other("injected set failure"));
        }
        Ok(())
    }
}

#[test]
fn requested_native_mode_enables_mouse_window_and_vt_without_quick_edit() {
    let unrelated = 0x4000;
    let original = unrelated
        | ENABLE_ECHO_INPUT
        | ENABLE_LINE_INPUT
        | ENABLE_PROCESSED_INPUT
        | ENABLE_QUICK_EDIT_MODE;
    let requested = requested_native_input_mode(original);
    assert_ne!(requested & ENABLE_MOUSE_INPUT, 0);
    assert_ne!(requested & ENABLE_WINDOW_INPUT, 0);
    assert_ne!(requested & ENABLE_EXTENDED_FLAGS, 0);
    assert_ne!(requested & ENABLE_VIRTUAL_TERMINAL_INPUT, 0);
    assert_eq!(requested & ENABLE_QUICK_EDIT_MODE, 0);
    assert_eq!(
        requested & (ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT),
        0
    );
    assert_ne!(requested & unrelated, 0);
}

#[test]
fn partial_native_mode_set_failure_restores_exact_snapshot() {
    let original = ENABLE_QUICK_EDIT_MODE | ENABLE_ECHO_INPUT | 0x4000;
    let mode = FakeMode::new(original);
    mode.fail_next_set.set(true);
    let error = configure_native_input_mode(&mode, original)
        .err()
        .map(|error| error.to_string());
    assert!(error.is_some());
    assert_eq!(mode.current.get(), original);
    assert_eq!(mode.calls.borrow().len(), 2);
}

#[test]
fn native_mode_readback_rejection_restores_exact_snapshot() {
    let original = ENABLE_QUICK_EDIT_MODE | 0x4000;
    let mut mode = FakeMode::new(original);
    mode.reject_mouse = true;
    assert!(configure_native_input_mode(&mode, original).is_err());
    assert_eq!(mode.current.get(), original);
    assert_eq!(mode.calls.borrow().len(), 2);
}

#[test]
fn explicit_native_close_verifies_exact_mode_restoration() {
    let original = ENABLE_QUICK_EDIT_MODE | ENABLE_ECHO_INPUT | 0x4000;
    let mode = FakeMode::new(original);
    mode.current.set(0x1234);
    assert!(restore_native_input_mode(&mode, original).is_ok());
    assert_eq!(mode.current.get(), original);
}

#[test]
fn explicit_close_then_guard_restore_then_drop_keeps_initial_mode() {
    let original = ENABLE_QUICK_EDIT_MODE | ENABLE_ECHO_INPUT | 0x4000;
    let raw_snapshot = 0x01f0;
    let mode = FakeMode::new(original);
    let mut reader_snapshot = Some(raw_snapshot);
    assert!(mode.set_mode(raw_snapshot).is_ok());
    assert!(restore_input_mode_snapshot(&mode, &mut reader_snapshot).is_ok());
    assert_eq!(reader_snapshot, None);

    // TerminalGuard restores the state it captured before the reader was created.
    assert!(restore_native_input_mode(&mode, original).is_ok());
    // WindowsTerminalInput::drop calls close again; a disarmed snapshot must be a no-op.
    assert!(restore_input_mode_snapshot(&mode, &mut reader_snapshot).is_ok());
    assert_eq!(mode.current.get(), original);
    assert_eq!(mode.calls.borrow().len(), 3);
}

#[test]
fn failed_close_keeps_snapshot_armed_for_a_verified_retry() {
    let original = 0x4000;
    let snapshot = ENABLE_MOUSE_INPUT | ENABLE_WINDOW_INPUT | ENABLE_EXTENDED_FLAGS;
    let mut mode = FakeMode::new(original);
    let mut reader_snapshot = Some(snapshot);
    mode.current.set(snapshot);
    mode.reject_mouse = true;
    assert!(restore_input_mode_snapshot(&mode, &mut reader_snapshot).is_err());
    assert_eq!(reader_snapshot, Some(snapshot));

    mode.reject_mouse = false;
    assert!(restore_input_mode_snapshot(&mode, &mut reader_snapshot).is_ok());
    assert_eq!(reader_snapshot, None);
    assert_eq!(mode.current.get(), snapshot);
}

#[test]
fn native_mouse_motion_with_pressed_button_is_drag() {
    for (state, expected) in [
        (0x0001, MouseButton::Left),
        (0x0004, MouseButton::Middle),
        (0x0002, MouseButton::Right),
    ] {
        assert_eq!(
            mouse_kind(
                EventFlags::MouseMoved,
                ButtonState::from(state),
                &mut MouseButtons::default(),
            ),
            Some(MouseEventKind::Drag(expected))
        );
    }
    assert_eq!(
        mouse_kind(
            EventFlags::MouseMoved,
            ButtonState::from(0),
            &mut MouseButtons::default(),
        ),
        Some(MouseEventKind::Moved)
    );
}

#[test]
fn native_right_button_down_and_release_keep_identity() {
    let mut previous = MouseButtons::default();
    assert_eq!(
        mouse_kind(
            EventFlags::PressOrRelease,
            ButtonState::from(0x0002),
            &mut previous,
        ),
        Some(MouseEventKind::Down(MouseButton::Right))
    );
    assert_eq!(
        mouse_kind(
            EventFlags::PressOrRelease,
            ButtonState::from(0),
            &mut previous,
        ),
        Some(MouseEventKind::Up(MouseButton::Right))
    );
}

#[test]
fn native_middle_button_down_and_release_keep_identity() {
    let mut previous = MouseButtons::default();
    assert_eq!(
        mouse_kind(
            EventFlags::PressOrRelease,
            ButtonState::from(0x0004),
            &mut previous,
        ),
        Some(MouseEventKind::Down(MouseButton::Middle))
    );
    assert_eq!(
        mouse_kind(
            EventFlags::PressOrRelease,
            ButtonState::from(0),
            &mut previous,
        ),
        Some(MouseEventKind::Up(MouseButton::Middle))
    );
}

#[test]
fn native_button_changes_keep_left_drag_and_other_button_transitions() {
    let mut previous = MouseButtons::default();
    assert_eq!(
        mouse_kind(
            EventFlags::PressOrRelease,
            ButtonState::from(0x0001),
            &mut previous,
        ),
        Some(MouseEventKind::Down(MouseButton::Left))
    );
    assert_eq!(
        mouse_kind(
            EventFlags::MouseMoved,
            ButtonState::from(0x0001),
            &mut previous,
        ),
        Some(MouseEventKind::Drag(MouseButton::Left))
    );
    assert_eq!(
        mouse_kind(
            EventFlags::PressOrRelease,
            ButtonState::from(0x0003),
            &mut previous,
        ),
        Some(MouseEventKind::Down(MouseButton::Right))
    );
    assert_eq!(
        mouse_kind(
            EventFlags::PressOrRelease,
            ButtonState::from(0x0002),
            &mut previous,
        ),
        Some(MouseEventKind::Up(MouseButton::Left))
    );
    assert_eq!(
        mouse_kind(
            EventFlags::PressOrRelease,
            ButtonState::from(0),
            &mut previous,
        ),
        Some(MouseEventKind::Up(MouseButton::Right))
    );
}

#[test]
fn simultaneous_mouse_button_changes_do_not_guess_chord_order() {
    let mut previous = MouseButtons::default();
    assert_eq!(
        mouse_kind(
            EventFlags::PressOrRelease,
            ButtonState::from(0x0003),
            &mut previous,
        ),
        None
    );
    assert_eq!(
        mouse_kind(
            EventFlags::PressOrRelease,
            ButtonState::from(0x0002),
            &mut previous,
        ),
        Some(MouseEventKind::Up(MouseButton::Left))
    );
}
