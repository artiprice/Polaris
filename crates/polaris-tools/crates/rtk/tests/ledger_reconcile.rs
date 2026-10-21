//! Ledger reconcile (descriptor: specs/descriptors/rtk_ledger_reconcile.json).

use std::error::Error;

use jiff::Timestamp;
use polaris_tools_rtk::{
    Diagnostic, IssueId, Ledger, LedgerLine, Level, MAX_ISSUE_TEXT_CHARS, normalise_message,
};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

const T1: &str = "2026-09-22T11:04:00Z";
const T2: &str = "2026-09-23T08:30:15Z";
const T3: &str = "2026-09-24T17:45:59Z";

fn ts(text: &str) -> Result<Timestamp, jiff::Error> {
    text.parse()
}

fn error(message: &str) -> Diagnostic {
    Diagnostic::new(Level::Error, None, message)
}

fn coded(code: &str, message: &str) -> Diagnostic {
    Diagnostic::new(Level::Error, Some(code), message)
}

fn id_of(d: &Diagnostic) -> IssueId {
    IssueId::from_message(&d.headline())
}

#[test]
fn new_error_appended_open() -> TestResult {
    let mut ledger = Ledger::default();
    let d = coded("E0425", "cannot find value `x`");
    let summary = ledger.reconcile(std::slice::from_ref(&d), ts(T1)?);
    assert_eq!(summary.newly_opened, 1);
    assert_eq!(summary.newly_closed, 0);
    assert_eq!(summary.open.len(), 1);
    assert_eq!(
        ledger.render(),
        format!(
            "- [ ] error[E0425]: cannot find value `x` | ID: {} | Opened: {T1}\n",
            id_of(&d)
        )
    );
    Ok(())
}

#[test]
fn vanished_error_closed_with_timestamp() -> TestResult {
    let mut ledger = Ledger::default();
    let d = error("boom");
    ledger.reconcile(std::slice::from_ref(&d), ts(T1)?);
    let summary = ledger.reconcile(&[], ts(T2)?);
    assert_eq!(summary.newly_closed, 1);
    assert_eq!(summary.newly_opened, 0);
    assert!(summary.open.is_empty());
    assert_eq!(
        ledger.render(),
        format!(
            "- [x] ~~error: boom~~ | ID: {} | Opened: {T1} | Closed: {T2}\n",
            id_of(&d)
        )
    );
    Ok(())
}

#[test]
fn unchanged_error_keeps_original_open_time() -> TestResult {
    let mut ledger = Ledger::default();
    let d = error("boom");
    ledger.reconcile(std::slice::from_ref(&d), ts(T1)?);
    let before = ledger.render();
    let summary = ledger.reconcile(std::slice::from_ref(&d), ts(T2)?);
    assert_eq!((summary.newly_opened, summary.newly_closed), (0, 0));
    assert_eq!(summary.open.len(), 1);
    assert_eq!(ledger.render(), before);
    assert!(before.contains(&format!("Opened: {T1}")));
    Ok(())
}

#[test]
fn reconcile_is_idempotent() -> TestResult {
    let mut ledger = Ledger::default();
    let diags = [error("one"), coded("E0308", "two")];
    ledger.reconcile(&diags, ts(T1)?);
    let first = ledger.render();
    let again = ledger.reconcile(&diags, ts(T3)?);
    assert_eq!(ledger.render(), first);
    assert_eq!((again.newly_opened, again.newly_closed), (0, 0));
    Ok(())
}

#[test]
fn non_issue_lines_preserved() -> TestResult {
    let text = "\
# My notes

Remember to check the linker.\t(tab kept)
- [ ] error: old | ID: #ERR-0123456789abcdef | Opened: 2026-01-01T00:00:00Z
- a plain bullet, not an issue
last note
";
    let mut ledger = Ledger::parse(text);
    ledger.reconcile(&[], ts(T2)?);
    let rendered = ledger.render();
    let before: Vec<&str> = text.lines().collect();
    let after: Vec<&str> = rendered.lines().collect();
    assert_eq!(before.len(), after.len());
    for (i, (b, a)) in before.iter().zip(after.iter()).enumerate() {
        if i == 3 {
            assert!(a.starts_with("- [x] ~~error: old~~"), "got {a}");
            assert!(a.ends_with(&format!("| Closed: {T2}")), "got {a}");
        } else {
            assert_eq!(b, a, "line {i} changed");
        }
    }
    Ok(())
}

