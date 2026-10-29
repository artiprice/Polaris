//! Prompt History (descriptor: specs/descriptors/uimodel_prompt_history.json).

use std::error::Error;

use polaris_ui_model::composer::Composer;
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn send(c: &mut Composer, text: &str) -> Result<(), Box<dyn Error>> {
    c.insert(text)?;
    c.submit()?;
    Ok(())
}

#[test]
fn history_restores_unsent_draft() -> TestResult {
    let mut c = Composer::new();
    send(&mut c, "one")?;
    send(&mut c, "two")?;
    c.insert("draft")?;
    c.move_left();
    let draft_cursor = c.cursor();
    assert_eq!(draft_cursor, 4);

    assert!(c.history_prev(true));
    assert_eq!(c.text(), "two");
    assert!(c.history_prev(true));
    assert_eq!(c.text(), "one");
    assert!(!c.history_prev(true), "already at the oldest entry");
    assert_eq!(c.text(), "one");
    assert!(c.history_next(true));
    assert_eq!(c.text(), "two");
    assert!(c.history_next(true));
    assert_eq!(c.text(), "draft");
    assert_eq!(c.cursor(), draft_cursor);
    assert!(!c.history_next(true), "not browsing any more");
    assert_eq!(c.text(), "draft");
    Ok(())
}

#[test]
fn history_round_trip_keeps_draft_chips() -> TestResult {
    let mut c = Composer::new();
    send(&mut c, "old")?;
    c.insert("d ")?;
    c.insert("1\n2\n3\n4\n5\n6")?;
    let text = c.text().to_owned();
    assert!(c.history_prev(true));
    assert_eq!(c.text(), "old");
    assert!(c.chips().is_empty());
    assert!(c.history_next(true));
    assert_eq!(c.text(), text);
    assert_eq!(c.chips().len(), 1);
    assert_eq!(c.submit()?.text, "d 1\n2\n3\n4\n5\n6");
    Ok(())
}

#[test]
fn history_only_moves_at_first_and_last_row() -> TestResult {
    let mut c = Composer::new();
    assert!(!c.history_prev(true), "empty history");
    send(&mut c, "one")?;
    c.insert("x")?;
    assert!(!c.history_prev(false));
    assert_eq!(c.text(), "x");
    assert!(c.history_prev(true));
    assert_eq!(c.text(), "one");
    assert!(!c.history_next(false));
    assert_eq!(
        c.text(),
        "one",
        "not on the last row: caller moves the cursor instead"
    );
    assert!(c.history_next(true));
    assert_eq!(c.text(), "x");
    Ok(())
}

#[test]
fn history_walks_oldest_to_newest() -> TestResult {
    let mut c = Composer::new();
    for text in ["a", "b", "c"] {
        send(&mut c, text)?;
    }
    assert_eq!(c.history_len(), 3);
    let mut seen = Vec::new();
    while c.history_prev(true) {
        seen.push(c.text().to_owned());
    }
    assert_eq!(seen, ["c", "b", "a"]);
    let mut back = Vec::new();
    while c.history_next(true) {
        back.push(c.text().to_owned());
    }
    assert_eq!(back, ["b", "c", ""]);
    Ok(())
}

#[test]
fn history_dedupes_consecutive_and_strips_image_markers() -> TestResult {
    let mut c = Composer::new();
    send(&mut c, "same")?;
    send(&mut c, "same")?;
    assert_eq!(c.history_len(), 1);

    c.insert("look ")?;
    c.add_image(5)?;
    c.submit()?;
    assert_eq!(c.history_len(), 2);
    assert!(c.history_prev(true));
    assert_eq!(c.text(), "look");

    // An image-only prompt leaves no history entry behind.
    let mut only = Composer::new();
    only.add_image(5)?;
    only.submit()?;
    assert_eq!(only.history_len(), 0);

    // A failed submit records nothing and keeps the draft.
    let mut blank = Composer::new();
    blank.insert("   ")?;
    assert!(blank.submit().is_err());
    assert_eq!(blank.history_len(), 0);
    Ok(())
}

#[test]
fn history_is_session_local() -> TestResult {
    let mut one = Composer::new();
    send(&mut one, "secret")?;
    let mut two = Composer::new();
    assert_eq!(two.history_len(), 0);
    assert!(!two.history_prev(true));
    Ok(())
}

proptest! {
    #[test]
    fn prop_history_never_loses_the_draft(
        entries in proptest::collection::vec("[a-z]{1,10}", 0..8),
        draft in "[a-z ]{0,10}",
        back in 0usize..12,
        extra in 0usize..4,
        cursor_back in 0usize..4,
        with_chip in any::<bool>(),
    ) {
        let mut c = Composer::new();
        for entry in &entries {
            c.insert(entry).map_err(|e| TestCaseError::fail(e.to_string()))?;
            c.submit().map_err(|e| TestCaseError::fail(e.to_string()))?;
        }
        c.insert(&draft).map_err(|e| TestCaseError::fail(e.to_string()))?;
        if with_chip {
            c.insert("1\n2\n3\n4\n5\n6").map_err(|e| TestCaseError::fail(e.to_string()))?;
        }
        for _ in 0..cursor_back {
            c.move_left();
        }
        let text = c.text().to_owned();
        let cursor = c.cursor();
        let chips = c.chips().len();

        let mut moved = 0usize;
        for _ in 0..back {
            if c.history_prev(true) {
                moved += 1;
            }
        }
        prop_assert!(moved <= entries.len());
        for _ in 0..moved + extra {
            let _ = c.history_next(true);
        }
        prop_assert_eq!(c.text(), text.as_str());
        prop_assert_eq!(c.cursor(), cursor);
        prop_assert_eq!(c.chips().len(), chips);
    }
}
