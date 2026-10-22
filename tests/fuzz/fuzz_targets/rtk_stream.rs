//! Fuzz target `rtk_stream` (`polaris-tools-rtk`): arbitrary bytes piped through the stream filter,
//! the diagnostic parser, the JSON report and the ledger must never panic or overflow the stack
//! (deeply nested JSON lines included), and these invariants hold:
//!
//! - the decoded and the filtered text contain no ESC and no CR;
//! - a stream detected as plain text never gains text: every non-empty filtered line is a
//!   substring of some decoded input line;
//! - the JSON report is valid JSON whose `error_count` equals the number of issues;
//! - reconciling the ledger with the stream's diagnostics twice is a no-op the second time, and a
//!   ledger rendered from arbitrary text is a fixed point of `render(parse(..))`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use polaris_tools_rtk::{
    Diagnostic, Ledger, StreamFormat, decode_stream, detect_format, filter_text, parse_diagnostics,
    render_json_report,
};

fn check(data: &[u8]) {
    let decoded = decode_stream(data);
    assert!(!decoded.contains('\u{1b}') && !decoded.contains('\r'));

    let Ok(filtered) = filter_text(data) else {
        return;
    };
    assert!(!filtered.contains('\u{1b}') && !filtered.contains('\r'));
    if detect_format(data) == StreamFormat::Text {
        for line in filtered.lines().filter(|l| !l.is_empty()) {
            assert!(
                decoded.lines().any(|input| input.contains(line)),
                "filtered line is not part of the input: {line:?}"
            );
        }
    }

    let format = detect_format(data);
    let Ok(diagnostics) = parse_diagnostics(data, format) else {
        return;
    };
    if let Ok(report) = render_json_report(&diagnostics) {
        let value: serde_json::Value =
            serde_json::from_str(&report).expect("the report is valid JSON");
        let issues = diagnostics
            .iter()
            .filter(|d| Diagnostic::is_issue(d))
            .count();
        assert_eq!(value.get("error_count"), Some(&serde_json::json!(issues)));
    }

    let now = jiff::Timestamp::UNIX_EPOCH;
    let mut ledger = Ledger::default();
    ledger.reconcile(&diagnostics, now);
    let once = ledger.render();
    let again = ledger.reconcile(&diagnostics, now);
    assert_eq!((again.newly_opened, again.newly_closed), (0, 0));
    assert_eq!(ledger.render(), once);

    let reparsed = Ledger::parse(&String::from_utf8_lossy(data)).render();
    assert_eq!(Ledger::parse(&reparsed).render(), reparsed);
}

fuzz_target!(|data: &[u8]| check(data));
