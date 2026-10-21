//! The `tracing` layer: format an event into reused buffers, redact it, enqueue it.
//!
//! Order of operations for one event (all on the emitting thread, no I/O):
//!
//! 1. The event's fields are formatted into thread-local buffers (`message k=v ...`, or one JSON
//!    object with one key per field). Formatting is bounded by [`RAW_LIMIT`]; a larger event is
//!    replaced by a fixed stub and never partially scanned.
//! 2. The text goes through the sdk [`Redactor`] (text mode: once for the whole line, so key-aware
//!    patterns see `password=...`; JSON mode: once per `key=value` pair, so JSON escaping cannot
//!    hide a secret). The redactor is infallible by contract, so a *panic* or an *incomplete scan*
//!    (`RedactStats::bytes_in` differing from the input length) is treated as failure and the
//!    record is dropped (fail closed).
//! 3. Control characters are escaped (log and terminal injection).
//! 4. The redacted record is pushed to the TUI ring, the telemetry ring and the file queue.

use std::cell::RefCell;
use std::fmt::{self, Write as _};
use std::io::Write as _;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use polaris_sdk::registry::Redactor;
use tracing::field::{Field, Visit};
use tracing::{Event, Metadata, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::entry::{LogRecordRef, MAX_TARGET_BYTES};
use crate::queue::DrainQueue;
use crate::ring::{RingSink, lock};

/// Largest formatted event (per buffer) that is redacted; a bigger event becomes a stub.
pub const RAW_LIMIT: usize = 65_536;

/// Initial capacity of each thread-local buffer.
const SCRATCH_INITIAL: usize = 2_048;

/// Bytes reserved at the end of a JSON object for `,"_truncated":true}`.
const JSON_TAIL: usize = 19;

const STUB_TEXT: &str = "[polaris-log: event exceeded 65536 bytes and was dropped]";
const STUB_JSON: &str = "{\"_dropped\":\"event exceeded 65536 bytes\"}";

/// Counters shared by the layer, the drain thread and the guard.
#[derive(Debug, Default)]
pub struct LogCounters {
    redactions: AtomicU64,
    redact_failures: AtomicU64,
    oversize_events: AtomicU64,
    sink_errors: AtomicU64,
    last_sink_error: Mutex<Option<String>>,
}

impl LogCounters {
    /// New zeroed counters, ready to share.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Secret/PII spans replaced by the redactor.
    #[must_use]
    pub fn redactions(&self) -> u64 {
        self.redactions.load(Ordering::Relaxed)
    }

    /// Events dropped because the redactor panicked or did not scan the whole input.
    #[must_use]
    pub fn redact_failures(&self) -> u64 {
        self.redact_failures.load(Ordering::Relaxed)
    }

    /// Events replaced by a stub because they exceeded [`RAW_LIMIT`].
    #[must_use]
    pub fn oversize_events(&self) -> u64 {
        self.oversize_events.load(Ordering::Relaxed)
    }

    /// Errors reported by the file / console sinks (after their bounded retries).
    #[must_use]
    pub fn sink_errors(&self) -> u64 {
        self.sink_errors.load(Ordering::Relaxed)
    }

    /// The most recent sink error, as text.
    #[must_use]
    pub fn last_sink_error(&self) -> Option<String> {
        lock(&self.last_sink_error).clone()
    }

    /// Records a sink error (cold path).
    pub(crate) fn record_sink_error(&self, message: String) {
        self.sink_errors.fetch_add(1, Ordering::Relaxed);
        *lock(&self.last_sink_error) = Some(message);
    }
}

/// The destinations a [`RedactionLayer`] feeds. Any subset may be present.
#[derive(Debug, Clone)]
pub struct Sinks {
    tui: Option<Arc<RingSink>>,
    telemetry: Option<Arc<RingSink>>,
    file: Option<Arc<DrainQueue>>,
    counters: Arc<LogCounters>,
}

impl Sinks {
    /// No destinations yet; failures are counted in `counters`.
    #[must_use]
    pub fn new(counters: Arc<LogCounters>) -> Self {
        Self {
            tui: None,
            telemetry: None,
            file: None,
            counters,
        }
    }

    /// Adds the TUI log-pane ring.
    #[must_use]
    pub fn with_tui(mut self, ring: Arc<RingSink>) -> Self {
        self.tui = Some(ring);
        self
    }

    /// Adds the telemetry ring.
    #[must_use]
    pub fn with_telemetry(mut self, ring: Arc<RingSink>) -> Self {
        self.telemetry = Some(ring);
        self
    }

    /// Adds the queue feeding the file / console drain thread.
    #[must_use]
    pub fn with_file(mut self, queue: Arc<DrainQueue>) -> Self {
        self.file = Some(queue);
        self
    }

    fn push_all(&self, rec: &LogRecordRef<'_>) {
        if let Some(ring) = &self.tui {
            ring.push(rec);
        }
        if let Some(ring) = &self.telemetry {
            ring.push(rec);
        }
        if let Some(queue) = &self.file {
            queue.push(rec);
        }
    }

    fn limits(&self) -> impl Iterator<Item = usize> + '_ {
        let rings = [&self.tui, &self.telemetry]
            .into_iter()
            .flatten()
            .map(|r| r.line_limit());
        rings.chain(self.file.iter().map(|q| q.line_limit()))
    }
}

