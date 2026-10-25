use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent};
use thiserror::Error;
/// Maximum UTF-8 byte length accepted for one normalized paste.
pub const MAX_PASTE_BYTES: usize = 1024 * 1024;
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
