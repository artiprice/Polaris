//! Composer Buffer (descriptor: specs/descriptors/uimodel_composer_buffer.json).

use std::error::Error;

use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_ui_model::composer::{
    Attachment, Composer, ImageRef, MAX_ATTACHMENTS_TOTAL, MAX_HISTORY_ENTRIES, MAX_IMAGE_RGBA,
    MAX_RETAINED_BYTES, MAX_TEXT_BYTES, MAX_UNDO_ENTRIES,
};
use polaris_ui_model::error::ComposerError;
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn six_lines() -> String {
    "1\n2\n3\n4\n5\n6".to_owned()
}

#[test]
fn long_paste_becomes_chip() -> TestResult {
    let mut c = Composer::new();
    c.insert(&six_lines())?;
    assert_eq!(c.text(), "[Pasted Content 11 chars]");
    assert_eq!(c.cursor(), c.text().len());
    let chips = c.chips();
    assert_eq!(chips.len(), 1);
    let chip = chips.first().ok_or("one chip")?;
    assert_eq!(chip.range, 0..c.text().len());
    assert_eq!(
        chip.attachment,
        &Attachment::Text {
            chars: 11,
            payload: six_lines().into()
        }
    );

    // Exactly 5 logical lines stays literal, a trailing newline does not add a line.
    let mut five = Composer::new();
    five.insert("1\n2\n3\n4\n5")?;
    assert_eq!(five.text(), "1\n2\n3\n4\n5");
    let mut five_nl = Composer::new();
    five_nl.insert("1\n2\n3\n4\n5\n")?;
    assert!(five_nl.chips().is_empty());

    // 500 chars stay literal, 501 become a chip; the label counts chars not bytes.
    let mut limit = Composer::new();
    limit.insert(&"é".repeat(500))?;
    assert!(limit.chips().is_empty());
    assert_eq!(limit.text().len(), 1000);
    let mut over = Composer::new();
    over.insert(&"é".repeat(501))?;
    assert_eq!(over.text(), "[Pasted Content 501 chars]");
    Ok(())
}

#[test]
fn short_insert_stays_literal() -> TestResult {
    let mut c = Composer::new();
    c.insert("hello")?;
    c.insert(" wörld")?;
    assert_eq!(c.text(), "hello wörld");
    assert_eq!(c.cursor(), c.text().len());
    assert!(c.chips().is_empty());
    assert!(!c.is_empty());
    assert!(Composer::new().is_empty());
    Ok(())
}

#[test]
fn submit_trims_trailing_blank_rows() -> TestResult {
    let mut c = Composer::new();
    c.insert("hello\n\n  \n")?;
    assert_eq!(c.submit()?.text, "hello");
    c.insert("hello  \nworld\n\n\n")?;
    assert_eq!(c.submit()?.text, "hello  \nworld");
    c.insert("\n\nhi\n")?;
    assert_eq!(c.submit()?.text, "\n\nhi", "leading blank rows are kept");
    Ok(())
}

#[test]
fn submit_keeps_interior_blank_paragraphs() -> TestResult {
    let mut c = Composer::new();
    c.insert("a\n\nb\n\n\nc\n\n")?;
    assert_eq!(c.submit()?.text, "a\n\nb\n\n\nc");
    Ok(())
}

#[test]
fn submit_expands_chips() -> TestResult {
    let mut c = Composer::new();
    c.insert("see: ")?;
    c.insert(&six_lines())?;
    c.insert(" end ")?;
    let index = c.add_image(100)?;
    assert_eq!(index, 1);
    let submission = c.submit()?;
    assert_eq!(
        submission.text,
        format!("see: {} end [Image 1]", six_lines())
    );
    assert_eq!(
        submission.images,
        [ImageRef {
            index: 1,
            rgba_len: 100
        }]
    );
    // The composer is empty again afterwards.
    assert_eq!(c.text(), "");
    assert_eq!(c.cursor(), 0);
    assert!(c.chips().is_empty());
    assert_eq!(c.image_bytes_total(), 0);
    Ok(())
}

