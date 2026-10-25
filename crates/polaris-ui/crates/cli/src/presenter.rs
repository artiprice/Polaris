//! Bounded terminal-frame staging for a Ratatui backend; Ratatui remains the sole cell differ.
use crossterm::{
    QueueableCommand,
    cursor::{Hide, MoveTo, Show},
};
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};
use thiserror::Error;
/// Maximum staged encoded frame size.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
/// Typed frame, staging, and output errors.
#[derive(Debug, Error)]
pub enum PresenterError {
    /// No frame was open for the requested operation.
    #[error("no frame is currently staged")]
    NoFrame,
    /// A previous frame must be committed or aborted first.
    #[error("a frame is already staged")]
    FrameActive,
    /// The staged frame exceeded its fixed byte budget.
    #[error("staged frame exceeds the 8 MiB output limit")]
    FrameTooLarge,
    /// A shared staging lock was poisoned.
    #[error("staging lock was poisoned")]
    LockPoisoned,
    /// The in-flight frame write or flush failed and output state is uncertain.
    #[error("terminal output failed: {0}")]
    Io(#[from] io::Error),
}
/// Cursor command staged in the same buffer as the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorIntent {
    /// Hide the terminal cursor.
    Hide,
    /// Show the terminal cursor.
    Show,
    /// Move the cursor to (column, row).
    MoveTo(u16, u16),
}
/// Cursor state known from successfully committed terminal output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorState {
    /// No cursor state has been observed yet.
    Unknown,
    /// Cursor is hidden.
    Hidden,
    /// Cursor is visible at column, row.
    Visible(u16, u16),
}