/// Reusable per-thread formatting buffers.
struct Scratch {
    msg: Vec<u8>,
    fields: Vec<u8>,
    raw: Vec<u8>,
    red: Vec<u8>,
    body: Vec<u8>,
}

impl Scratch {
    fn new() -> Self {
        Self {
            msg: Vec::with_capacity(SCRATCH_INITIAL),
            fields: Vec::with_capacity(SCRATCH_INITIAL),
            raw: Vec::with_capacity(SCRATCH_INITIAL),
            red: Vec::with_capacity(SCRATCH_INITIAL),
            body: Vec::with_capacity(SCRATCH_INITIAL),
        }
    }
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::new());
}

enum Outcome {
    /// `Scratch::body` holds the finished message.
    Ready,
    /// The event exceeded [`RAW_LIMIT`].
    Oversize,
    /// The redactor failed; the record is dropped (already counted).
    Failed,
}

/// A `tracing_subscriber::Layer` that redacts every event before any sink sees it.
///
/// Per event it uses the calling thread's reusable buffers, so after warm-up (first event on the
/// thread, buffers grown to the largest event seen) the path from `tracing::info!` to the enqueue
/// does not allocate. See `tests/log_enqueue_zero_alloc.rs` for exactly what is measured.
pub struct RedactionLayer<R> {
    redactor: R,
    sinks: Sinks,
    json: bool,
    text_cap: usize,
    json_budget: usize,
}

impl<R: Redactor + 'static> RedactionLayer<R> {
    /// A text-mode layer feeding `sinks`.
    #[must_use]
    pub fn new(redactor: R, sinks: Sinks) -> Self {
        let max = sinks
            .limits()
            .max()
            .unwrap_or(crate::ring::DEFAULT_RING_LINE_BYTES);
        let min = sinks
            .limits()
            .min()
            .unwrap_or(crate::ring::DEFAULT_RING_LINE_BYTES);
        Self {
            redactor,
            sinks,
            json: false,
            text_cap: max,
            json_budget: min.saturating_sub(MAX_TARGET_BYTES + JSON_TAIL),
        }
    }

    /// Switches to JSON mode: the message of each record is one JSON object with one key per
    /// tracing field (`message` included), shaped to fit the smallest sink line limit.
    #[must_use]
    pub fn with_json(mut self, json: bool) -> Self {
        self.json = json;
        self
    }

    /// Runs the redactor over `input`, appending to `out`. `false` means fail closed.
    fn redact(&self, input: &[u8], out: &mut Vec<u8>) -> bool {
        out.clear();
        let result = catch_unwind(AssertUnwindSafe(|| self.redactor.redact_into(input, out)));
        match result {
            Ok(stats) if stats.bytes_in == input.len() => {
                if stats.replacements > 0 {
                    self.sinks
                        .counters
                        .redactions
                        .fetch_add(u64::from(stats.replacements), Ordering::Relaxed);
                }
                true
            }
            Ok(_) | Err(_) => {
                self.sinks
                    .counters
                    .redact_failures
                    .fetch_add(1, Ordering::Relaxed);
                out.clear();
                false
            }
        }
    }

    fn process(&self, s: &mut Scratch, event: &Event<'_>, time: SystemTime) {
        let outcome = if self.json {
            self.build_json(s, event)
        } else {
            self.build_text(s, event)
        };
        let meta = event.metadata();
        match outcome {
            Outcome::Failed => {}
            Outcome::Oversize => {
                self.sinks
                    .counters
                    .oversize_events
                    .fetch_add(1, Ordering::Relaxed);
                let stub = if self.json { STUB_JSON } else { STUB_TEXT };
                self.dispatch(meta, time, stub);
            }
            Outcome::Ready => match std::str::from_utf8(&s.body) {
                Ok(message) => self.dispatch(meta, time, message),
                Err(_) => {
                    self.sinks
                        .counters
                        .redact_failures
                        .fetch_add(1, Ordering::Relaxed);
                }
            },
        }
    }

    fn dispatch(&self, meta: &Metadata<'_>, time: SystemTime, message: &str) {
        self.sinks.push_all(&LogRecordRef {
            time,
            level: *meta.level(),
            target: meta.target(),
            message,
        });
    }

    fn build_text(&self, s: &mut Scratch, event: &Event<'_>) -> Outcome {
        s.msg.clear();
        s.fields.clear();
        let mut visitor = TextVisitor {
            msg: &mut s.msg,
            fields: &mut s.fields,
            overflow: false,
        };
        event.record(&mut visitor);
        if visitor.overflow {
            return Outcome::Oversize;
        }
        s.raw.clear();
        s.raw.extend_from_slice(&s.msg);
        if !s.fields.is_empty() {
            if !s.raw.is_empty() {
                s.raw.push(b' ');
            }
            s.raw.extend_from_slice(&s.fields);
        }
        if !self.redact(&s.raw, &mut s.red) {
            return Outcome::Failed;
        }
        s.body.clear();
        sanitize_into(&mut s.body, &String::from_utf8_lossy(&s.red), self.text_cap);
        Outcome::Ready
    }

    fn build_json(&self, s: &mut Scratch, event: &Event<'_>) -> Outcome {
        s.body.clear();
        s.body.push(b'{');
        let mut visitor = JsonVisitor {
            layer: self,
            raw: &mut s.raw,
            red: &mut s.red,
            body: &mut s.body,
            first: true,
            stopped: false,
            truncated: false,
            failed: false,
            oversize: false,
        };
        event.record(&mut visitor);
        let (failed, oversize, truncated, first) = (
            visitor.failed,
            visitor.oversize,
            visitor.truncated,
            visitor.first,
        );
        if failed {
            return Outcome::Failed;
        }
        if oversize {
            return Outcome::Oversize;
        }
        if truncated {
            if !first {
                s.body.push(b',');
            }
            s.body.extend_from_slice(b"\"_truncated\":true");
        }
        s.body.push(b'}');
        Outcome::Ready
    }
}