#[test]
fn empty_submit_rejected_and_state_kept() -> TestResult {
    let mut c = Composer::new();
    assert_eq!(c.submit(), Err(ComposerError::EmptySubmission));
    c.insert("  \n ")?;
    assert_eq!(c.submit(), Err(ComposerError::EmptySubmission));
    assert_eq!(c.text(), "  \n ");
    assert_eq!(c.cursor(), 4);
    Ok(())
}

#[test]
fn text_over_1mib_rejected() -> TestResult {
    let mut c = Composer::new();
    c.insert(&"a".repeat(MAX_TEXT_BYTES))?;
    assert_eq!(c.logical_len(), MAX_TEXT_BYTES);
    let before_text = c.text().to_owned();
    let before_cursor = c.cursor();
    let err = c.insert("b").err().ok_or("insert past the cap must fail")?;
    assert_eq!(
        err,
        ComposerError::TextTooLarge {
            limit: MAX_TEXT_BYTES,
            attempted: MAX_TEXT_BYTES + 1
        }
    );
    assert_eq!(err.kind(), ErrorKind::Permanent);
    assert_eq!(c.text(), before_text);
    assert_eq!(c.cursor(), before_cursor);
    assert!(c.newline().is_err(), "a newline counts toward the cap too");

    let mut fresh = Composer::new();
    assert!(fresh.insert(&"a".repeat(MAX_TEXT_BYTES + 1)).is_err());
    assert_eq!(fresh.text(), "");
    Ok(())
}

#[test]
fn chip_payload_counts_toward_text_cap() -> TestResult {
    let mut c = Composer::new();
    c.insert(&"x".repeat(MAX_TEXT_BYTES - 10))?;
    assert!(c.insert(&"y".repeat(20)).is_err());
    c.insert(&"y".repeat(10))?;
    assert_eq!(c.logical_len(), MAX_TEXT_BYTES);
    assert!(c.insert("z").is_err());
    Ok(())
}

#[test]
fn session_attachment_cap_enforced() -> TestResult {
    let mut c = Composer::new();
    c.add_image(MAX_IMAGE_RGBA)?;
    c.add_image(MAX_IMAGE_RGBA)?;
    assert_eq!(c.image_bytes_total(), MAX_ATTACHMENTS_TOTAL);
    let before = c.text().to_owned();
    assert_eq!(
        c.add_image(1),
        Err(ComposerError::AttachmentsTooLarge {
            total: MAX_ATTACHMENTS_TOTAL + 1,
            limit: MAX_ATTACHMENTS_TOTAL
        })
    );
    assert_eq!(c.text(), before);
    // Removing a chip frees its budget.
    assert!(c.backspace());
    assert_eq!(c.image_bytes_total(), MAX_IMAGE_RGBA);
    c.add_image(MAX_IMAGE_RGBA)?;
    assert!(c.add_image(1).is_err());
    Ok(())
}

#[test]
fn image_over_64mib_rejected() {
    let mut c = Composer::new();
    assert_eq!(
        c.add_image(MAX_IMAGE_RGBA + 1),
        Err(ComposerError::ImageTooLarge {
            len: MAX_IMAGE_RGBA + 1,
            limit: MAX_IMAGE_RGBA
        })
    );
    assert_eq!(c.add_image(0), Err(ComposerError::ImageEmpty));
    assert_eq!(c.text(), "");
    assert!(c.chips().is_empty());
}

#[test]
fn image_chip_label_and_numbering() -> TestResult {
    let mut c = Composer::new();
    assert_eq!(c.add_image(10)?, 1);
    assert_eq!(c.add_image(10)?, 2);
    assert_eq!(c.text(), "[Image 1][Image 2]");
    assert!(c.backspace());
    assert_eq!(c.text(), "[Image 1]");
    assert_eq!(c.add_image(10)?, 3, "labels stay unique within one draft");
    assert_eq!(c.text(), "[Image 1][Image 3]");
    let submission = c.submit()?;
    assert_eq!(submission.images.len(), 2);
    assert_eq!(c.add_image(10)?, 1, "numbering restarts after submit");
    Ok(())
}

#[test]
fn crlf_normalized_to_lf() -> TestResult {
    let mut c = Composer::new();
    c.insert("a\r\nb\rc\nd")?;
    assert_eq!(c.text(), "a\nb\nc\nd");
    assert!(!c.text().contains('\r'));
    Ok(())
}

