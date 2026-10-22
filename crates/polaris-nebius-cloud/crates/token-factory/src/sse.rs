//! Bounded incremental Server-Sent Events framing.

use crate::TfError;

/// Parsed event payload or the SSE completion sentinel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseEvent {
    /// UTF-8 contents of joined data fields.
    Data(String),
    /// Exact `[DONE]` data sentinel.
    Done,
}

/// Incremental parser for SSE lines split at arbitrary byte boundaries.
///
/// Input bytes are buffered until a complete line arrives, so UTF-8 codepoints split between
/// transport chunks are decoded only after reconstruction. Frame bytes are bounded by the
/// configured maximum.
#[derive(Debug)]
pub struct SseParser {
    line: Vec<u8>,
    data: Vec<u8>,
    has_data: bool,
    frame_bytes: usize,
    max_frame_bytes: usize,
    skip_lf: bool,
    emitted: usize,
    max_events_per_feed: usize,
    finished: bool,
}

impl SseParser {
    /// Create a parser with a per-frame and per-feed event bound.
    ///
    /// # Errors
    /// Returns Configuration when either bound is zero.
    pub fn new(max_frame_bytes: usize, max_events_per_feed: usize) -> Result<Self, TfError> {
        if max_frame_bytes == 0 || max_events_per_feed == 0 {
            return Err(TfError::Configuration);
        }
        Ok(Self {
            line: Vec::new(),
            data: Vec::new(),
            has_data: false,
            frame_bytes: 0,
            max_frame_bytes,
            skip_lf: false,
            emitted: 0,
            max_events_per_feed,
            finished: false,
        })
    }

    /// Feed a network chunk and append complete events to out.
    ///
    /// out is caller-owned and at most max_events_per_feed events are emitted per call.
    /// # Errors
    /// Returns TooLarge for a frame/event overflow and Protocol for invalid UTF-8 in a completed
    /// event.
    pub fn feed(&mut self, chunk: &[u8], out: &mut Vec<SseEvent>) -> Result<(), TfError> {
        if self.finished {
            return Err(TfError::Protocol);
        }
        self.emitted = 0;
        for byte in chunk.iter().copied() {
            if self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if byte == b'\r' || byte == b'\n' {
                self.process_line(out)?;
                self.skip_lf = byte == b'\r';
            } else {
                self.frame_bytes = self.frame_bytes.checked_add(1).ok_or(TfError::TooLarge)?;
                if self.frame_bytes > self.max_frame_bytes {
                    return Err(TfError::TooLarge);
                }
                self.line.push(byte);
            }
        }
        Ok(())
    }

    /// Flush a final unterminated line and event at EOF.
    ///
    /// The stream layer separately requires both finish reason and `[DONE]`; flushing does not
    /// imply successful completion.
    pub fn finish(&mut self, out: &mut Vec<SseEvent>) -> Result<(), TfError> {
        if self.finished {
            return Err(TfError::Protocol);
        }
        self.finished = true;
        self.emitted = 0;
        if !self.line.is_empty() {
            self.process_line(out)?;
        }
        if self.has_data {
            self.dispatch(out)?;
        }
        Ok(())
    }

    fn process_line(&mut self, out: &mut Vec<SseEvent>) -> Result<(), TfError> {
        if self.line.is_empty() {
            return self.dispatch(out);
        }
        if self.line.first() == Some(&b':') {
            self.line.clear();
            return Ok(());
        }
        let mut fields = self.line.splitn(2, |byte| *byte == b':');
        let field = fields.next().ok_or(TfError::Protocol)?;
        let value = match fields.next() {
            Some(value) => value,
            None => &[],
        };
        if field == b"data" {
            let value = match value.strip_prefix(&[b' ']) {
                Some(rest) => rest,
                None => value,
            };
            let separator = if self.has_data { 1 } else { 0 };
            let new_len = self
                .data
                .len()
                .checked_add(value.len())
                .and_then(|n| n.checked_add(separator))
                .ok_or(TfError::TooLarge)?;
            if new_len > self.max_frame_bytes {
                return Err(TfError::TooLarge);
            }
            if self.has_data {
                self.data.push(b'\n');
            }
            self.data.extend_from_slice(value);
            self.has_data = true;
        }
        self.line.clear();
        Ok(())
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) -> Result<(), TfError> {
        if !self.has_data {
            self.frame_bytes = 0;
            return Ok(());
        }
        self.emitted = self.emitted.checked_add(1).ok_or(TfError::TooLarge)?;
        if self.emitted > self.max_events_per_feed {
            return Err(TfError::TooLarge);
        }
        let data =
            String::from_utf8(std::mem::take(&mut self.data)).map_err(|_| TfError::Protocol)?;
        self.has_data = false;
        if data == "[DONE]" {
            out.push(SseEvent::Done);
        } else {
            out.push(SseEvent::Data(data));
        }
        self.frame_bytes = 0;
        Ok(())
    }
}