impl<S, R> Layer<S> for RedactionLayer<R>
where
    S: Subscriber,
    R: Redactor + 'static,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let time = SystemTime::now();
        let done = SCRATCH.try_with(|cell| match cell.try_borrow_mut() {
            Ok(mut scratch) => {
                self.process(&mut scratch, event, time);
                true
            }
            Err(_) => false,
        });
        if !matches!(done, Ok(true)) {
            // Buffers busy (re-entrant call) or thread-local storage already torn down: use a
            // private set instead of losing the record. Allocates, but only on this rare path.
            self.process(&mut Scratch::new(), event, time);
        }
    }
}

/// Appends formatted text to a buffer, refusing (and flagging) anything past [`RAW_LIMIT`].
struct Bounded<'a> {
    buf: &'a mut Vec<u8>,
    overflow: &'a mut bool,
}

impl fmt::Write for Bounded<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        if *self.overflow || self.buf.len().saturating_add(s.len()) > RAW_LIMIT {
            *self.overflow = true;
            return Err(fmt::Error);
        }
        self.buf.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

struct TextVisitor<'a> {
    msg: &'a mut Vec<u8>,
    fields: &'a mut Vec<u8>,
    overflow: bool,
}

impl TextVisitor<'_> {
    fn message(&mut self, write: impl FnOnce(&mut Bounded<'_>) -> fmt::Result) {
        let mut w = Bounded {
            buf: self.msg,
            overflow: &mut self.overflow,
        };
        let _ = write(&mut w);
    }

    fn field(&mut self, name: &str, write: impl FnOnce(&mut Bounded<'_>) -> fmt::Result) {
        let mut w = Bounded {
            buf: self.fields,
            overflow: &mut self.overflow,
        };
        if !w.buf.is_empty() {
            let _ = w.write_char(' ');
        }
        let _ = w.write_str(name);
        let _ = w.write_char('=');
        let _ = write(&mut w);
    }
}

impl Visit for TextVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message(|w| w.write_str(value));
        } else {
            self.field(field.name(), |w| write!(w, "{value:?}"));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message(|w| write!(w, "{value:?}"));
        } else {
            self.field(field.name(), |w| write!(w, "{value:?}"));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// May be emitted unquoted if redaction left it unchanged (integers, booleans, finite floats).
    Bare,
    /// Always a JSON string.
    Quoted,
}