#[test]
fn control_characters_are_stripped() -> TestResult {
    let mut c = Composer::new();
    c.insert("a\u{1b}[31mb\u{7}\tc\u{0}")?;
    assert_eq!(c.text(), "a[31mb\tc");
    Ok(())
}

#[test]
fn cursor_moves_by_grapheme() -> TestResult {
    let mut c = Composer::new();
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    c.insert(&format!("e\u{301}x{family}y"))?;
    let end = c.text().len();
    assert_eq!(c.cursor(), end);
    c.move_left();
    assert_eq!(c.cursor(), end - 1);
    c.move_left();
    assert_eq!(
        c.cursor(),
        end - 1 - family.len(),
        "the whole ZWJ family is one step"
    );
    c.move_left();
    assert_eq!(c.cursor(), 3, "x follows e + combining acute");
    c.move_left();
    assert_eq!(c.cursor(), 0, "e + combining acute is one step");
    c.move_left();
    assert_eq!(c.cursor(), 0);
    c.move_right();
    assert_eq!(c.cursor(), 3);
    c.move_end();
    assert_eq!(c.cursor(), end);
    c.move_right();
    assert_eq!(c.cursor(), end);
    c.move_home();
    assert_eq!(c.cursor(), 0);

    // A chip is crossed as one unit.
    let mut chip = Composer::new();
    chip.insert("ab")?;
    chip.insert(&six_lines())?;
    chip.insert("cd")?;
    let range = chip.chips().first().ok_or("chip")?.range.clone();
    chip.move_left();
    chip.move_left();
    assert_eq!(chip.cursor(), range.end);
    chip.move_left();
    assert_eq!(chip.cursor(), range.start);
    chip.move_right();
    assert_eq!(chip.cursor(), range.end);
    // Clicking into the middle of a chip or a char snaps to a boundary.
    chip.set_cursor(range.start + 3);
    assert_eq!(chip.cursor(), range.start);
    let mut accent = Composer::new();
    accent.insert("aé")?;
    accent.set_cursor(2);
    assert_eq!(accent.cursor(), 1);
    accent.set_cursor(99);
    assert_eq!(accent.cursor(), 3);
    Ok(())
}

#[test]
fn backspace_deletes_whole_grapheme() -> TestResult {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let mut c = Composer::new();
    c.insert("ae\u{301}")?;
    assert!(c.backspace());
    assert_eq!(c.text(), "a");
    c.insert(family)?;
    assert!(c.backspace());
    assert_eq!(c.text(), "a");
    assert!(c.backspace());
    assert!(!c.backspace(), "nothing left to delete");
    let mut d = Composer::new();
    d.insert("e\u{301}x")?;
    d.move_home();
    assert!(d.delete());
    assert_eq!(d.text(), "x");
    d.move_end();
    assert!(!d.delete());
    Ok(())
}

#[test]
fn backspace_after_chip_removes_chip_and_frees_budget() -> TestResult {
    let mut c = Composer::new();
    c.insert(&"x".repeat(MAX_TEXT_BYTES))?;
    assert_eq!(c.chips().len(), 1);
    assert!(c.backspace());
    assert_eq!(c.text(), "");
    assert!(c.chips().is_empty());
    assert_eq!(c.logical_len(), 0);
    c.insert(&"y".repeat(MAX_TEXT_BYTES))?;
    assert_eq!(c.logical_len(), MAX_TEXT_BYTES);
    // Delete (forward) removes the chip under the cursor as well.
    c.move_home();
    assert!(c.delete());
    assert!(c.chips().is_empty());
    Ok(())
}

