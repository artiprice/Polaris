//! Source selection helpers and notes selection preserve Unicode and chip boundaries.
use std::error::Error;

use polaris_ui_model::composer::{
    Composer, source_grapheme_range, source_line_range, source_word_range,
};
use polaris_ui_model::questionnaire::{
    QuestionKind, QuestionOption, QuestionSpec, QuestionnaireAction as A, QuestionnaireSpec,
    QuestionnaireState,
};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn source_grapheme_range_covers_combining_zwj_and_clamped_offsets() {
    let text = "x e\u{301} 👩‍💻 界";
    let combining_start = 2;
    let combining_end = combining_start + "e\u{301}".len();
    assert_eq!(
        source_grapheme_range(text, combining_start + 1),
        Some(combining_start..combining_end)
    );
    let emoji_start = combining_end + 1;
    assert_eq!(
        source_grapheme_range(text, emoji_start + 2),
        Some(emoji_start..emoji_start + "👩‍💻".len())
    );
    let wide_start = text.len() - "界".len();
    assert_eq!(
        source_grapheme_range(text, usize::MAX),
        Some(wide_start..text.len())
    );
    assert_eq!(source_grapheme_range("", 0), None);
}

#[test]
fn source_word_range_groups_unicode_words_and_actual_whitespace_runs() {
    let text = "café_界  \t!?";
    assert_eq!(source_word_range(text, 2), Some(0.."café_界".len()));
    assert_eq!(
        source_word_range(text, "café_界 ".len()),
        Some("café_界".len().."café_界  \t".len())
    );
    assert_eq!(
        source_word_range(text, "café_界  \t".len()),
        Some("café_界  \t".len().."café_界  \t!".len())
    );
    assert_eq!(source_word_range("", 0), None);
}

#[test]
fn source_line_range_includes_present_lf_and_crlf_and_handles_eof() {
    let lf = "first\n\nlast";
    assert_eq!(source_line_range(lf, 2), Some(0..6));
    assert_eq!(source_line_range(lf, 6), Some(6..7));
    assert_eq!(source_line_range(lf, usize::MAX), Some(7..11));
    let crlf = "first\r\nlast\r\n";
    assert_eq!(source_line_range(crlf, 2), Some(0..7));
    assert_eq!(source_line_range(crlf, 9), Some(7..13));
    assert_eq!(source_line_range("", 0), None);
}

#[test]
fn full_selection_deletes_unicode_and_chip_atomically_and_undo_restores_it() -> TestResult {
    let mut composer = Composer::new();
    composer.insert("e\u{301} 👩‍💻 ")?;
    composer.insert("one\ntwo\nthree\nfour\nfive\nsix")?;
    let chip = composer
        .chips()
        .first()
        .ok_or("expected pasted-content chip")?
        .range
        .clone();
    let text = composer.text().to_owned();
    composer.select_all();
    assert!(composer.delete());
    assert!(composer.text().is_empty());
    assert!(composer.chips().is_empty());
    assert!(composer.undo());
    assert_eq!(composer.text(), text);
    assert_eq!(composer.selection(), Some(0..text.len()));
    assert_eq!(
        composer.chips().first().map(|item| item.range.clone()),
        Some(chip)
    );
    Ok(())
}

#[test]
fn questionnaire_set_selection_edits_notes_and_undo_restores_selected_range() -> TestResult {
    let mut state = QuestionnaireState::new(QuestionnaireSpec {
        request_id: "selection".to_owned(),
        revision: 1,
        requester_label: "demo".to_owned(),
        questions: vec![QuestionSpec {
            id: "q".to_owned(),
            prompt: "Choose".to_owned(),
            required: true,
            kind: QuestionKind::SingleChoice {
                options: vec![QuestionOption {
                    id: "a".to_owned(),
                    label: "A".to_owned(),
                }],
            },
        }],
    })?;
    state.apply(A::FocusNotes)?;
    state.apply(A::InsertText("e\u{301} 👩‍💻 note".to_owned()))?;
    state.apply(A::SetSelection {
        anchor: 0,
        head: "e\u{301} 👩‍💻".len(),
    })?;
    state.apply(A::InsertText("selected".to_owned()))?;
    assert_eq!(state.notes(), "selected note");
    assert_eq!(state.answer_text(), "");
    state.apply(A::Undo)?;
    assert_eq!(state.notes(), "e\u{301} 👩‍💻 note");
    assert_eq!(
        state.active_editor().and_then(Composer::selection),
        Some(0.."e\u{301} 👩‍💻".len())
    );
    Ok(())
}

proptest::proptest! {
    #[test]
    fn source_unit_ranges_remain_legal_for_arbitrary_unicode(text in proptest::arbitrary::any::<String>(), at in 0usize..4096) {
        for range in [source_grapheme_range(&text,at),source_word_range(&text,at),source_line_range(&text,at)].into_iter().flatten(){
            proptest::prop_assert!(range.start<=range.end);
            proptest::prop_assert!(range.end<=text.len());
            proptest::prop_assert!(text.get(range.clone()).is_some());
            let start_is_boundary=range.start==text.len()||unicode_segmentation::UnicodeSegmentation::grapheme_indices(text.as_str(),true).any(|(index,_)|index==range.start);
            let end_is_boundary=range.end==text.len()||unicode_segmentation::UnicodeSegmentation::grapheme_indices(text.as_str(),true).any(|(index,_)|index==range.end);
            proptest::prop_assert!(start_is_boundary&&end_is_boundary);
        }
    }
}
