//! Record types shared by the sinks, the fixed-size slot used by the ring and the queue, and the
//! text / JSON line rendering.

use std::fmt;
use std::time::SystemTime;

pub use tracing::Level;

/// Smallest line limit any sink accepts, in bytes (target and message together).
pub const MIN_LINE_BYTES: usize = 256;

/// Longest target kept in a slot, in bytes. A longer target is cut and the line flagged truncated.
pub const MAX_TARGET_BYTES: usize = 96;

/// A borrowed, already redacted record handed to a sink. Nothing here allocates.
#[derive(Debug, Clone, Copy)]
pub struct LogRecordRef<'a> {
    /// When the event happened.
    pub time: SystemTime,
    /// Severity.
    pub level: Level,
    /// The tracing target (module path unless overridden).
    pub target: &'a str,
    /// Redacted message: `text key=value ...`, or a JSON object in JSON mode.
    pub message: &'a str,
}

/// An owned log line, as read back from a ring or the drain queue (cold path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    /// Position of the line in its sink (counts every push, including overwritten lines).
    pub seq: u64,
    /// When the event happened.
    pub time: SystemTime,
    /// Severity.
    pub level: Level,
    /// The tracing target.
    pub target: String,
    /// Redacted message.
    pub message: String,
    /// The target or message was cut to fit the sink's fixed line limit.
    pub truncated: bool,
}

impl fmt::Display for LogLine {
    /// `2023-11-14T22:13:20.123Z INFO  target: message`, with ` [truncated]` appended when cut.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {:<5} {}: {}",
            format_timestamp(self.time),
            self.level.as_str(),
            self.target,
            self.message
        )?;
        if self.truncated {
            f.write_str(" [truncated]")?;
        }
        Ok(())
    }
}

impl LogLine {
    /// One JSON object: `{"ts":..,"level":..,"target":..,"fields":<message>}`. The message is
    /// embedded verbatim, so it must already be a JSON object (JSON mode guarantees that).
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(self.message.len() + self.target.len() + 80);
        out.push_str("{\"ts\":\"");
        out.push_str(&format_timestamp(self.time));
        out.push_str("\",\"level\":\"");
        out.push_str(self.level.as_str());
        out.push_str("\",\"target\":\"");
        escape_json_into(&mut out, &self.target);
        out.push_str("\",\"fields\":");
        out.push_str(&self.message);
        out.push('}');
        out
    }
}

/// Appends `s` to `out` with JSON string escaping (no surrounding quotes).
pub(crate) fn escape_json_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", u32::from(c)));
            }
            c => out.push(c),
        }
    }
}

/// Renders a time as RFC 3339 UTC with milliseconds (`2023-11-14T22:13:20.123Z`). Times before the
/// Unix epoch render as the epoch.
#[must_use]
pub fn format_timestamp(time: SystemTime) -> String {
    let since = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = since.as_secs();
    let millis = since.subsec_millis();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to (year, month, day) in the proleptic Gregorian calendar
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

/// The longest prefix of `s` that is at most `max` bytes and ends on a character boundary.
pub(crate) fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or_default()
}

/// One pre-allocated slot of a ring or queue: target and message bytes share one buffer whose
/// capacity is the line limit, so filling a slot never allocates.
#[derive(Debug)]
pub(crate) struct Entry {
    seq: u64,
    time: SystemTime,
    level: Level,
    target_len: usize,
    truncated: bool,
    buf: Vec<u8>,
}

impl Entry {
    /// An empty slot with room for `limit` bytes.
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            seq: 0,
            time: SystemTime::UNIX_EPOCH,
            level: Level::INFO,
            target_len: 0,
            truncated: false,
            buf: Vec::with_capacity(limit),
        }
    }

    /// Overwrites the slot with `rec`, cutting target and message to fit `limit` bytes in total.
    pub(crate) fn fill(&mut self, seq: u64, rec: &LogRecordRef<'_>, limit: usize) {
        let target = clip(rec.target, MAX_TARGET_BYTES.min(limit / 2));
        let message = clip(rec.message, limit.saturating_sub(target.len()));
        self.buf.clear();
        self.buf.extend_from_slice(target.as_bytes());
        self.buf.extend_from_slice(message.as_bytes());
        self.seq = seq;
        self.time = rec.time;
        self.level = rec.level;
        self.target_len = target.len();
        self.truncated = target.len() < rec.target.len() || message.len() < rec.message.len();
    }

    /// An owned copy of the slot (cold path).
    pub(crate) fn to_line(&self) -> LogLine {
        let (target, message) = self.buf.split_at(self.target_len.min(self.buf.len()));
        LogLine {
            seq: self.seq,
            time: self.time,
            level: self.level,
            target: String::from_utf8_lossy(target).into_owned(),
            message: String::from_utf8_lossy(message).into_owned(),
            truncated: self.truncated,
        }
    }
}