#[test]
fn undo_redo_restores_text_cursor_and_chips() -> TestResult {
    let mut c = Composer::new();
    c.insert("ab")?;
    c.insert(&six_lines())?;
    c.insert("cd")?;
    assert!(c.undo());
    assert_eq!(c.text(), "ab[Pasted Content 11 chars]");
    assert!(c.undo());
    assert_eq!(c.text(), "ab");
    assert!(c.chips().is_empty());
    assert_eq!(c.cursor(), 2);
    assert!(c.undo());
    assert_eq!(c.text(), "");
    assert!(!c.undo());
    assert!(c.redo());
    assert!(c.redo());
    let chips = c.chips();
    assert_eq!(chips.len(), 1);
    assert_eq!(
        chips.first().map(|chip| chip.attachment),
        Some(&Attachment::Text {
            chars: 11,
            payload: six_lines().into()
        })
    );
    assert!(c.redo());
    assert_eq!(c.text(), "ab[Pasted Content 11 chars]cd");
    assert_eq!(c.cursor(), c.text().len());
    assert!(!c.redo());

    // A new edit after undo discards the redo history.
    assert!(c.undo());
    c.insert("z")?;
    assert!(!c.redo());

    // Typing groups into word-sized undo steps.
    let mut typing = Composer::new();
    typing.insert("hello")?;
    typing.insert(" ")?;
    typing.insert("world")?;
    assert!(typing.undo());
    assert_eq!(typing.text(), "hello ");
    assert!(typing.undo());
    assert_eq!(typing.text(), "");

    // Deleting a chip is undoable and brings the payload back.
    let mut deleted = Composer::new();
    deleted.insert(&six_lines())?;
    assert!(deleted.backspace());
    assert!(deleted.chips().is_empty());
    assert!(deleted.undo());
    assert_eq!(deleted.chips().len(), 1);
    assert_eq!(deleted.submit()?.text, six_lines());
    Ok(())
}

#[test]
fn expand_chip_inserts_payload_literally() -> TestResult {
    let mut c = Composer::new();
    c.insert("a ")?;
    c.insert(&six_lines())?;
    let id = c.chips().first().ok_or("chip")?.id;
    let before = c.logical_len();
    c.expand_chip(id)?;
    assert_eq!(c.text(), format!("a {}", six_lines()));
    assert!(c.chips().is_empty());
    assert_eq!(c.logical_len(), before);
    assert_eq!(c.cursor(), c.text().len());
    assert_eq!(c.expand_chip(id), Err(ComposerError::NoSuchChip));
    assert!(c.undo());
    assert_eq!(c.chips().len(), 1);
    Ok(())
}

#[test]
fn apply_completion_replaces_range() -> TestResult {
    let mut c = Composer::new();
    c.insert("use $web now")?;
    c.apply_completion(4..8, "$websearch")?;
    assert_eq!(c.text(), "use $websearch now");
    assert_eq!(c.cursor(), 14);
    assert_eq!(
        c.apply_completion(4..99, "x"),
        Err(ComposerError::InvalidRange)
    );
    let (start, end) = (9, 4);
    assert_eq!(
        c.apply_completion(start..end, "x"),
        Err(ComposerError::InvalidRange)
    );

    let mut accent = Composer::new();
    accent.insert("é$w")?;
    assert_eq!(
        accent.apply_completion(1..3, "x"),
        Err(ComposerError::InvalidRange)
    );

    let mut chip = Composer::new();
    chip.insert(&six_lines())?;
    assert_eq!(
        chip.apply_completion(3..6, "x"),
        Err(ComposerError::InvalidRange)
    );
    assert_eq!(chip.chips().len(), 1);
    Ok(())
}

/// One step of the model-free state machine used by the proptests.
#[derive(Debug, Clone)]
enum Op {
    Insert(String),
    Newline,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    SetCursor(usize),
    Undo,
    Redo,
    Image(usize),
    HistoryPrev(bool),
    HistoryNext(bool),
    Submit,
    ExpandFirstChip,
}

