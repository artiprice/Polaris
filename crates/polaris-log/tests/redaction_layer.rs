//! Redaction Layer (descriptor: specs/descriptors/log_redaction_layer.json).

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use polaris_log::{DrainQueue, LogCounters, LogLine, RedactionLayer, RingSink, Sinks};
use polaris_sdk::registry::{RedactStats, Redactor};
use proptest::prelude::*;
use tracing::Dispatch;
use tracing_subscriber::Registry;
use tracing_subscriber::layer::SubscriberExt;

type TestResult = Result<(), Box<dyn Error>>;

const MASK: &[u8] = b"[REDACTED]";

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Replaces every occurrence of `needle` with `[REDACTED]`.
struct Mask {
    needle: Vec<u8>,
}

impl Mask {
    fn new(needle: &str) -> Self {
        Self {
            needle: needle.as_bytes().to_vec(),
        }
    }
}

impl Redactor for Mask {
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
        let mut rest = input;
        let mut replacements = 0u32;
        while let Some(pos) = find(rest, &self.needle) {
            out.extend_from_slice(rest.get(..pos).unwrap_or_default());
            out.extend_from_slice(MASK);
            rest = rest.get(pos + self.needle.len()..).unwrap_or_default();
            replacements += 1;
        }
        out.extend_from_slice(rest);
        RedactStats {
            bytes_in: input.len(),
            replacements,
        }
    }
}

struct Panics;

impl Redactor for Panics {
    fn redact_into(&self, _input: &[u8], _out: &mut Vec<u8>) -> RedactStats {
        panic!("redactor failure injected by the test");
    }
}

/// Claims to have read one byte less than it was given (an incomplete scan).
struct ShortRead;

impl Redactor for ShortRead {
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
        out.extend_from_slice(input);
        RedactStats {
            bytes_in: input.len().saturating_sub(1),
            replacements: 0,
        }
    }
}

/// Logs from inside the redactor (re-entrancy) exactly once.
struct Reentrant {
    fired: AtomicBool,
}

impl Redactor for Reentrant {
    fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats {
        if !self.fired.swap(true, Ordering::SeqCst) {
            tracing::info!("from inside the redactor");
        }
        out.extend_from_slice(input);
        RedactStats {
            bytes_in: input.len(),
            replacements: 0,
        }
    }
}

struct Rig {
    dispatch: Dispatch,
    tui: Arc<RingSink>,
    telemetry: Arc<RingSink>,
    queue: Arc<DrainQueue>,
    counters: Arc<LogCounters>,
}

fn rig<R: Redactor + 'static>(redactor: R, json: bool, line_limit: usize, queue_cap: usize) -> Rig {
    let counters = LogCounters::new();
    let tui = Arc::new(RingSink::with_line_limit(64, line_limit));
    let telemetry = Arc::new(RingSink::with_line_limit(64, line_limit));
    let queue = Arc::new(DrainQueue::new(queue_cap, line_limit));
    let sinks = Sinks::new(Arc::clone(&counters))
        .with_tui(Arc::clone(&tui))
        .with_telemetry(Arc::clone(&telemetry))
        .with_file(Arc::clone(&queue));
    let layer = RedactionLayer::new(redactor, sinks).with_json(json);
    Rig {
        dispatch: Dispatch::new(Registry::default().with(layer)),
        tui,
        telemetry,
        queue,
        counters,
    }
}

fn queue_lines(queue: &DrainQueue) -> Vec<LogLine> {
    let mut out = Vec::new();
    while let Some(l) = queue.try_recv() {
        out.push(l);
    }
    out
}

#[derive(Debug)]
struct Holder {
    token: String,
}

fn fake_secret(tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("sk-fake-{tag}-{nanos:x}")
}

fn assert_secret_free(rig: &Rig, secret: &str) {
    let all = [
        rig.tui.snapshot(),
        rig.telemetry.snapshot(),
        queue_lines(&rig.queue),
    ];
    for lines in &all {
        assert!(!lines.is_empty());
        for l in lines {
            assert!(!l.message.contains(secret), "leaked: {}", l.message);
            assert!(!l.target.contains(secret));
        }
    }
}

