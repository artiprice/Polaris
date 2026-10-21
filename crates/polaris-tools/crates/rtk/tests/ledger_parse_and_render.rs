//! Ledger parse and render (descriptor: specs/descriptors/rtk_ledger_parse_and_render.json).

use std::error::Error;

use polaris_tools_rtk::{IssueLine, Ledger, LedgerLine};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

const OPEN: &str = "- [ ] error[E0425]: cannot find value `config` in this scope | ID: #ERR-a1b2c3d4e5f60718 | Opened: 2026-09-22T11:04:00Z";
const CLOSED: &str = "- [x] ~~error[E0308]: mismatched types~~ | ID: #ERR-0123456789abcdef | Opened: 2026-09-21T09:12:00Z | Closed: 2026-09-22T11:04:00Z";

fn raw_count(ledger: &Ledger) -> usize {
    ledger
        .lines()
        .iter()
        .filter(|l| matches!(l, LedgerLine::Raw(_)))
        .count()
}

fn issues(ledger: &Ledger) -> Vec<&IssueLine> {
    ledger
        .lines()
        .iter()
        .filter_map(|l| match l {
            LedgerLine::Issue(i) => Some(i),
            LedgerLine::Raw(_) => None,
        })
        .collect()
}

#[test]
fn open_and_closed_lines_parse() -> TestResult {
    let text = format!("{OPEN}\n{CLOSED}\n");
    let ledger = Ledger::parse(&text);
    let found = issues(&ledger);
    assert_eq!(found.len(), 2);
    let open = found.first().ok_or("missing open")?;
    assert_eq!(open.id.as_str(), "#ERR-a1b2c3d4e5f60718");
    assert_eq!(
        open.text,
        "error[E0425]: cannot find value `config` in this scope"
    );
    assert_eq!(open.opened, "2026-09-22T11:04:00Z");
    assert!(open.is_open());
    let closed = found.get(1).ok_or("missing closed")?;
    assert_eq!(closed.id.as_str(), "#ERR-0123456789abcdef");
    assert_eq!(closed.text, "error[E0308]: mismatched types");
    assert_eq!(closed.closed.as_deref(), Some("2026-09-22T11:04:00Z"));
    assert!(!closed.is_open());
    assert_eq!(ledger.render(), text);
    Ok(())
}

#[test]
fn raw_lines_preserved_byte_for_byte() -> TestResult {
    let text = "# Notes\n\n\tindented\twith tabs  \n| table | row |\n- [ ] no id here\nunicode: größe — ✓\n   \n";
    let ledger = Ledger::parse(text);
    assert_eq!(raw_count(&ledger), 7);
    assert_eq!(ledger.render(), text);
    Ok(())
}

#[test]
fn crlf_ledger_renders_with_lf() -> TestResult {
    let ledger = Ledger::parse(&format!("a note\r\n{OPEN}\r\n"));
    assert_eq!(issues(&ledger).len(), 1);
    assert_eq!(ledger.render(), format!("a note\n{OPEN}\n"));
    Ok(())
}

#[test]
fn malformed_issue_lines_stay_raw() -> TestResult {
    let text = "\
- [ ] missing id | Opened: 2026-09-22T11:04:00Z
- [x] ~~strike~~ | ID: #ERR-0123456789abcdef
- [ ] text | ID: #ERR-nothex | Opened: 2026-09-22T11:04:00Z
- [x] not struck | ID: #ERR-0123456789abcdef | Opened: a | Closed: b
";
    let ledger = Ledger::parse(text);
    assert!(issues(&ledger).is_empty());
    assert_eq!(raw_count(&ledger), 4);
    assert_eq!(ledger.render(), text);
    Ok(())
}

#[test]
fn legacy_ids_without_err_prefix_stay_raw() -> TestResult {
    let text = "- [ ] error: x | ID: a1b2c3d4 | Opened: 2026-09-22T11:04:00Z\n";
    let ledger = Ledger::parse(text);
    assert!(issues(&ledger).is_empty());
    assert_eq!(ledger.render(), text);
    Ok(())
}

#[test]
fn render_of_empty_ledger_is_empty() -> TestResult {
    assert_eq!(Ledger::parse("").render(), "");
    assert_eq!(Ledger::default().render(), "");
    assert!(Ledger::parse("").lines().is_empty());
    Ok(())
}

fn ledger_line() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(OPEN.to_owned()),
        Just(CLOSED.to_owned()),
        "[ -~]{0,30}".prop_map(|t| format!("- [ ] {t} | ID: #ERR-0123456789abcdef | Opened: x")),
        "[ -~]{0,30}".prop_map(|t| format!(
            "- [x] ~~{t}~~ | ID: #ERR-fedcba9876543210 | Opened: x | Closed: y"
        )),
        "\\PC{0,50}",
        "[ -~]{0,10}\\r{0,3}",
    ]
}

proptest! {
    #[test]
    fn prop_render_parse_render_is_a_fixed_point(
        lines in proptest::collection::vec(ledger_line(), 0..20),
        trailing_newline in any::<bool>(),
    ) {
        let mut text = lines.join("\n");
        if trailing_newline {
            text.push('\n');
        }
        let once = Ledger::parse(&text).render();
        let twice = Ledger::parse(&once).render();
        prop_assert_eq!(once, twice);
    }
}
