//! Line Format (descriptor: specs/descriptors/log_line_format.json).

use std::time::{Duration, SystemTime};

use polaris_log::{Level, LogLine, format_timestamp};

fn at(secs: u64, millis: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(secs) + Duration::from_millis(millis)
}

fn line(level: Level, target: &str, message: &str, truncated: bool) -> LogLine {
    LogLine {
        seq: 0,
        time: at(1_700_000_000, 123),
        level,
        target: target.to_string(),
        message: message.to_string(),
        truncated,
    }
}

#[test]
fn timestamp_known_values() {
    assert_eq!(format_timestamp(at(0, 0)), "1970-01-01T00:00:00.000Z");
    assert_eq!(
        format_timestamp(at(1_700_000_000, 123)),
        "2023-11-14T22:13:20.123Z"
    );
    // Leap day and a century that is not a leap year boundary.
    assert_eq!(
        format_timestamp(at(1_709_164_800, 999)),
        "2024-02-29T00:00:00.999Z"
    );
    assert_eq!(
        format_timestamp(at(4_102_444_800, 0)),
        "2100-01-01T00:00:00.000Z"
    );
    assert_eq!(
        format_timestamp(at(951_782_400, 0)),
        "2000-02-29T00:00:00.000Z"
    );
}

#[test]
fn timestamp_before_epoch_clamps_to_epoch() {
    let before = SystemTime::UNIX_EPOCH - Duration::from_secs(10);
    assert_eq!(format_timestamp(before), "1970-01-01T00:00:00.000Z");
}

#[test]
fn text_line_layout() {
    let l = line(Level::INFO, "polaris::kernel", "started", false);
    assert_eq!(
        l.to_string(),
        "2023-11-14T22:13:20.123Z INFO  polaris::kernel: started"
    );
    let w = line(Level::ERROR, "x", "boom", false);
    assert_eq!(w.to_string(), "2023-11-14T22:13:20.123Z ERROR x: boom");
}

#[test]
fn text_line_marks_truncation() {
    let l = line(Level::WARN, "x", "abc", true);
    assert_eq!(
        l.to_string(),
        "2023-11-14T22:13:20.123Z WARN  x: abc [truncated]"
    );
}

#[test]
fn json_line_wraps_the_fields_object() {
    let l = line(Level::DEBUG, "polaris::net", r#"{"k":"v"}"#, false);
    assert_eq!(
        l.to_json(),
        r#"{"ts":"2023-11-14T22:13:20.123Z","level":"DEBUG","target":"polaris::net","fields":{"k":"v"}}"#
    );
}

#[test]
fn json_line_escapes_the_target() {
    let l = line(Level::INFO, "a\"b\\c\nd\u{1}", "{}", false);
    assert_eq!(
        l.to_json(),
        r#"{"ts":"2023-11-14T22:13:20.123Z","level":"INFO","target":"a\"b\\c\nd\u0001","fields":{}}"#
    );
}