#[test]
fn secret_never_reaches_sink() {
    let secret = fake_secret("text");
    let rig = rig(Mask::new(&secret), false, 1024, 16);
    let holder = Holder {
        token: secret.clone(),
    };
    assert_eq!(holder.token, secret, "fixture holds the seeded secret");
    tracing::dispatcher::with_default(&rig.dispatch, || {
        tracing::info!("auth header token={}", secret);
        tracing::info!(api_key = %secret, "request sent");
        tracing::warn!(holder = ?holder, "debug-formatted holder");
        tracing::error!(secret = secret.as_str(), "str field");
    });
    assert_eq!(rig.tui.len(), 4);
    assert!(rig.counters.redactions() >= 4);
    assert!(
        rig.tui
            .snapshot()
            .iter()
            .all(|l| l.message.contains("[REDACTED]")),
        "every record had its secret masked"
    );
    assert_secret_free(&rig, &secret);
}

#[test]
fn secret_never_reaches_json_sink() {
    let secret = fake_secret("json");
    let rig = rig(Mask::new(&secret), true, 1024, 16);
    tracing::dispatcher::with_default(&rig.dispatch, || {
        tracing::info!(token = secret.as_str(), "login");
        tracing::info!("inline {secret} in message");
    });
    assert_eq!(rig.counters.redact_failures(), 0);
    assert!(rig.counters.redactions() >= 2);
    assert_secret_free(&rig, &secret);
}

#[test]
fn non_secret_text_unchanged() -> TestResult {
    let rig = rig(Mask::new("never-present"), false, 1024, 16);
    tracing::dispatcher::with_default(&rig.dispatch, || {
        tracing::info!(n = 5, name = "bob", ok = true, "hello world");
    });
    let line = rig.tui.snapshot().into_iter().next().ok_or("no line")?;
    // The message comes first, then the fields in declaration order.
    assert_eq!(line.message, "hello world n=5 name=\"bob\" ok=true");
    assert_eq!(rig.counters.redactions(), 0);
    Ok(())
}

#[test]
fn level_and_target_are_preserved() -> TestResult {
    let rig = rig(Mask::new("x"), false, 1024, 16);
    tracing::dispatcher::with_default(&rig.dispatch, || {
        tracing::warn!(target: "my::target", "careful");
    });
    let line = rig.tui.snapshot().into_iter().next().ok_or("no line")?;
    assert_eq!(line.level, tracing::Level::WARN);
    assert_eq!(line.target, "my::target");
    Ok(())
}

#[test]
fn redactor_error_drops_record() {
    for failing in [0, 1] {
        let secret = fake_secret("fail");
        let (dispatch_rig, name) = if failing == 0 {
            (rig(Panics, false, 1024, 16), "panicking redactor")
        } else {
            (rig(ShortRead, false, 1024, 16), "short-read redactor")
        };
        tracing::dispatcher::with_default(&dispatch_rig.dispatch, || {
            tracing::info!("payload {secret}");
            tracing::info!("another {secret}");
        });
        assert!(dispatch_rig.tui.is_empty(), "{name}: ring got a record");
        assert!(dispatch_rig.telemetry.is_empty(), "{name}");
        assert_eq!(dispatch_rig.queue.pending(), 0, "{name}");
        assert_eq!(dispatch_rig.counters.redact_failures(), 2, "{name}");
    }
}

#[test]
fn control_characters_are_escaped_in_text() -> TestResult {
    let rig = rig(Mask::new("zzz"), false, 1024, 16);
    tracing::dispatcher::with_default(&rig.dispatch, || {
        tracing::info!("line one\nFAKE forged line\r\u{1b}[31mred");
    });
    let line = rig.tui.snapshot().into_iter().next().ok_or("no line")?;
    assert!(!line.message.contains('\n'));
    assert!(!line.message.contains('\r'));
    assert!(!line.message.contains('\u{1b}'));
    assert!(line.message.contains("\\n"));
    Ok(())
}

#[test]
fn json_fields_are_structured() -> TestResult {
    let rig = rig(Mask::new("never-present"), true, 1024, 16);
    tracing::dispatcher::with_default(&rig.dispatch, || {
        tracing::info!(user = "alice", count = 3, ok = true, "hello \"q\"");
    });
    let line = rig.tui.snapshot().into_iter().next().ok_or("no line")?;
    assert_eq!(
        line.message,
        r#"{"message":"hello \"q\"","user":"alice","count":3,"ok":true}"#
    );
    Ok(())
}