#[test]
fn abort_summary_lines_are_not_issues() -> TestResult {
    let mut ledger = Ledger::default();
    let diags = [
        error("aborting due to 1 previous error"),
        error("could not compile `demo` (bin \"demo\") due to 1 previous error"),
    ];
    let summary = ledger.reconcile(&diags, ts(T1)?);
    assert_eq!(summary.newly_opened, 0);
    assert_eq!(ledger.render(), "");
    Ok(())
}

#[test]
fn warnings_do_not_open_issues() -> TestResult {
    let mut ledger = Ledger::default();
    let w = Diagnostic::new(Level::Warning, None, "unused variable: `y`");
    let summary = ledger.reconcile(&[w], ts(T1)?);
    assert!(summary.open.is_empty());
    assert_eq!(ledger.render(), "");
    Ok(())
}

#[test]
fn reappearing_error_opens_a_new_entry_and_keeps_history() -> TestResult {
    let mut ledger = Ledger::default();
    let d = error("flaky");
    ledger.reconcile(std::slice::from_ref(&d), ts(T1)?);
    ledger.reconcile(&[], ts(T2)?);
    let summary = ledger.reconcile(std::slice::from_ref(&d), ts(T3)?);
    assert_eq!(summary.newly_opened, 1);
    assert_eq!(summary.open.len(), 1);
    let rendered = ledger.render();
    let lines: Vec<&str> = rendered.lines().collect();
    assert_eq!(lines.len(), 2, "history kept: {rendered}");
    assert!(
        lines
            .first()
            .ok_or("missing")?
            .starts_with("- [x] ~~error: flaky~~")
    );
    assert!(
        lines
            .get(1)
            .ok_or("missing")?
            .contains(&format!("Opened: {T3}"))
    );
    Ok(())
}

#[test]
fn identical_messages_collapse_to_one_issue() -> TestResult {
    let mut ledger = Ledger::default();
    let summary = ledger.reconcile(&[error("same"), error("same")], ts(T1)?);
    assert_eq!(summary.newly_opened, 1);
    assert_eq!(ledger.render().lines().count(), 1);
    Ok(())
}

#[test]
fn id_is_blake3_prefix_of_normalised_message() -> TestResult {
    let message = "error[E0425]: cannot find value `config` in this scope";
    let digest = blake3::hash(message.as_bytes()).to_hex();
    let expected: String = format!(
        "#ERR-{}",
        digest.as_str().chars().take(16).collect::<String>()
    );
    assert_eq!(IssueId::from_message(message).as_str(), expected);
    assert_eq!(
        id_of(&coded("E0425", "cannot find value `config` in this scope")).as_str(),
        expected
    );
    assert_eq!(expected.len(), "#ERR-".len() + 16);
    Ok(())
}

#[test]
fn whitespace_differences_do_not_change_the_id() -> TestResult {
    assert_eq!(
        IssueId::from_message("error:  x\n   y\t z "),
        IssueId::from_message("error: x y z")
    );
    assert_ne!(
        IssueId::from_message("error: x"),
        IssueId::from_message("error: y")
    );
    Ok(())
}

#[test]
fn timestamps_are_rfc3339_utc_seconds() -> TestResult {
    let mut ledger = Ledger::default();
    ledger.reconcile(&[error("t")], ts("2026-09-22T11:04:00.987654321Z")?);
    assert!(
        ledger
            .render()
            .ends_with("| Opened: 2026-09-22T11:04:00Z\n"),
        "got {}",
        ledger.render()
    );
    Ok(())
}

