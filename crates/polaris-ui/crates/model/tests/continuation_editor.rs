//! Continuation Editor test cases.

use std::error::Error;

use polaris_ui_model::composer::{Composer, MAX_TEXT_BYTES};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn revision_tracks_cursor_edits_and_undo() -> TestResult {
    let mut composer = Composer::new();
    let initial = composer.revision();
    composer.insert("hello")?;
    let after_insert = composer.revision();
    assert!(after_insert > initial);

    composer.set_cursor(2);
    let after_move = composer.revision();
    assert!(after_move > after_insert);

    composer.insert("!")?;
    let after_edit = composer.revision();
    assert!(after_edit > after_move);
    assert!(composer.undo());
    assert!(composer.revision() > after_edit);
    assert_eq!(composer.text(), "hello");
    Ok(())
}

#[test]
fn selection_replaces_text_and_undo_restores_selection() -> TestResult {
    let mut c = Composer::new();
    c.insert("hello world")?;
    c.set_selection(6..11);
    assert_eq!(c.selection(), Some(6..11));
    c.insert("you")?;
    assert_eq!(c.text(), "hello you");
    assert_eq!(c.selection(), None);
    assert!(c.undo());
    assert_eq!(c.text(), "hello world");
    assert_eq!(c.selection(), Some(6..11));
    Ok(())
}

#[test]
fn word_navigation_and_deletion_respect_graphemes_and_chips() -> TestResult {
    let mut c = Composer::new();
    c.insert("one e\u{301} two")?;
    c.move_word_left();
    assert_eq!(c.cursor(), 8);
    c.delete_word_backward();
    assert_eq!(c.text(), "one two");
    let mut chip = Composer::new();
    chip.insert("prefix ")?;
    chip.insert("a\nb\nc\nd\ne\nf")?;
    let range = chip.chips().first().ok_or("chip")?.range.clone();
    chip.move_end();
    chip.move_word_left();
    assert_eq!(chip.cursor(), range.start);
    let mut right = Composer::new();
    right.insert("alpha beta")?;
    right.move_document_home();
    right.move_word_right();
    assert_eq!(right.cursor(), 6);
    right.delete_word_forward();
    assert_eq!(right.text(), "alpha ");
    Ok(())
}

#[test]
fn line_and_document_home_end_are_distinct() -> TestResult {
    let mut c = Composer::new();
    c.insert("first\nsecond\nthird")?;
    c.move_document_home();
    c.move_line_end();
    assert_eq!(c.cursor(), 5);
    c.set_cursor(6);
    c.move_line_home();
    assert_eq!(c.cursor(), 6);
    c.move_document_end();
    assert_eq!(c.cursor(), c.text().len());
    c.move_document_home_with_selection();
    assert_eq!(c.selection(), Some(0..c.text().len()));
    Ok(())
}

#[test]
fn history_restores_saved_draft_selection() -> TestResult {
    let mut c = Composer::new();
    c.insert("sent one")?;
    c.submit()?;
    c.insert("draft text")?;
    c.set_selection(6..10);
    assert!(c.history_prev(true));
    assert!(c.history_next(true));
    assert_eq!(c.text(), "draft text");
    assert_eq!(c.selection(), Some(6..10));
    Ok(())
}

#[test]
fn selected_chip_copy_replace_and_byte_budget_are_consistent() -> TestResult {
    let payload = "a\nb\nc\nd\ne\nf";
    let mut c = Composer::new();
    c.insert("prefix ")?;
    c.insert(payload)?;
    let range = c.chips().first().ok_or("pasted text chip")?.range.clone();
    c.set_selection(range.clone());
    assert_eq!(c.selected_text().as_deref(), Some(payload));
    c.insert("short")?;
    assert_eq!(c.text(), "prefix short");
    assert!(c.undo());
    assert_eq!(c.selection(), Some(range));
    assert_eq!(c.cut().as_deref(), Some(payload));
    assert_eq!(c.text(), "prefix ");

    let mut full = Composer::new();
    let original = "a".repeat(MAX_TEXT_BYTES);
    full.insert(&original)?;
    full.select_all();
    full.insert(&original)?;
    assert_eq!(full.logical_len(), MAX_TEXT_BYTES);
    Ok(())
}

#[test]
fn shift_navigation_and_select_all_keep_legal_boundaries() -> TestResult {
    let mut c = Composer::new();
    c.insert("e\u{301}👨\u{200d}👩")?;
    c.move_document_home();
    c.move_right_with_selection(true);
    c.move_right_with_selection(true);
    assert_eq!(c.selection(), Some(0.."e\u{301}👨\u{200d}👩".len()));
    c.select_all();
    assert_eq!(c.selection(), Some(0..c.text().len()));
    assert!(c.delete());
    assert!(c.text().is_empty());
    assert!(c.undo());
    assert_eq!(c.selection(), Some(0..c.text().len()));
    Ok(())
}

#[test]
fn double_click_selection_tracks_unicode_words_whitespace_and_atomic_chips() -> TestResult {
    let mut c = Composer::new();
    c.insert("café foo_bar!")?;
    assert!(c.select_word_at(2));
    assert_eq!(c.selected_text().as_deref(), Some("café"));
    assert!(c.select_word_at(5));
    assert_eq!(c.selected_text().as_deref(), Some(" "));
    assert!(c.select_word_at(10));
    assert_eq!(c.selected_text().as_deref(), Some("foo_bar"));
    let punctuation = c.text().len().saturating_sub(1);
    assert!(c.select_word_at(punctuation));
    assert_eq!(c.selected_text().as_deref(), Some("!"));

    let mut chip = Composer::new();
    chip.insert("before ")?;
    chip.insert("a\nb\nc\nd\ne\nf")?;
    let range = chip.chips().first().ok_or("text chip")?.range.clone();
    assert!(chip.select_word_at(range.start.saturating_add(2)));
    assert_eq!(chip.selection(), Some(range));
    Ok(())
}
