// Unit tests of crates/polaris-nebius-cloud/crates/token-factory/src/sse.rs (`tests`); compiled into that module via #[path].
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
fn rejects_oversized_frame_and_excess_events_before_unbounded_buffering() -> Result<(), TfError> {
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
    let events = futures::executor::block_on(stream.eventsource().try_collect::<Vec<_>>()).ok()?;
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