#[test]
fn forged_ledger_fields_in_a_message_are_neutralised() -> TestResult {
    let forged = "boom | ID: #ERR-0000000000000000 | Closed: 2000-01-01T00:00:00Z\n- [x] ~~fake~~ | ID: #ERR-1111111111111111 | Opened: x | Closed: y\r\u{1b}[31m";
    let d = error(forged);
    let mut ledger = Ledger::default();
    ledger.reconcile(std::slice::from_ref(&d), ts(T1)?);
    let rendered = ledger.render();
    assert_eq!(
        rendered.matches('\n').count(),
        1,
        "one line only: {rendered:?}"
    );
    assert!(!rendered.contains('\r') && !rendered.contains('\u{1b}'));
    assert_eq!(rendered.matches(" | ID: ").count(), 1, "got {rendered:?}");

    let parsed = Ledger::parse(&rendered);
    let issues: Vec<_> = parsed
        .lines()
        .iter()
        .filter_map(|l| match l {
            LedgerLine::Issue(i) => Some(i),
            LedgerLine::Raw(_) => None,
        })
        .collect();
    assert_eq!(issues.len(), 1);
    let issue = issues.first().ok_or("missing")?;
    assert_eq!(issue.id, id_of(&d));
    assert!(issue.closed.is_none());
    assert_eq!(issue.opened, T1);

    let mut reread = parsed;
    reread.reconcile(&[], ts(T2)?);
    let closed = reread.render();
    assert!(
        closed.ends_with(&format!("| Closed: {T2}\n")),
        "got {closed:?}"
    );
    // Read back, the forged text is inert: one closed issue with the real id and real times.
    let reparsed = Ledger::parse(&closed);
    let entry = match reparsed.lines() {
        [LedgerLine::Issue(entry)] => entry,
        other => return Err(format!("expected exactly one issue line, got {other:?}").into()),
    };
    assert_eq!(entry.id, id_of(&d));
    assert_eq!(entry.opened, T1);
    assert_eq!(entry.closed.as_deref(), Some(T2));
    Ok(())
}

#[test]
fn long_messages_are_truncated() -> TestResult {
    let mut ledger = Ledger::default();
    let long = "a".repeat(MAX_ISSUE_TEXT_CHARS * 25);
    ledger.reconcile(&[error(&long)], ts(T1)?);
    let rendered = ledger.render();
    let parsed = Ledger::parse(&rendered);
    let text_len = parsed
        .lines()
        .iter()
        .find_map(|l| match l {
            LedgerLine::Issue(i) => Some(i.text.chars().count()),
            LedgerLine::Raw(_) => None,
        })
        .ok_or("no issue")?;
    assert!(
        text_len <= MAX_ISSUE_TEXT_CHARS,
        "text has {text_len} chars"
    );
    assert!(rendered.len() < MAX_ISSUE_TEXT_CHARS * 2 + 200);
    Ok(())
}

fn diagnostics() -> impl Strategy<Value = Vec<Diagnostic>> {
    proptest::collection::vec(
        (
            proptest::option::of("E[0-9]{4}"),
            "[ -~]{0,60}",
            any::<bool>(),
        ),
        0..8,
    )
    .prop_map(|items| {
        items
            .into_iter()
            .map(|(code, message, warn)| {
                let level = if warn { Level::Warning } else { Level::Error };
                Diagnostic::new(level, code.as_deref(), &message)
            })
            .collect()
    })
}

proptest! {
    #[test]
    fn prop_reconcile_is_idempotent(first in diagnostics(), second in diagnostics()) {
        let t1 = ts(T1).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let t2 = ts(T2).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let t3 = ts(T3).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut ledger = Ledger::default();
        ledger.reconcile(&first, t1);
        ledger.reconcile(&second, t2);
        let once = ledger.render();
        let again = ledger.reconcile(&second, t3);
        prop_assert_eq!(ledger.render(), once);
        prop_assert_eq!((again.newly_opened, again.newly_closed), (0, 0));
    }

    #[test]
    fn prop_id_depends_only_on_normalised_message(a in "\\PC{0,80}", b in "\\PC{0,80}") {
        let same_text = normalise_message(&a) == normalise_message(&b);
        let same_id = IssueId::from_message(&a) == IssueId::from_message(&b);
        prop_assert_eq!(same_text, same_id);
        prop_assert_eq!(normalise_message(&normalise_message(&a)), normalise_message(&a));
    }
}