struct JsonVisitor<'a, R> {
    layer: &'a RedactionLayer<R>,
    raw: &'a mut Vec<u8>,
    red: &'a mut Vec<u8>,
    body: &'a mut Vec<u8>,
    first: bool,
    stopped: bool,
    truncated: bool,
    failed: bool,
    oversize: bool,
}

impl<R: Redactor + 'static> JsonVisitor<'_, R> {
    fn field(
        &mut self,
        name: &str,
        kind: Kind,
        write: impl FnOnce(&mut Bounded<'_>) -> fmt::Result,
    ) {
        if self.stopped {
            return;
        }
        self.raw.clear();
        let mut overflow = false;
        {
            let mut w = Bounded {
                buf: self.raw,
                overflow: &mut overflow,
            };
            let _ = w.write_str(name);
            let _ = w.write_char('=');
            let _ = write(&mut w);
        }
        if overflow {
            self.oversize = true;
            self.stopped = true;
            return;
        }
        // The redactor sees `key=value` so key-aware patterns still work.
        if !self.layer.redact(self.raw, self.red) {
            self.failed = true;
            self.stopped = true;
            return;
        }
        let unchanged = *self.red == *self.raw;
        let value = self
            .red
            .strip_prefix(name.as_bytes())
            .and_then(|r| r.strip_prefix(b"="))
            .unwrap_or(self.red);

        let budget = self.layer.json_budget;
        let mark = self.body.len();
        if !self.first {
            self.body.push(b',');
        }
        self.body.push(b'"');
        let key_fits = escape_json_bounded(self.body, name, budget);
        self.body.extend_from_slice(b"\":");
        if !key_fits || self.body.len() >= budget {
            self.body.truncate(mark);
            self.truncated = true;
            self.stopped = true;
            return;
        }
        if kind == Kind::Bare && unchanged {
            if self.body.len().saturating_add(value.len()) > budget {
                self.body.truncate(mark);
                self.truncated = true;
                self.stopped = true;
                return;
            }
            self.body.extend_from_slice(value);
        } else {
            self.body.push(b'"');
            let text = String::from_utf8_lossy(value);
            let fits = escape_json_bounded(self.body, &text, budget.saturating_sub(1));
            self.body.push(b'"');
            if !fits {
                self.truncated = true;
                self.stopped = true;
            }
        }
        self.first = false;
    }
}

impl<R: Redactor + 'static> Visit for JsonVisitor<'_, R> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.field(field.name(), Kind::Quoted, |w| w.write_str(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.field(field.name(), Kind::Bare, |w| write!(w, "{value}"));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.field(field.name(), Kind::Bare, |w| write!(w, "{value}"));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.field(field.name(), Kind::Bare, |w| write!(w, "{value}"));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        let kind = if value.is_finite() {
            Kind::Bare
        } else {
            Kind::Quoted
        };
        self.field(field.name(), kind, |w| write!(w, "{value:?}"));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.field(field.name(), Kind::Quoted, |w| write!(w, "{value:?}"));
    }
}

/// Appends `s` to `out`, escaping control characters, stopping once `cap` bytes are reached.
fn sanitize_into(out: &mut Vec<u8>, s: &str, cap: usize) {
    let mut buf = [0u8; 4];
    for c in s.chars() {
        if out.len() >= cap {
            break;
        }
        match c {
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.push(b'\t'),
            c if c.is_control() => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
            c => out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes()),
        }
    }
}

fn hex_digit(n: u32) -> u8 {
    char::from_digit(n & 15, 16).map_or(b'0', |c| c as u8)
}

/// Appends `s` JSON-escaped (no quotes) to `out` as long as `out` stays within `limit` bytes.
/// Returns `false` if some of `s` did not fit.
fn escape_json_bounded(out: &mut Vec<u8>, s: &str, limit: usize) -> bool {
    let mut utf8 = [0u8; 4];
    for c in s.chars() {
        let cp = u32::from(c);
        let esc: [u8; 6];
        let encoded: &[u8] = match c {
            '"' => b"\\\"".as_slice(),
            '\\' => b"\\\\".as_slice(),
            '\n' => b"\\n".as_slice(),
            '\r' => b"\\r".as_slice(),
            '\t' => b"\\t".as_slice(),
            _ if cp < 0x20 => {
                esc = [b'\\', b'u', b'0', b'0', hex_digit(cp >> 4), hex_digit(cp)];
                esc.as_slice()
            }
            _ => c.encode_utf8(&mut utf8).as_bytes(),
        };
        if out.len().saturating_add(encoded.len()) > limit {
            return false;
        }
        out.extend_from_slice(encoded);
    }
    true
}