fn op_strategy() -> impl Strategy<Value = Op> {
    let text = prop_oneof![
        "[a-zA-Z0-9 \n\r]{0,40}",
        any::<String>(),
        "(a\n){0,10}",
        "x{0,600}",
        "(e\u{301}|\u{1F468}\u{200D}\u{1F469}|é|\r\n){0,8}",
    ];
    prop_oneof![
        6 => text.prop_map(Op::Insert).boxed(),
        1 => Just(Op::Newline).boxed(),
        2 => Just(Op::Backspace).boxed(),
        2 => Just(Op::Delete).boxed(),
        2 => Just(Op::Left).boxed(),
        2 => Just(Op::Right).boxed(),
        1 => Just(Op::Home).boxed(),
        1 => Just(Op::End).boxed(),
        1 => (0usize..100).prop_map(Op::SetCursor).boxed(),
        1 => Just(Op::Undo).boxed(),
        1 => Just(Op::Redo).boxed(),
        1 => (0usize..1000).prop_map(Op::Image).boxed(),
        1 => any::<bool>().prop_map(Op::HistoryPrev).boxed(),
        1 => any::<bool>().prop_map(Op::HistoryNext).boxed(),
        1 => Just(Op::Submit).boxed(),
        1 => Just(Op::ExpandFirstChip).boxed(),
    ]
}

fn apply(c: &mut Composer, op: &Op) {
    match op {
        Op::Insert(text) => {
            let _ = c.insert(text);
        }
        Op::Newline => {
            let _ = c.newline();
        }
        Op::Backspace => {
            let _ = c.backspace();
        }
        Op::Delete => {
            let _ = c.delete();
        }
        Op::Left => c.move_left(),
        Op::Right => c.move_right(),
        Op::Home => c.move_home(),
        Op::End => c.move_end(),
        Op::SetCursor(pos) => c.set_cursor(*pos),
        Op::Undo => {
            let _ = c.undo();
        }
        Op::Redo => {
            let _ = c.redo();
        }
        Op::Image(len) => {
            let _ = c.add_image(*len);
        }
        Op::HistoryPrev(row) => {
            let _ = c.history_prev(*row);
        }
        Op::HistoryNext(row) => {
            let _ = c.history_next(*row);
        }
        Op::Submit => {
            let _ = c.submit();
        }
        Op::ExpandFirstChip => {
            if let Some(id) = c.chips().first().map(|chip| chip.id) {
                let _ = c.expand_chip(id);
            }
        }
    }
}

proptest! {
    #[test]
    fn prop_cursor_always_on_char_boundary(ops in proptest::collection::vec(op_strategy(), 0..60)) {
        let mut c = Composer::new();
        for op in &ops {
            apply(&mut c, op);
            let text = c.text();
            prop_assert!(c.cursor() <= text.len(), "cursor past end after {op:?}");
            prop_assert!(text.is_char_boundary(c.cursor()), "cursor off a char boundary after {op:?}");
            prop_assert!(!text.contains('\r'));
            prop_assert!(c.logical_len() <= MAX_TEXT_BYTES);
            prop_assert!(c.image_bytes_total() <= MAX_ATTACHMENTS_TOTAL);
            let mut previous_end = 0;
            for chip in c.chips() {
                prop_assert!(chip.range.start >= previous_end, "chips overlap after {op:?}");
                prop_assert!(chip.range.start < chip.range.end);
                prop_assert!(chip.range.end <= text.len());
                prop_assert!(text.is_char_boundary(chip.range.start));
                prop_assert!(text.is_char_boundary(chip.range.end));
                let label = text.get(chip.range.clone()).unwrap_or("");
                prop_assert!(label.starts_with("[Pasted Content ") || label.starts_with("[Image "));
                prop_assert!(
                    c.cursor() <= chip.range.start || c.cursor() >= chip.range.end,
                    "cursor inside a chip after {op:?}"
                );
                previous_end = chip.range.end;
            }
        }
    }

    #[test]
    fn prop_submit_preserves_chip_payloads(
        pieces in proptest::collection::vec(
            prop_oneof![
                "[a-z]{1,10}",
                "[a-z]{1,10}(\n[a-z]{1,10}){5,8}",
                "[a-z]{501,700}",
            ],
            1..8,
        ),
    ) {
        let mut c = Composer::new();
        let mut expected_chips = 0;
        for piece in &pieces {
            c.insert(piece).map_err(|e| TestCaseError::fail(e.to_string()))?;
            if piece.lines().count() > 5 || piece.chars().count() > 500 {
                expected_chips += 1;
            }
        }
        prop_assert_eq!(c.chips().len(), expected_chips);
        let payloads: Vec<String> = c
            .chips()
            .iter()
            .filter_map(|chip| match chip.attachment {
                Attachment::Text { payload, .. } => Some(payload.to_string()),
                Attachment::Image { .. } => None,
            })
            .collect();
        let submission = c.submit().map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(&submission.text, &pieces.concat());
        for payload in payloads {
            prop_assert!(submission.text.contains(&payload));
        }
    }
}

