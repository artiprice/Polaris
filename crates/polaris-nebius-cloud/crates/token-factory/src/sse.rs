//! Bounded incremental Server-Sent Events framing.

use crate::TfError;

const HARD_MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Parsed event payload or the SSE completion sentinel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseEvent {
    /// UTF-8 contents of joined data fields.
    Data(String),
    /// Exact `[DONE]` data sentinel.
    Done,
}

/// Borrowed event view used by streaming clients to route a frame without allocating a `String`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SseEventRef<'a> {
    /// UTF-8 contents of joined data fields, borrowed from the parser's bounded frame buffer.
    Data(&'a str),
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
        if max_frame_bytes == 0
            || max_frame_bytes > HARD_MAX_FRAME_BYTES
            || max_events_per_feed == 0
        {
            return Err(TfError::Configuration);
        }
        let mut line = Vec::new();
        line.try_reserve_exact(max_frame_bytes)
            .map_err(|_| TfError::TooLarge)?;
        let mut data = Vec::new();
        data.try_reserve_exact(max_frame_bytes)
            .map_err(|_| TfError::TooLarge)?;
        Ok(Self {
            line,
            data,
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
        self.feed_with(chunk, |event| {
            out.push(match event {
                SseEventRef::Data(data) => SseEvent::Data(data.to_owned()),
                SseEventRef::Done => SseEvent::Done,
            });
            Ok(())
        })
    }

    /// Feed a network chunk and synchronously route complete borrowed events.
    ///
    /// The callback must consume the event before returning. The event borrows the parser's
    /// bounded frame buffer and remains valid only for the callback's duration.
    /// # Errors
    /// Returns TooLarge for frame/event overflow, Protocol for invalid UTF-8, or the callback's
    /// error.
    pub fn feed_with<F>(&mut self, chunk: &[u8], mut sink: F) -> Result<(), TfError>
    where
        F: FnMut(SseEventRef<'_>) -> Result<(), TfError>,
    {
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
                self.process_line(&mut sink)?;
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
        self.finish_with(|event| {
            out.push(match event {
                SseEventRef::Data(data) => SseEvent::Data(data.to_owned()),
                SseEventRef::Done => SseEvent::Done,
            });
            Ok(())
        })
    }

    /// Flush the final unterminated line and synchronously route borrowed events.
    /// The stream layer separately requires successful terminal markers.
    pub fn finish_with<F>(&mut self, mut sink: F) -> Result<(), TfError>
    where
        F: FnMut(SseEventRef<'_>) -> Result<(), TfError>,
    {
        if self.finished {
            return Err(TfError::Protocol);
        }
        self.finished = true;
        self.emitted = 0;
        if !self.line.is_empty() {
            self.process_line(&mut sink)?;
        }
        if self.has_data {
            self.dispatch(&mut sink)?;
        }
        Ok(())
    }

    fn process_line<F>(&mut self, sink: &mut F) -> Result<(), TfError>
    where
        F: FnMut(SseEventRef<'_>) -> Result<(), TfError>,
    {
        if self.line.is_empty() {
            return self.dispatch(sink);
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
            let value = match value.strip_prefix(b" ") {
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

    fn dispatch<F>(&mut self, sink: &mut F) -> Result<(), TfError>
    where
        F: FnMut(SseEventRef<'_>) -> Result<(), TfError>,
    {
        if !self.has_data {
            self.frame_bytes = 0;
            return Ok(());
        }
        self.emitted = self.emitted.checked_add(1).ok_or(TfError::TooLarge)?;
        if self.emitted > self.max_events_per_feed {
            return Err(TfError::TooLarge);
        }
        let data = std::str::from_utf8(&self.data).map_err(|_| TfError::Protocol)?;
        if data == "[DONE]" {
            sink(SseEventRef::Done)?;
        } else {
            sink(SseEventRef::Data(data))?;
        }
        self.data.clear();
        self.has_data = false;
        self.frame_bytes = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{SseEvent, SseParser};
    use crate::TfError;
    use bytes::Bytes;
    use eventsource_stream::Eventsource;
    use futures::TryStreamExt;
    use proptest::{prelude::*, proptest};

    #[test]
    fn parses_comment_multiline_data_and_done_with_crlf() -> Result<(), TfError> {
        let mut parser = SseParser::new(128, 4)?;
        let mut events = Vec::new();
        parser.feed(
            b": keepalive\r\ndata: first\r\ndata: second\r\n\r\ndata: [DONE]\r\n\r\n",
            &mut events,
        )?;
        parser.finish(&mut events)?;
        if events != [SseEvent::Data("first\nsecond".to_owned()), SseEvent::Done] {
            return Err(TfError::Protocol);
        }
        Ok(())
    }

    #[test]
    fn arbitrary_chunk_split_inside_multibyte_utf8_preserves_event() -> Result<(), TfError> {
        let input = "data: λ😀\r\n\r\n".as_bytes();
        let mut baseline = None;
        for split in 0..=input.len() {
            let mut parser = SseParser::new(64, 4)?;
            let mut events = Vec::new();
            parser.feed(input.get(..split).ok_or(TfError::Protocol)?, &mut events)?;
            parser.feed(input.get(split..).ok_or(TfError::Protocol)?, &mut events)?;
            parser.finish(&mut events)?;
            if events != [SseEvent::Data("λ😀".to_owned())] {
                return Err(TfError::Protocol);
            }
            if baseline
                .as_ref()
                .is_some_and(|expected| expected != &events)
            {
                return Err(TfError::Protocol);
            }
            baseline = Some(events);
        }
        Ok(())
    }

    #[test]
    fn rejects_oversized_frame_and_excess_events_before_unbounded_buffering() -> Result<(), TfError>
    {
        let mut parser = SseParser::new(8, 1)?;
        let mut events = Vec::new();
        if !matches!(
            parser.feed(b"data: 1234\n\n", &mut events),
            Err(TfError::TooLarge)
        ) {
            return Err(TfError::Protocol);
        }

        let mut parser = SseParser::new(64, 1)?;
        let mut events = Vec::new();
        if !matches!(
            parser.feed(b"data: one\n\ndata: two\n\n", &mut events),
            Err(TfError::TooLarge)
        ) || events != [SseEvent::Data("one".to_owned())]
        {
            return Err(TfError::Protocol);
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_utf8_and_feed_after_finish() -> Result<(), TfError> {
        let mut parser = SseParser::new(32, 4)?;
        let mut events = Vec::new();
        if !matches!(
            parser.feed(b"data: \xff\n\n", &mut events),
            Err(TfError::Protocol)
        ) {
            return Err(TfError::Protocol);
        }
        let mut parser = SseParser::new(32, 4)?;
        if !matches!(parser.finish(&mut events), Ok(())) {
            return Err(TfError::Protocol);
        }
        if !matches!(
            parser.feed(b"data: late\n\n", &mut events),
            Err(TfError::Protocol)
        ) {
            return Err(TfError::Protocol);
        }
        Ok(())
    }

    fn parse_partition(input: &[u8], sizes: &[usize]) -> Option<Vec<SseEvent>> {
        let mut parser = SseParser::new(4096, 128).ok()?;
        let mut events = Vec::new();
        let mut offset = 0usize;
        for size in sizes {
            if offset == input.len() {
                break;
            }
            let end = offset.checked_add(*size)?.min(input.len());
            parser.feed(input.get(offset..end)?, &mut events).ok()?;
            offset = end;
        }
        if offset < input.len() {
            parser.feed(input.get(offset..)?, &mut events).ok()?;
        }
        parser.finish(&mut events).ok()?;
        Some(events)
    }

    fn parse_oracle_partition(input: &[u8], sizes: &[usize]) -> Option<Vec<String>> {
        let mut chunks = Vec::new();
        let mut offset = 0usize;
        for size in sizes {
            if offset == input.len() {
                break;
            }
            let end = offset.checked_add(*size)?.min(input.len());
            chunks.push(Bytes::copy_from_slice(input.get(offset..end)?));
            offset = end;
        }
        if offset < input.len() {
            chunks.push(Bytes::copy_from_slice(input.get(offset..)?));
        }
        if chunks.is_empty() {
            chunks.push(Bytes::copy_from_slice(input));
        }
        let stream = futures::stream::iter(
            chunks
                .into_iter()
                .map(Ok::<Bytes, std::convert::Infallible>),
        );
        let events =
            futures::executor::block_on(stream.eventsource().try_collect::<Vec<_>>()).ok()?;
        Some(events.into_iter().map(|event| event.data).collect())
    }

    proptest! {
        #[test]
        fn arbitrary_chunk_partition_preserves_multibyte_sse_data(
            chars in prop::collection::vec(prop_oneof![Just('a'), Just('λ'), Just('😀'), Just(' '), Just(':')], 1..96),
            sizes in prop::collection::vec(1usize..24, 0..48),
        ) {
            let payload: String = chars.into_iter().collect();
            let frame = format!("data: {payload}\r\n\r\n");
            let expected = Some(vec![SseEvent::Data(payload)]);
            prop_assert_eq!(parse_partition(frame.as_bytes(), &[]), expected.clone());
            let partitioned = parse_partition(frame.as_bytes(), &sizes);
            prop_assert_eq!(partitioned.clone(), expected.clone());
            let oracle = parse_oracle_partition(frame.as_bytes(), &sizes);
            let expected_data = expected
                .as_ref()
                .map(|events| events.iter().map(|event| match event {
                    SseEvent::Data(data) => data.clone(),
                    SseEvent::Done => "[DONE]".to_owned(),
                }).collect::<Vec<_>>());
            prop_assert_eq!(oracle, expected_data);
        }
    }
}
