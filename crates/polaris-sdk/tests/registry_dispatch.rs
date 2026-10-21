//! Registry Dispatch (descriptor: specs/descriptors/sdk_registry_dispatch.json).
//! The backends below are in-test doubles; they are never shipped.

use std::error::Error;
use std::sync::Mutex;

use polaris_sdk::backends::{OneOf2, OneOf3};
use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::effort::Effort;
use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::ids::{ModelId, Seq, SessionId};
use polaris_sdk::registry::{
    ChatChunk, ChatChunkSource, ChatMessage, ChatModel, ChatRequest, ChatRole, EventStore,
    EventStoreError, ModelError, RedactStats, Redactor, SandboxError, ToolError,
};

type TestResult = Result<(), Box<dyn Error>>;

struct MemStore(Mutex<Vec<EventRecord>>);

impl EventStore for MemStore {
    fn append(&self, record: &EventRecord) -> Result<Seq, EventStoreError> {
        let mut guard = self
            .0
            .lock()
            .map_err(|_| EventStoreError::Unavailable("poisoned".into()))?;
        guard.push(*record);
        Ok(Seq::new(u64::try_from(guard.len()).unwrap_or(u64::MAX)))
    }
    fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
        let guard = self
            .0
            .lock()
            .map_err(|_| EventStoreError::Unavailable("poisoned".into()))?;
        Ok(guard
            .iter()
            .filter(|r| r.seq() >= from && r.seq() <= to)
            .copied()
            .collect())
    }
}

struct ReadOnlyStore;

impl EventStore for ReadOnlyStore {
    fn append(&self, _record: &EventRecord) -> Result<Seq, EventStoreError> {
        Err(EventStoreError::ReadOnly)
    }
    fn read_range(&self, _from: Seq, _to: Seq) -> Result<Vec<EventRecord>, EventStoreError> {
        Ok(Vec::new())
    }
}

fn record() -> Result<EventRecord, Box<dyn Error>> {
    let at: Timestamp = "2026-10-20T12:00:00Z".parse()?;
    Ok(EventRecord::new(
        Seq::new(1),
        at,
        SessionId::from_u128(1),
        Event::SessionClosed,
    ))
}

#[test]
fn one_of2_forwards_event_store_to_active_variant() -> TestResult {
    let active: OneOf2<MemStore, ReadOnlyStore> = OneOf2::A(MemStore(Mutex::new(Vec::new())));
    assert_eq!(active.append(&record()?)?, Seq::new(1));
    assert_eq!(active.read_range(Seq::new(0), Seq::new(10))?.len(), 1);

    let other: OneOf2<MemStore, ReadOnlyStore> = OneOf2::B(ReadOnlyStore);
    assert!(matches!(
        other.append(&record()?),
        Err(EventStoreError::ReadOnly)
    ));
    Ok(())
}

struct Upper;
struct Star;
struct Keep;

impl Redactor for Upper {
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
        out.extend(input.iter().map(u8::to_ascii_uppercase));
        RedactStats {
            bytes_in: input.len(),
            replacements: 0,
        }
    }
}
impl Redactor for Star {
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
        out.extend(input.iter().map(|_| b'*'));
        RedactStats {
            bytes_in: input.len(),
            replacements: u32::try_from(input.len()).unwrap_or(u32::MAX),
        }
    }
}
impl Redactor for Keep {
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
        out.extend_from_slice(input);
        RedactStats {
            bytes_in: input.len(),
            replacements: 0,
        }
    }
}

#[test]
fn one_of3_forwards_redactor() {
    let r: OneOf3<Upper, Keep, Star> = OneOf3::C(Star);
    let mut out = Vec::new();
    let stats = r.redact_into(b"key=abc", &mut out);
    assert_eq!(out, b"*******");
    assert_eq!(
        stats,
        RedactStats {
            bytes_in: 7,
            replacements: 7
        }
    );

    let r: OneOf3<Upper, Keep, Star> = OneOf3::A(Upper);
    let mut out = Vec::new();
    r.redact_into(b"abc", &mut out);
    assert_eq!(out, b"ABC");
}

struct Scripted(Vec<&'static str>);
struct Chunks(std::vec::IntoIter<&'static str>);

impl ChatChunkSource for Chunks {
    async fn next_chunk(&mut self) -> Option<Result<ChatChunk, ModelError>> {
        self.0.next().map(|t| Ok(ChatChunk::Text(t.to_owned())))
    }
}

impl ChatModel for Scripted {
    type Stream = Chunks;
    async fn stream(&self, _req: ChatRequest) -> Result<Self::Stream, ModelError> {
        Ok(Chunks(self.0.clone().into_iter()))
    }
}

struct Refusing;

impl ChatModel for Refusing {
    type Stream = Chunks;
    async fn stream(&self, _req: ChatRequest) -> Result<Self::Stream, ModelError> {
        Err(ModelError::RateLimited {
            retry_after_ms: Some(1500),
        })
    }
}

#[tokio::test]
async fn one_of2_forwards_async_chat_stream() -> TestResult {
    let req = ChatRequest {
        model: ModelId::new("nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B")?,
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: "hi".into(),
        }],
        tools: Vec::new(),
        max_output_tokens: None,
        effort: Effort::Low,
    };
    let model: OneOf2<Scripted, Refusing> = OneOf2::A(Scripted(vec!["Hel", "lo"]));
    let mut stream = model.stream(req.clone()).await?;
    let mut text = String::new();
    while let Some(chunk) = stream.next_chunk().await {
        if let ChatChunk::Text(t) = chunk? {
            text.push_str(&t);
        }
    }
    assert_eq!(text, "Hello");

    let model: OneOf2<Scripted, Refusing> = OneOf2::B(Refusing);
    assert!(matches!(
        model.stream(req).await,
        Err(ModelError::RateLimited {
            retry_after_ms: Some(1500)
        })
    ));
    Ok(())
}

#[test]
fn error_kinds_cover_taxonomy() {
    assert_eq!(
        ModelError::RateLimited {
            retry_after_ms: None
        }
        .kind(),
        ErrorKind::Transient
    );
    assert_eq!(
        ModelError::BadRequest("schema".into()).kind(),
        ErrorKind::Permanent
    );
    assert_eq!(
        ModelError::Upstream { status: 503 }.kind(),
        ErrorKind::Transient
    );
    assert_eq!(
        ModelError::Upstream { status: 400 }.kind(),
        ErrorKind::Permanent
    );
    assert_eq!(
        SandboxError::PolicyDenied("egress".into()).kind(),
        ErrorKind::Policy
    );
    assert_eq!(
        ToolError::Forbidden("outside workspace".into()).kind(),
        ErrorKind::Policy
    );
    assert_eq!(
        EventStoreError::Corrupt { seq: Seq::new(3) }.kind(),
        ErrorKind::Fatal
    );
    assert_eq!(
        EventStoreError::Unavailable("locked".into()).kind(),
        ErrorKind::Transient
    );
}

#[test]
fn arc_forwards_sync_registry_traits() -> TestResult {
    let shared = std::sync::Arc::new(MemStore(Mutex::new(Vec::new())));
    let handle = std::sync::Arc::clone(&shared);
    assert_eq!(EventStore::append(&handle, &record()?)?, Seq::new(1));
    assert_eq!(
        shared.read_range(Seq::new(0), Seq::new(9))?.len(),
        1,
        "both handles see the same store"
    );
    let redactor = std::sync::Arc::new(Star);
    let mut out = Vec::new();
    redactor.redact_into(b"ab", &mut out);
    assert_eq!(out, b"**");
    Ok(())
}