#[test]
fn prepare_cancel_and_rejection_preserve_complete_draft() -> TestResult {
    let mut c = Composer::new();
    c.insert("before ")?;
    c.insert(&six_lines())?;
    let chips_before: Vec<_> = c
        .chips()
        .into_iter()
        .map(|chip| (chip.id, chip.range))
        .collect();
    let text_before = c.text().to_owned();
    c.move_left();
    let cursor_before = c.cursor();
    let prepared = c.prepare_submit()?;
    assert_eq!(
        prepared.submission().text,
        format!("before {}", six_lines())
    );
    c.cancel_submit(prepared);
    assert_eq!(c.text(), text_before);
    assert_eq!(c.cursor(), cursor_before);
    assert_eq!(
        c.chips()
            .into_iter()
            .map(|chip| (chip.id, chip.range))
            .collect::<Vec<_>>(),
        chips_before
    );
    let prepared = c.prepare_submit()?;
    assert_eq!(
        c.text(),
        text_before,
        "backend rejection drops prepared value without commit"
    );
    let committed = c.commit_submit(prepared)?;
    assert_eq!(committed.text, format!("before {}", six_lines()));
    assert!(c.is_empty());
    Ok(())
}

#[test]
fn stale_prepared_submission_cannot_clear_new_edits() -> TestResult {
    let mut c = Composer::new();
    c.insert("first")?;
    let prepared = c.prepare_submit()?;
    c.insert(" second")?;
    assert_eq!(
        c.commit_submit(prepared),
        Err(ComposerError::StalePreparedSubmission)
    );
    assert_eq!(c.text(), "first second");
    Ok(())
}

#[test]
fn undo_and_history_respect_retention_bounds() -> TestResult {
    let mut c = Composer::new();
    for index in 0..(MAX_HISTORY_ENTRIES + 5) {
        c.insert(&format!("entry-{index}"))?;
        c.submit()?;
    }
    assert_eq!(c.history_len(), MAX_HISTORY_ENTRIES);
    assert!(c.history_retained_bytes() <= MAX_RETAINED_BYTES);
    assert!(c.history_prev(true));
    assert!(c.text().starts_with("entry-"));
    let mut edits = Composer::new();
    for _ in 0..(MAX_UNDO_ENTRIES + 5) {
        edits.insert("x")?;
    }
    assert_eq!(edits.undo_len(), MAX_UNDO_ENTRIES);
    assert!(edits.undo_retained_bytes() <= MAX_RETAINED_BYTES);
    for _ in 0..MAX_UNDO_ENTRIES {
        assert!(edits.undo());
    }
    assert!(!edits.undo());
    Ok(())
}

#[test]
fn cloned_undo_snapshot_shares_paste_payload_storage() -> TestResult {
    use std::sync::Arc;
    let mut composer = Composer::new();
    composer.insert(&six_lines())?;
    let original_chips = composer.chips();
    let retained = match original_chips.first().map(|chip| chip.attachment) {
        Some(Attachment::Text { payload, .. }) => Arc::clone(payload),
        _ => return Err("paste chip missing".into()),
    };
    composer.insert("tail")?;
    assert!(composer.undo());
    let restored = composer.chips();
    let shared = match restored.first().map(|chip| chip.attachment) {
        Some(Attachment::Text { payload, .. }) => Arc::ptr_eq(&retained, payload),
        _ => false,
    };
    assert!(shared);
    Ok(())
}

#[test]
fn stale_prepared_submission_cannot_clear_cursor_change() -> TestResult {
    let mut composer = Composer::new();
    composer.insert("draft")?;
    let prepared = composer.prepare_submit()?;
    composer.move_left();
    assert_eq!(
        composer.commit_submit(prepared),
        Err(ComposerError::StalePreparedSubmission)
    );
    assert_eq!(composer.text(), "draft");
    assert_eq!(composer.cursor(), 4);
    Ok(())
}