#[derive(Debug, Default)]
struct Staging {
    closed: bool,
    active: bool,
    bytes: Vec<u8>,
    overflow: bool,
}
/// Cloneable Ratatui writer handle sharing only the bounded staging buffer.
#[derive(Clone, Debug)]
pub struct StagedWriter {
    staging: Arc<Mutex<Staging>>,
}
impl Write for StagedWriter {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let mut s = self
            .staging
            .lock()
            .map_err(|_| io::Error::other("staging lock poisoned"))?;
        if s.closed {
            return Err(io::Error::other("presenter has been closed"));
        }
        if !s.active {
            return Err(io::Error::other("no active frame"));
        }
        if s.bytes
            .len()
            .checked_add(b.len())
            .filter(|n| *n <= MAX_FRAME_BYTES)
            .is_none()
        {
            s.overflow = true;
            return Err(io::Error::other("frame exceeds 8 MiB"));
        }
        s.bytes.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
const STYLE_RESET_WITH_UNDERLINE: &[u8] = b"\x1b[39m\x1b[49m\x1b[59m\x1b[0m";
const STYLE_RESET: &[u8] = b"\x1b[39m\x1b[49m\x1b[0m";

fn redundant_style_reset_prefix(bytes: &[u8]) -> Option<usize> {
    if bytes.starts_with(STYLE_RESET_WITH_UNDERLINE) {
        Some(STYLE_RESET_WITH_UNDERLINE.len())
    } else if bytes.starts_with(STYLE_RESET) {
        Some(STYLE_RESET.len())
    } else {
        None
    }
}

fn style_reset_ends_frame(bytes: &[u8]) -> bool {
    bytes
        .windows(STYLE_RESET_WITH_UNDERLINE.len())
        .any(|part| part == STYLE_RESET_WITH_UNDERLINE)
        || bytes
            .windows(STYLE_RESET.len())
            .any(|part| part == STYLE_RESET)
}

fn is_cursor_suffix(bytes: &[u8]) -> bool {
    let mut probe = bytes.to_vec();
    normalize_cursor_suffix(&mut probe, CursorState::Unknown).is_some() && probe == bytes
}

fn normalize_cursor_suffix(bytes: &mut Vec<u8>, previous: CursorState) -> Option<CursorState> {
    const HIDE: &[u8] = b"\x1b[?25l";
    const SHOW: &[u8] = b"\x1b[?25h";
    if bytes.ends_with(HIDE) {
        let start = bytes.len().checked_sub(HIDE.len())?;
        if previous == CursorState::Hidden {
            bytes.truncate(start);
        }
        return Some(CursorState::Hidden);
    }
    if bytes.last() != Some(&b'H') {
        return None;
    }
    let esc = bytes.iter().rposition(|byte| *byte == 0x1b)?;
    let command = bytes.get(esc..)?;
    let body = command.strip_prefix(b"\x1b[")?.strip_suffix(b"H")?;
    let separator = body.iter().position(|byte| *byte == b';')?;
    let row = std::str::from_utf8(body.get(..separator)?)
        .ok()?
        .parse::<u32>()
        .ok()?;
    let column = std::str::from_utf8(body.get(separator.checked_add(1)?..)?)
        .ok()?
        .parse::<u32>()
        .ok()?;
    let row = u16::try_from(row.checked_sub(1)?).ok()?;
    let column = u16::try_from(column.checked_sub(1)?).ok()?;
    let shown_at = esc
        .checked_sub(SHOW.len())
        .filter(|start| bytes.get(*start..esc) == Some(SHOW));
    let cursor_start = match shown_at {
        Some(start) => start,
        None => esc,
    };
    let state = CursorState::Visible(column, row);
    if previous == state {
        bytes.truncate(cursor_start);
    } else if matches!(previous, CursorState::Visible(_, _)) && shown_at.is_some() {
        if let Some(start) = shown_at {
            bytes.drain(start..esc);
        }
    }
    Some(state)
}

/// Owns the sole output writer and commits at most one staged frame.
#[derive(Debug)]
pub struct BufferedPresenter<W: Write> {
    output: W,
    staging: Arc<Mutex<Staging>>,
    resync: bool,
    cursor: CursorState,
    style_reset_known: bool,
}
impl<W: Write> BufferedPresenter<W> {
    /// Creates an idle presenter around the terminal output sink.
    pub fn new(output: W) -> Self {
        Self {
            output,
            staging: Arc::new(Mutex::new(Staging::default())),
            resync: false,
            cursor: CursorState::Unknown,
            style_reset_known: false,
        }
    }
    /// Creates a presenter while supplying already-known terminal cursor state.
    pub fn new_with_cursor_state(output: W, cursor: CursorState) -> Self {
        Self {
            output,
            staging: Arc::new(Mutex::new(Staging::default())),
            resync: false,
            cursor,
            style_reset_known: false,
        }
    }
    /// Returns a cloneable backend handle which can only write into an active staged frame.
    pub fn writer(&self) -> StagedWriter {
        StagedWriter {
            staging: Arc::clone(&self.staging),
        }
    }
    /// Opens an empty frame buffer.
    pub fn begin_frame(&mut self) -> Result<(), PresenterError> {
        let mut s = self
            .staging
            .lock()
            .map_err(|_| PresenterError::LockPoisoned)?;
        if s.active {
            return Err(PresenterError::FrameActive);
        }
        s.bytes.clear();
        s.overflow = false;
        s.active = true;
        Ok(())
    }
    /// Discards a staged frame without touching terminal output.
    pub fn abort_frame(&mut self) -> Result<(), PresenterError> {
        let mut s = self
            .staging
            .lock()
            .map_err(|_| PresenterError::LockPoisoned)?;
        s.bytes.clear();
        s.overflow = false;
        s.active = false;
        Ok(())
    }
    /// Queues a cursor command in the active frame.
    pub fn stage_cursor(&mut self, intent: CursorIntent) -> Result<(), PresenterError> {
        let mut w = self.writer();
        match intent {
            CursorIntent::Hide => {
                w.queue(Hide)?;
            }
            CursorIntent::Show => {
                w.queue(Show)?;
            }
            CursorIntent::MoveTo(x, y) => {
                w.queue(MoveTo(x, y))?;
            }
        }
        Ok(())
    }
    /// Writes and flushes one nonempty frame; a partial failure requests a full resync.
    /// On output failure, the owner should invalidate Ratatui's diff and close the
    /// terminal guard; close attempts cursor reveal and all remaining mode restores,
    /// though a failed output device can prevent the terminal from receiving them.
    pub fn commit(&mut self) -> Result<usize, PresenterError> {
        let mut s = self
            .staging
            .lock()
            .map_err(|_| PresenterError::LockPoisoned)?;
        if !s.active {
            return Err(PresenterError::NoFrame);
        }
        if s.overflow {
            s.bytes.clear();
            s.active = false;
            return Err(PresenterError::FrameTooLarge);
        }
        let mut bytes = std::mem::take(&mut s.bytes);
        s.active = false;
        drop(s);
        let next_cursor = normalize_cursor_suffix(&mut bytes, self.cursor);
        if self.style_reset_known {
            if redundant_style_reset_prefix(&bytes) == Some(bytes.len()) {
                bytes.clear();
            } else if let Some(reset_len) = redundant_style_reset_prefix(&bytes) {
                if bytes.get(reset_len..).is_some_and(is_cursor_suffix) {
                    bytes.drain(..reset_len);
                }
            }
        }
        if bytes.is_empty() {
            if let Some(cursor) = next_cursor {
                self.cursor = cursor;
            }
            return Ok(0);
        }
        if let Err(e) = self
            .output
            .write_all(&bytes)
            .and_then(|_| self.output.flush())
        {
            self.resync = true;
            self.cursor = CursorState::Unknown;
            self.style_reset_known = false;
            return Err(PresenterError::Io(e));
        }
        if let Some(cursor) = next_cursor {
            self.cursor = cursor;
        }
        self.style_reset_known = style_reset_ends_frame(&bytes);
        Ok(bytes.len())
    }
    /// Takes the flag indicating Ratatui should invalidate its current cell buffer.
    pub fn take_resync_required(&mut self) -> bool {
        std::mem::take(&mut self.resync)
    }
    /// Borrows the owned sink for diagnostics and tests.
    pub fn output(&self) -> &W {
        &self.output
    }
    /// Closes every outstanding writer handle and returns the output sink.
    pub fn into_inner(self) -> Result<W, PresenterError> {
        let mut staging = self
            .staging
            .lock()
            .map_err(|_| PresenterError::LockPoisoned)?;
        staging.closed = true;
        staging.active = false;
        staging.bytes.clear();
        drop(staging);
        Ok(self.output)
    }
}
