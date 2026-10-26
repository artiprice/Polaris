use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent};
use std::{io, time::Duration};
use thiserror::Error;
/// Maximum UTF-8 byte length accepted for one normalized paste.
pub const MAX_PASTE_BYTES: usize = 1024 * 1024;
/// Maximum delay between native console availability checks.
pub const NATIVE_POLL_QUANTUM: Duration = Duration::from_millis(5);
/// Crossterm events retained for application-owned semantic processing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizedInput {
    /// Keyboard event including its original modifiers and phase.
    Key(KeyEvent),
    /// Literal bracketed-paste payload.
    Paste(String),
    /// Mouse event.
    Mouse(MouseEvent),
    /// Current terminal dimensions.
    Resize { columns: u16, rows: u16 },
    /// Whether terminal focus is currently held.
    Focus { focused: bool },
}
/// A rejected terminal event with a typed cause.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum InputError {
    #[error("paste exceeds the 1 MiB normalized input limit")]
    PasteTooLarge,
}
/// Polls a terminal input source and returns at most one normalized event.
pub trait TerminalInput {
    /// Waits up to `timeout`; `None` means no event was ready before the deadline.
    fn poll(&mut self, timeout: Duration) -> io::Result<Option<NormalizedInput>>;

    /// Reports the active input source's declared Shift+Enter capability.
    ///
    /// This describes the event source, not a runtime guarantee that a terminal or ConPTY
    /// preserved the modifier. Confirm actual events at the normalized input boundary.
    fn shift_enter_capability(&self) -> ShiftEnterCapability;

    /// Returns a token that can stop a bounded wait from another thread.
    fn stop_token(&self) -> InputStopToken;

    /// Restores input resources and stops any active bounded poll.
    ///
    /// # Errors
    /// Returns an error if restoring the input source fails.
    fn close(&mut self) -> io::Result<()>;
}

/// Evidence level for the active input source's Shift+Enter handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShiftEnterCapability {
    /// Native key records carry Shift independently from Enter; the host may still collapse it.
    Native,
    /// The terminal explicitly confirmed enhanced keyboard reporting.
    EnhancedConfirmed,
    /// No confirmation has been received from the terminal.
    #[default]
    Unknown,
}

/// Thread-safe cancellation signal for a [`TerminalInput`] poll.
#[derive(Debug, Clone, Default)]
pub struct InputStopToken(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl InputStopToken {
    /// Requests that the current or next bounded poll return without waiting further.
    pub fn stop(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }

    /// Returns whether a stop has been requested.
    pub fn is_stopped(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Crossterm-backed input source used where the native console reader is unavailable.
#[cfg(not(windows))]
#[derive(Debug, Clone, Default)]
pub struct CrosstermTerminalInput {
    stop: InputStopToken,
}

#[cfg(not(windows))]
impl CrosstermTerminalInput {
    /// Creates a Crossterm input source. Keyboard enhancement is enabled separately by the terminal guard.
    pub fn new() -> io::Result<Self> {
        Ok(Self::default())
    }
}

#[cfg(not(windows))]
impl TerminalInput for CrosstermTerminalInput {
    fn poll(&mut self, timeout: Duration) -> io::Result<Option<NormalizedInput>> {
        if self.stop.is_stopped() || !crossterm::event::poll(timeout)? {
            return Ok(None);
        }
        normalize_event(crossterm::event::read()?)
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    fn shift_enter_capability(&self) -> ShiftEnterCapability {
        ShiftEnterCapability::Unknown
    }

    fn stop_token(&self) -> InputStopToken {
        self.stop.clone()
    }

    fn close(&mut self) -> io::Result<()> {
        self.stop.stop();
        Ok(())
    }
}

/// Platform-default input source.
#[cfg(windows)]
pub type InputReader = crate::windows_input::WindowsTerminalInput;
/// Platform-default input source.
#[cfg(not(windows))]
pub type InputReader = CrosstermTerminalInput;
/// Normalize a Crossterm event while preserving its payload and enforcing paste limits.
pub fn normalize_event(event: Event) -> Result<NormalizedInput, InputError> {
    match event {
        Event::Key(k) => Ok(NormalizedInput::Key(k)),
        Event::Mouse(m) => Ok(NormalizedInput::Mouse(m)),
        Event::Resize(columns, rows) => Ok(NormalizedInput::Resize { columns, rows }),
        Event::FocusGained => Ok(NormalizedInput::Focus { focused: true }),
        Event::FocusLost => Ok(NormalizedInput::Focus { focused: false }),
        Event::Paste(s) if s.len() <= MAX_PASTE_BYTES => Ok(NormalizedInput::Paste(s)),
        Event::Paste(_) => Err(InputError::PasteTooLarge),
    }
}
impl NormalizedInput {
    /// Returns whether this event may use the reserved cancellation lane.
    pub fn is_cancel_priority(&self) -> bool {
        match self {
            Self::Key(k) if matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                k.code == KeyCode::Esc
                    || (matches!(k.code, KeyCode::Char('c') | KeyCode::Char('C'))
                        && k.modifiers.contains(KeyModifiers::CONTROL))
            }
            _ => false,
        }
    }
    /// Returns payload bytes retained by the event.
    pub fn retained_payload_bytes(&self) -> usize {
        match self {
            Self::Paste(s) => s.len(),
            _ => 0,
        }
    }
}