#[test]
fn json_values_that_were_redacted_become_strings() -> TestResult {
    let rig = rig(Mask::new("12345"), true, 1024, 16);
    tracing::dispatcher::with_default(&rig.dispatch, || {
        tracing::info!(pin = 12345, other = 7, "pin set");
    });
    let line = rig.tui.snapshot().into_iter().next().ok_or("no line")?;
    assert_eq!(
        line.message,
        r#"{"message":"pin set","pin":"[REDACTED]","other":7}"#
    );
    Ok(())
}

#[test]
fn json_over_limit_keeps_valid_object_with_marker() -> TestResult {
    let rig = rig(Mask::new("never-present"), true, 512, 16);
    let big = "a".repeat(2_000);
    tracing::dispatcher::with_default(&rig.dispatch, || {
        tracing::info!(first = 1, blob = big.as_str(), "tail");
    });
    let line = rig.tui.snapshot().into_iter().next().ok_or("no line")?;
    assert!(
        line.message
            .starts_with(r#"{"message":"tail","first":1,"blob":"aaa"#),
        "{}",
        line.message
    );
    assert!(
        line.message.ends_with(r#","_truncated":true}"#),
        "{}",
        line.message
    );
    assert!(line.message.len() <= 512 - 96);
    assert!(
        !line.truncated,
        "the object itself was shaped to fit, not cut"
    );
    Ok(())
}

#[test]
fn oversize_event_is_replaced_by_a_stub() -> TestResult {
    let secret = fake_secret("big");
    let rig = rig(Mask::new(&secret), false, 1024, 16);
    let huge = format!("{}{secret}", "q".repeat(100_000));
    tracing::dispatcher::with_default(&rig.dispatch, || {
        tracing::info!("{huge}");
    });
    let line = rig.tui.snapshot().into_iter().next().ok_or("no line")?;
    assert!(line.message.contains("exceeded"), "{}", line.message);
    assert!(!line.message.contains(&secret));
    assert_eq!(rig.counters.oversize_events(), 1);
    Ok(())
}

#[test]
fn queue_overflow_never_blocks_the_caller() {
    let rig = rig(Mask::new("x"), false, 1024, 4);
    tracing::dispatcher::with_default(&rig.dispatch, || {
        for i in 0..1000 {
            tracing::info!(i, "event");
        }
    });
    assert_eq!(rig.queue.accepted(), 4);
    assert_eq!(rig.queue.dropped(), 996);
    assert_eq!(rig.tui.len(), 64, "the ring still sees every record");
}

#[test]
fn redactor_that_logs_does_not_deadlock_or_panic() {
    let rig = rig(
        Reentrant {
            fired: AtomicBool::new(false),
        },
        false,
        1024,
        16,
    );
    tracing::dispatcher::with_default(&rig.dispatch, || {
        tracing::info!("outer");
        tracing::info!("second");
    });
    // tracing-core refuses to re-enter a subscriber from inside its own callbacks, so the event
    // logged by the redactor is discarded by tracing itself; the layer must neither deadlock on
    // its buffers nor panic, and must keep working for later events.
    let msgs: Vec<String> = rig.tui.snapshot().into_iter().map(|l| l.message).collect();
    assert_eq!(msgs, ["outer", "second"]);
}

proptest! {
    #[test]
    fn prop_secret_never_survives_and_never_lengthens(
        prefix in "\\PC{0,40}",
        suffix in "\\PC{0,40}",
        secret in "sk-fake-[0-9a-f]{16}",
    ) {
        let rig = rig(Mask::new(&secret), false, 1024, 16);
        let original = format!("{prefix}{secret}{suffix}");
        tracing::dispatcher::with_default(&rig.dispatch, || {
            tracing::info!("{original}");
        });
        let lines = rig.tui.snapshot();
        prop_assert_eq!(lines.len(), 1);
        let plain = format!("{prefix}{suffix}");
        for l in &lines {
            prop_assert!(!l.message.contains(&secret));
            // \PC has no control characters, so nothing is escaped: the visible text is the
            // surrounding text plus the mask, strictly shorter than the secret-bearing original.
            prop_assert_eq!(l.message.len(), plain.len() + MASK.len());
            prop_assert!(l.message.len() < original.len());
        }
        for l in &queue_lines(&rig.queue) {
            prop_assert!(!l.message.contains(&secret));
        }
    }
}
